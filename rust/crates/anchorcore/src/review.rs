//! Review queues — port of `backend/app/routers/entities.py:169` `review_router`.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::health::AppState;

#[derive(Deserialize)]
pub struct DupQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub kind: Option<String>,
}

#[derive(Deserialize)]
pub struct MergePayload {
    pub proposal_id: i64,
    pub decision: String,
}

fn similar(a: &str, b: &str) -> f64 {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let norm = |s: &str| {
        let re = RE.get_or_init(|| regex::Regex::new(r"[^a-z0-9 ]").unwrap());
        re.replace_all(&s.to_lowercase(), " ").to_string()
    };
    let wa: std::collections::HashSet<String> = norm(a).split_whitespace().map(|s| s.to_string()).collect();
    let wb: std::collections::HashSet<String> = norm(b).split_whitespace().map(|s| s.to_string()).collect();
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let inter = wa.intersection(&wb).count() as f64;
    inter / ((wa.len() as f64 * wb.len() as f64).sqrt())
}

fn low_conf_row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "id": r.get::<_, i64>(0)?,
        "item_id": r.get::<_, i64>(1)?,
        "kind": r.get::<_, String>(2)?,
        "summary": r.get::<_, String>(3)?,
        "reasoning": r.get::<_, String>(4)?,
        "confidence": r.get::<_, f64>(5)?,
        "author": r.get::<_, String>(6)?,
        "source_ref": r.get::<_, String>(7)?,
        "status": r.get::<_, String>(8)?,
        "owner": r.get::<_, String>(9)?,
        "window_text": r.get::<_, String>(10)?,
        "dispute_count": r.get::<_, i64>(11)?,
        "created_at": r.get::<_, Option<String>>(12)?.unwrap_or_default(),
        "updated_at": r.get::<_, Option<String>>(13)?.unwrap_or_default(),
        "doc_type": r.get::<_, String>(14).unwrap_or_default()
    }))
}

pub async fn low_confidence_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let threshold: f64 = std::env::var("ANCHOR_LOW_CONFIDENCE_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.6);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at, (SELECT doc_type FROM ingested_items WHERE ingested_items.id = entities.item_id) FROM entities WHERE confidence < ?1 AND status = 'unverified' ORDER BY confidence ASC LIMIT 100").unwrap();
        let rows = stmt.query_map([threshold], low_conf_row).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Value::Array(out)
    }).await.unwrap();
    Json(result)
}

fn propose_duplicates(
    conn: &rusqlite::Connection,
    entities: &[(i64, String, String, String)],
    existing: &std::collections::HashSet<(i64, i64)>,
    dup_threshold: f64,
    offset: usize,
    limit: usize,
) -> Vec<Value> {
    let mut proposals: Vec<Value> = vec![];
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for i in 0..entities.len() {
        if proposals.len() >= offset + limit { break; }
        let (aid, ref asum, _, _) = entities[i];
        if seen.contains(&aid) { continue; }
        for j in (i + 1)..entities.len() {
            if proposals.len() >= offset + limit { break; }
            let (bid, ref bsum, _, _) = entities[j];
            if seen.contains(&bid) { continue; }
            if existing.contains(&(aid, bid)) || existing.contains(&(bid, aid)) { continue; }
            if similar(asum, bsum) > dup_threshold {
                // insert proposal
                let _ = conn.execute("INSERT INTO merge_actions (entity_a_id, entity_b_id, status) VALUES (?1, ?2, 'proposed')", rusqlite::params![aid, bid]);
                let pid = conn.last_insert_rowid();
                proposals.push(serde_json::json!({"id": pid, "entity_a_id": aid, "entity_b_id": bid, "reason": "similar summaries"}));
                seen.insert(aid);
                seen.insert(bid);
            }
        }
    }
    proposals
}

pub async fn duplicates_handler(State(state): State<AppState>, Query(q): Query<DupQuery>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let kind = q.kind.clone();
    let dup_threshold: f64 = std::env::var("ANCHOR_DUPLICATE_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.92);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut sql = "SELECT id, summary, kind, created_at FROM entities WHERE status != 'stale'".to_string();
        if let Some(k) = kind {
            sql.push_str(&format!(" AND kind = '{}'", k.replace('\'', "''")));
        }
        sql.push_str(" ORDER BY created_at DESC");
        let mut stmt = conn.prepare(&sql).unwrap();
        let mut entities: Vec<(i64, String, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?.unwrap_or_default()))).unwrap().filter_map(|r| r.ok()).collect();
        if entities.len() > 500 {
            entities.sort_by(|a, b| b.3.cmp(&a.3));
            entities.truncate(200);
        }
        // existing merge actions
        let mut stmt2 = conn.prepare("SELECT entity_a_id, entity_b_id FROM merge_actions").unwrap();
        let existing: std::collections::HashSet<(i64, i64)> = stmt2.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        let proposals = propose_duplicates(&conn, &entities, &existing, dup_threshold, offset as usize, limit as usize);
        let start = offset as usize;
        let end = (start + limit as usize).min(proposals.len());
        if start >= proposals.len() {
            Value::Array(vec![])
        } else {
            Value::Array(proposals[start..end].to_vec())
        }
    }).await.unwrap();
    Json(result)
}

fn apply_merge(
    conn: &rusqlite::Connection,
    a_id: i64,
    b_id: i64,
    a_row: (String, String, f64, String),
    b_row: (String, String, f64, String),
) {
    let (_a_sum, _a_reas, a_conf, a_auth) = a_row;
    let (b_sum, b_reas, b_conf, b_auth) = b_row;
    // repoint chunks
    let _ = conn.execute("UPDATE chunks SET entity_id = ?1 WHERE entity_id = ?2", rusqlite::params![a_id, b_id]);
    if b_conf > a_conf {
        let _ = conn.execute("UPDATE entities SET summary=?1, reasoning=?2, confidence=?3 WHERE id=?4", rusqlite::params![b_sum, b_reas, b_conf, a_id]);
    }
    if a_auth.is_empty() && !b_auth.is_empty() {
        let _ = conn.execute("UPDATE entities SET author=?1 WHERE id=?2", rusqlite::params![b_auth, a_id]);
    }
    let _ = conn.execute("UPDATE entities SET status='unverified' WHERE id=?1", [a_id]);
    let _ = conn.execute("DELETE FROM relationships WHERE from_entity_id=?1 OR to_entity_id=?1", [b_id]);
    let _ = conn.execute("DELETE FROM merge_actions WHERE entity_a_id=?1 OR entity_b_id=?1", [b_id]);
    let _ = conn.execute("DELETE FROM entities WHERE id=?1", [b_id]);
}

fn fetch_proposal(conn: &rusqlite::Connection, proposal_id: i64) -> Option<(i64, i64, String)> {
    conn.query_row("SELECT entity_a_id, entity_b_id, status FROM merge_actions WHERE id = ?1", [proposal_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()
}

fn fetch_entity(conn: &rusqlite::Connection, id: i64) -> Option<(String, String, f64, String)> {
    conn.query_row("SELECT summary, reasoning, confidence, author FROM entities WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).ok()
}

fn run_merge(data_dir: &str, proposal_id: i64, decision: &str) -> Value {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
    let Some((a_id, b_id, _status)) = fetch_proposal(&conn, proposal_id) else {
        return serde_json::json!({"detail":"Proposal not found"});
    };
    if decision == "dismiss" {
        let _ = conn.execute("UPDATE merge_actions SET status='dismissed' WHERE id=?1", [proposal_id]);
        return serde_json::json!({"merged": false});
    }
    // fetch entities
    let a_row = fetch_entity(&conn, a_id);
    let b_row = fetch_entity(&conn, b_id);
    if a_row.is_none() || b_row.is_none() {
        return serde_json::json!({"detail":"Entity not found"});
    }
    apply_merge(&conn, a_id, b_id, a_row.unwrap(), b_row.unwrap());
    let _ = conn.execute("UPDATE merge_actions SET status='merged' WHERE id=?1", [proposal_id]);
    serde_json::json!({"merged": true, "entity_id": a_id})
}

pub async fn merge_handler(State(state): State<AppState>, Json(payload): Json<MergePayload>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let proposal_id = payload.proposal_id;
    let decision = payload.decision;
    let result = tokio::task::spawn_blocking(move || run_merge(&data_dir, proposal_id, &decision)).await.unwrap();
    Json(result)
}

// --- R40.8: vocabulary approval (human-confirmation half of R40.7) ---

#[derive(Deserialize)]
pub struct VocabDecisionPayload {
    pub decision: String,
}

/// One witness doc for a proposed pair (LEFT JOIN — a NULL-item row still
/// lists with an empty doc rather than vanishing from review).
fn vocab_witness(
    conn: &rusqlite::Connection,
    term: &str,
    expansion: &str,
) -> Option<(i64, String, String, String)> {
    conn.query_row(
        "SELECT v.item_id, i.title, i.external_id, v.origin FROM vocabulary v
         LEFT JOIN ingested_items i ON i.id = v.item_id
         WHERE v.term = ?1 AND v.expansion = ?2 AND v.status = 'proposed'
         ORDER BY v.id LIMIT 1",
        rusqlite::params![term, expansion],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, String>(3)?,
            ))
        },
    )
    .ok()
}

/// Proposed pairs for review: id is the MIN row id (pair handle for POST),
/// with a witness doc, origin, and witness count. Rule-origin pairs are
/// excluded structurally (rules are born active) AND explicitly (a rule row
/// anywhere keeps the pair out of review — rules skip review entirely).
fn list_proposed_vocab(conn: &rusqlite::Connection) -> Vec<Value> {
    let mut stmt = match conn.prepare(
        "SELECT MIN(id), term, expansion, COUNT(DISTINCT item_id) FROM vocabulary
         WHERE status = 'proposed'
         AND NOT EXISTS (SELECT 1 FROM vocabulary r WHERE r.term = vocabulary.term AND r.expansion = vocabulary.expansion AND r.origin = 'rule')
         GROUP BY term, expansion ORDER BY MIN(id) LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<(i64, String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|(id, term, expansion, confirmations)| {
            let (item_id, title, external_id, origin) = vocab_witness(conn, &term, &expansion)?;
            Some(serde_json::json!({
                "id": id,
                "term": term,
                "expansion": expansion,
                "origin": origin,
                "confirmations": confirmations,
                "item_id": item_id,
                "item_title": title,
                "external_id": external_id,
            }))
        })
        .collect()
}

/// Approve → the pair's proposed rows go active (never resurrects
/// rejected); reject → every non-rejected row of the pair goes rejected
/// (R40.7 store then skips it on every re-sync). One decision covers the
/// whole pair family, however many items witnessed it.
fn apply_vocab_decision(conn: &rusqlite::Connection, id: i64, decision: &str) -> Value {
    let pair: Option<(String, String)> = conn
        .query_row("SELECT term, expansion FROM vocabulary WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    let Some((term, expansion)) = pair else {
        return serde_json::json!({"detail": "Vocabulary entry not found"});
    };
    match decision {
        "approve" => {
            let n = conn
                .execute(
                    "UPDATE vocabulary SET status = 'active' WHERE term = ?1 AND expansion = ?2 AND status = 'proposed'",
                    rusqlite::params![term, expansion],
                )
                .unwrap_or(0);
            serde_json::json!({"approved": true, "term": term, "expansion": expansion, "activated": n})
        }
        "reject" => {
            let n = conn
                .execute(
                    "UPDATE vocabulary SET status = 'rejected' WHERE term = ?1 AND expansion = ?2 AND status != 'rejected'",
                    rusqlite::params![term, expansion],
                )
                .unwrap_or(0);
            serde_json::json!({"approved": false, "term": term, "expansion": expansion, "rejected": n})
        }
        _ => serde_json::json!({"detail": "decision must be 'approve' or 'reject'"}),
    }
}

pub async fn vocabulary_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        Value::Array(list_proposed_vocab(&conn))
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn vocabulary_decide_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<VocabDecisionPayload>,
) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let decision = payload.decision;
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        apply_vocab_decision(&conn, id, &decision)
    })
    .await
    .unwrap();
    Json(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_db() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("review.db")).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        (dir, conn)
    }

    fn add_item(conn: &rusqlite::Connection, external: &str, title: &str) -> i64 {
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, content_hash) VALUES (1,?1,?2,'h')",
            rusqlite::params![external, title],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn pair(term: &str, expansion: &str) -> crate::vocab::DefinedTerm {
        crate::vocab::normalize_pair(term, expansion).expect("test pair must normalize")
    }

    #[test]
    fn vocabulary_list_shows_llm_proposals_with_witnesses() {
        let (_dir, conn) = review_db();
        let a = add_item(&conn, "a.md", "Alpha doc");
        crate::vocab::store_vocab(&conn, a, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        // Rules-extracted pairs skip review (born active).
        let b = add_item(&conn, "b.md", "Beta doc");
        crate::vocab::store_vocab(&conn, b, 1, &[pair("code", "long form")], &[]).unwrap();
        let list = list_proposed_vocab(&conn);
        assert_eq!(list.len(), 1, "only the llm proposal lists: {list:?}");
        let row = &list[0];
        assert_eq!(row["term"], "dunning");
        assert_eq!(row["expansion"], "payment reminder");
        assert_eq!(row["origin"], "llm");
        assert_eq!(row["confirmations"], 1);
        assert_eq!(row["item_title"], "Alpha doc");
        assert_eq!(row["external_id"], "a.md");
    }

    #[test]
    fn vocabulary_list_excludes_rule_touched_pairs() {
        // A rule row anywhere keeps the pair out of review, even beside a
        // proposed LLM row (hand-seeded — store_vocab would have activated
        // the family, so this shape only arises out of band).
        let (_dir, conn) = review_db();
        let a = add_item(&conn, "a.md", "Alpha doc");
        conn.execute(
            "INSERT INTO vocabulary (term, expansion, item_id, source_id, origin, status) VALUES ('ca','direct debit',?1,1,'llm','proposed')",
            [a],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vocabulary (term, expansion, item_id, source_id, origin, status) VALUES ('ca','direct debit',?1,1,'rule','active')",
            [a],
        )
        .unwrap();
        assert!(list_proposed_vocab(&conn).is_empty(), "rule-touched pair skips review");
    }

    #[test]
    fn vocabulary_approve_reject_round_trip() {
        let (_dir, conn) = review_db();
        let a = add_item(&conn, "a.md", "Alpha doc");
        crate::vocab::store_vocab(&conn, a, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        let id: i64 = conn.query_row("SELECT id FROM vocabulary WHERE term = 'dunning'", [], |r| r.get(0)).unwrap();
        let approved = apply_vocab_decision(&conn, id, "approve");
        assert_eq!(approved["approved"], true);
        assert_eq!(approved["activated"], 1);
        assert!(list_proposed_vocab(&conn).is_empty(), "approved pair leaves the queue");
        let status: String =
            conn.query_row("SELECT status FROM vocabulary WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(status, "active");
        // Reject path on a fresh pair.
        let b = add_item(&conn, "b.md", "Beta doc");
        crate::vocab::store_vocab(&conn, b, 1, &[], &[pair("bogus", "bad guess")]).unwrap();
        let bid: i64 = conn.query_row("SELECT id FROM vocabulary WHERE term = 'bogus'", [], |r| r.get(0)).unwrap();
        let rejected = apply_vocab_decision(&conn, bid, "reject");
        assert_eq!(rejected["approved"], false);
        assert_eq!(rejected["rejected"], 1);
        assert!(list_proposed_vocab(&conn).is_empty(), "rejected pair leaves the queue");
    }

    #[test]
    fn vocabulary_rejected_pair_survives_resync() {
        // R40.8's durability clause, end to end at the store layer: reject
        // via the handler path, then re-run ingest storage — the pair stays
        // rejected and gains no rows.
        let (_dir, conn) = review_db();
        let a = add_item(&conn, "a.md", "Alpha doc");
        crate::vocab::store_vocab(&conn, a, 1, &[], &[pair("bogus", "bad guess")]).unwrap();
        let id: i64 = conn.query_row("SELECT id FROM vocabulary WHERE term = 'bogus'", [], |r| r.get(0)).unwrap();
        apply_vocab_decision(&conn, id, "reject");
        crate::vocab::store_vocab(&conn, a, 1, &[], &[pair("bogus", "bad guess")]).unwrap();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT id, status FROM vocabulary WHERE term = 'bogus'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(rows, vec![(id, "rejected".to_string())], "no resurrection, no dupes: {rows:?}");
    }

    #[test]
    fn vocabulary_decide_rejects_bad_input() {
        let (_dir, conn) = review_db();
        assert_eq!(apply_vocab_decision(&conn, 4242, "approve")["detail"], "Vocabulary entry not found");
        let a = add_item(&conn, "a.md", "Alpha doc");
        crate::vocab::store_vocab(&conn, a, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        let id: i64 = conn.query_row("SELECT id FROM vocabulary WHERE term = 'dunning'", [], |r| r.get(0)).unwrap();
        assert_eq!(apply_vocab_decision(&conn, id, "maybe")["detail"], "decision must be 'approve' or 'reject'");
    }
}
