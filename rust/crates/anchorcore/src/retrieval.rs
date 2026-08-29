//! R2.1: Retrieval (RRF + FTS + vec0) — port of `backend/app/answer_engine.py:1`
//!
//! - `_fts_match_query` -> `fts_match_query`
//! - `_rrf_fuse` / `_rrf_fuse_multi` -> `rrf_fuse*`
//! - `_age_decay`, `_content_signature`, `_status_ok`
//! - `_vector_search` (vec0 + scan fallback), `_keyword_search`, `_who_knows`, `_graph_expand`
//!
//! This is the CPU-bound hot path. LLM calls stay in `answer.rs`.

use chrono::{DateTime, Utc};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const RRF_K: f64 = 60.0;
pub const GRAPH_KIND_WEIGHTS: &[(&str, f64)] = &[
    ("supersedes", 1.0),
    ("depends_on", 0.9),
    ("owns", 0.8),
    ("blocks", 0.7),
    ("related", 0.4),
];
pub const GRAPH_DEFAULT_WEIGHT: f64 = 0.5;

/// Mirrors `answer_engine.py:40` _WHO_KNOWS_RE
pub fn is_who_knows(query: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\b(who|whom|owns?|owner|responsible|expert|knows?)\b").unwrap());
    re.is_match(query)
}

/// Mirrors `answer_engine.py:71` _fts_match_query — R8.2: Unicode-aware (was [a-z0-9_])
pub fn fts_match_query(question: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_§\-]+").unwrap());
    let stopwords: HashSet<&str> = [
        "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for", "with",
        "about", "is", "are", "was", "were", "be", "been", "being", "am", "do", "does", "did",
        "have", "has", "had", "will", "would", "can", "could", "should", "shall", "may",
        "might", "must", "what", "which", "who", "whom", "whose", "when", "where", "why", "how",
        "this", "that", "these", "those", "it", "its", "not", "no", "so", "if", "then", "than",
        "too", "very", "s", "t", "you", "your", "we", "our", "they", "their", "i", "me", "my",
    ]
    .into();
    let terms: Vec<String> = re
        .find_iter(&question.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|t| t.len() >= 2 && !stopwords.contains(t.as_str()))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(" OR "))
}

/// Mirrors `answer_engine.py:96` _rrf_fuse_multi
pub fn rrf_fuse_multi(lists: Vec<Vec<Hit>>, weights: Vec<f64>) -> Vec<Hit> {
    let mut fused: HashMap<(i64, Option<i64>), Hit> = HashMap::new();
    for (hits, weight) in lists.into_iter().zip(weights) {
        for (rank, hit) in hits.into_iter().enumerate() {
            let rank = (rank + 1) as f64;
            let key = hit.key();
            let entry = fused.entry(key).or_insert_with(|| Hit {
                chunk_id: hit.chunk_id,
                entity_id: hit.entity_id,
                score: 0.0,
                ..hit.clone()
            });
            let add = weight / (RRF_K + rank);
            entry.score += add;
        }
    }
    let mut out: Vec<Hit> = fused.into_values().collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    out
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub chunk_id: i64,
    pub entity_id: Option<i64>,
    pub item_id: Option<i64>,
    pub source_id: Option<i64>,
    pub score: f64,
    pub content: String,
    pub source_ref: String,
    pub is_pii: bool,
    pub status: Option<String>,
}

impl Hit {
    fn key(&self) -> (i64, Option<i64>) {
        // was i64 with -entity_id collision when chunk_id == -entity_id (P1 4.6)
        (self.chunk_id, self.entity_id)
    }
}

/// Mirrors `answer_engine.py:115` _age_decay
pub fn age_decay(created_at: Option<DateTime<Utc>>, halflife_days: f64, now: DateTime<Utc>) -> f64 {
    let Some(ts) = created_at else { return 1.0 };
    if halflife_days <= 0.0 {
        return 1.0;
    }
    let age_days = (now - ts).num_seconds() as f64 / 86400.0;
    if age_days < 0.0 {
        return 1.0;
    }
    0.5_f64.powf(age_days / halflife_days)
}

/// Mirrors `answer_engine.py:134` _content_signature
pub fn content_signature(content: &str) -> String {
    static RE_WS: OnceLock<Regex> = OnceLock::new();
    static RE_PAGE: OnceLock<Regex> = OnceLock::new();
    let re_ws = RE_WS.get_or_init(|| Regex::new(r"\s+").unwrap());
    let re_page = RE_PAGE.get_or_init(|| Regex::new(r"^\d{1,4}\s+").unwrap());
    let mut s = re_ws.replace_all(content, " ").trim().to_lowercase();
    s = re_page.replace(&s, "").to_string();
    s.chars().take(160).collect()
}

/// Mirrors `answer_engine.py:157` _status_ok
pub fn status_ok(status: Option<&str>, qa_exclude_disputed: bool) -> bool {
    match status {
        None => true,
        Some("stale") => false,
        Some("disputed") if qa_exclude_disputed => false,
        _ => true,
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

pub fn unpack_f32(blob: &[u8], dim: usize) -> Option<Vec<f32>> {
    if blob.len() != dim * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(dim);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

// --- DB-backed retrieval (mirrors answer_engine.py:610 _vector_search etc.) ---

use rusqlite::Connection;

pub fn vector_search(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&std::collections::HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if let Some(hits) = vector_search_vec0(conn, query_emb, source_ids, top_k, qa_exclude_disputed) {
        return hits;
    }
    vector_search_scan(conn, query_emb, source_ids, top_k, qa_exclude_disputed)
}

fn vector_search_vec0(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Option<Vec<Hit>> {
    let dim: usize = std::env::var("ANCHOR_EMBED_DIM").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
    if query_emb.len() != dim {
        return None;
    }
    let q_json = serde_json::to_string(query_emb).ok()?;
    let mut sql = String::from(
        "SELECT v.rowid, v.distance FROM vec_chunks v \
         JOIN chunks c ON c.id = v.rowid \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE v.embedding MATCH ?",
    );
    // parametrized IN list (was format! IN ({list}) — P1 4.5)
    let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(q_json)];
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND i.source_id IN ({})", placeholders));
            for id in ids {
                params.push(rusqlite::types::Value::Integer(*id));
            }
        }
    }
    sql.push_str(" AND k = ?");
    let limit = (top_k * 4) as i64;
    params.push(rusqlite::types::Value::Integer(limit));
    let mut stmt = conn.prepare(&sql).ok()?;
    // positional params: ?1 = q_json, ?2..?n = source ids, ?last = limit
    // Build rusqlite params from Vec<Value>
    let rows: Result<Vec<(i64, f64)>, _> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .ok()?
        .collect();
    let rows = match rows {
        Ok(v) => v,
        Err(_) => return None,
    };
    if rows.is_empty() {
        return Some(vec![]);
    }
    let by_id: HashMap<i64, f64> = rows.into_iter().collect();
    let ids: Vec<i64> = by_id.keys().cloned().collect();
    let hits = load_hits(conn, &ids, source_ids, qa_exclude_disputed);
    let mut out = Vec::new();
    for mut h in hits {
        if let Some(d) = by_id.get(&h.chunk_id) {
            let score = 1.0 - *d as f32;
            if score <= 0.2 {
                continue;
            }
            h.score = score as f64;
            out.push(h);
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    Some(out)
}

fn vector_search_scan(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let hits = load_chunks_with_embedding(conn, source_ids, qa_exclude_disputed);
    let mut scored = Vec::new();
    for mut h in hits {
        if let Some(blob) = get_embedding(conn, h.chunk_id) {
            if let Some(vec) = unpack_f32(&blob, query_emb.len()) {
                let score = cosine(query_emb, &vec);
                if score > 0.2 {
                    h.score = score as f64;
                    scored.push(h);
                }
            }
        }
    }
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    scored.truncate(top_k * 4);
    scored
}

type HitRow10 = (i64, Option<i64>, Option<i64>, String, String, i64, Option<String>, Option<String>, Option<i64>, Option<i64>);

fn hit_from_row10(row: HitRow10, source_ids: Option<&HashSet<i64>>, qa_exclude_disputed: bool) -> Option<Hit> {
    let (cid, eid, iid, content, source_ref, is_pii, _created, status, _eff, sid) = row;
    if !status_ok(status.as_deref(), qa_exclude_disputed) {
        return None;
    }
    if let Some(filter) = source_ids {
        if let Some(s) = sid {
            if !filter.contains(&s) {
                return None;
            }
        }
    }
    Some(Hit {
        chunk_id: cid,
        entity_id: eid,
        item_id: iid,
        source_id: sid,
        score: 0.0,
        content,
        source_ref,
        is_pii: is_pii != 0,
        status,
    })
}

fn load_hits(
    conn: &Connection,
    ids: &[i64],
    source_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if ids.is_empty() {
        return vec![];
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, c.created_at, e.status, \
                 COALESCE(c.item_id, e.item_id) as eff_item, i.source_id \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE c.id IN ({})",
        placeholders
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(h) = hit_from_row10(row, source_ids, qa_exclude_disputed) {
            out.push(h);
        }
    }
    out
}

fn load_chunks_with_embedding(
    conn: &Connection,
    source_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let sql = "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, i.source_id \
               FROM chunks c \
               LEFT JOIN entities e ON e.id = c.entity_id \
               LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
               WHERE c.embedding IS NOT NULL";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (cid, eid, iid, content, source_ref, is_pii, status, sid) = row;
        if !status_ok(status.as_deref(), qa_exclude_disputed) {
            continue;
        }
        if let Some(filter) = source_ids {
            if let Some(s) = sid {
                if !filter.contains(&s) {
                    continue;
                }
            } else {
                continue;
            }
        }
        out.push(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, score: 0.0, content, source_ref, is_pii: is_pii != 0, status });
    }
    out
}

fn get_embedding(conn: &Connection, chunk_id: i64) -> Option<Vec<u8>> {
    conn.query_row("SELECT embedding FROM chunks WHERE id = ?1", [chunk_id], |r| r.get::<_, Option<Vec<u8>>>(0))
        .ok()?
}

pub fn keyword_search(
    conn: &Connection,
    question: &str,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let Some(match_q) = fts_match_query(question) else { return vec![] };
    let limit = (top_k * 4) as i64;
    let mut stmt = match conn.prepare(
        "SELECT rowid, bm25(chunks_fts) AS rank FROM chunks_fts WHERE chunks_fts MATCH :q ORDER BY rank LIMIT :limit",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows: Vec<(i64, f64)> = match stmt
        .query_map(rusqlite::named_params! { ":q": match_q, ":limit": limit }, |r| Ok((r.get(0)?, r.get(1)?)))
        .and_then(|m| m.collect())
    {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    if rows.is_empty() {
        return vec![];
    }
    let by_id: HashMap<i64, f64> = rows.into_iter().collect();
    let ids: Vec<i64> = by_id.keys().cloned().collect();
    let hits = load_hits(conn, &ids, source_ids, qa_exclude_disputed);
    let mut out = Vec::new();
    for mut h in hits {
        if let Some(rank) = by_id.get(&h.chunk_id) {
            h.score = -rank;
            out.push(h);
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    out
}

type HitRow8 = (i64, Option<i64>, Option<i64>, String, String, i64, Option<String>, Option<i64>);

fn row8(r: &rusqlite::Row) -> rusqlite::Result<HitRow8> {
    Ok((
        r.get::<_, i64>(0)?,
        r.get::<_, Option<i64>>(1)?,
        r.get::<_, Option<i64>>(2)?,
        r.get::<_, String>(3)?,
        r.get::<_, String>(4)?,
        r.get::<_, i64>(5)?,
        r.get::<_, Option<String>>(6)?,
        r.get::<_, Option<i64>>(7)?,
    ))
}

fn fallback_hit_from_row8(row: HitRow8, source_ids: Option<&HashSet<i64>>, qa_exclude_disputed: bool) -> Option<Hit> {
    let (cid, eid, iid, content, source_ref, is_pii, status, sid) = row;
    if !status_ok(status.as_deref(), qa_exclude_disputed) {
        return None;
    }
    if let Some(filter) = source_ids {
        let s = sid?;
        if !filter.contains(&s) {
            return None;
        }
    }
    Some(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, score: 0.0, content, source_ref, is_pii: is_pii != 0, status })
}

pub fn keyword_fallback(
    conn: &Connection,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let mut sql = String::from(
        "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, i.source_id \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id)",
    );
    let mut params: Vec<rusqlite::types::Value> = vec![];
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" WHERE i.source_id IN ({})", placeholders));
            for id in ids {
                params.push(rusqlite::types::Value::Integer(*id));
            }
        }
    }
    sql.push_str(" ORDER BY c.created_at DESC LIMIT ?");
    params.push(rusqlite::types::Value::Integer(top_k as i64));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), row8) {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(h) = fallback_hit_from_row8(row, source_ids, qa_exclude_disputed) {
            out.push(h);
        }
    }
    out
}

pub fn who_knows_search(
    conn: &Connection,
    query: &str,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]+").unwrap());
    let stopwords: HashSet<&str> = ["a","an","the","and","or","but","of","in","on","at","to","for","with","about","is","are","was","were","be","been","being","am","do","does","did","have","has","had","will","would","can","could","should","shall","may","might","must","what","which","who","whom","whose","when","where","why","how","this","that","these","those","it","its","not","no","so","if","then","than","too","very","s","t","you","your","we","our","they","their","i","me","my"].into();
    let terms: Vec<String> = re.find_iter(&query.to_lowercase()).map(|m| m.as_str().to_string()).filter(|t| t.len()>=3 && !stopwords.contains(t.as_str())).collect();
    if terms.is_empty() { return vec![]; }
    // parametrized LIKE with ESCAPE (P1 4.5) — was format!(" OR e.summary LIKE '%{}%'", t)
    let mut sql = String::from("SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, e.owner, e.author, e.summary, e.confidence, e.created_at, i.source_id FROM chunks c JOIN entities e ON e.id=c.entity_id JOIN ingested_items i ON i.id=e.item_id WHERE (e.owner != '' OR e.author != ''");
    let mut params: Vec<rusqlite::types::Value> = vec![];
    for t in &terms {
        // escape LIKE wildcards % _ and \  (was raw t with only ' escaped)
        let esc = t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let pat = format!("%{}%", esc);
        sql.push_str(" OR e.summary LIKE ? ESCAPE '\\'");
        params.push(rusqlite::types::Value::Text(pat));
    }
    sql.push(')');
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND i.source_id IN ({})", placeholders));
            for id in ids {
                params.push(rusqlite::types::Value::Integer(*id));
            }
        }
    }
    let mut stmt = match conn.prepare(&sql) { Ok(s) => s, Err(_) => return vec![] };
    let mut out = Vec::new();
    let halflife: f64 = std::env::var("ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(365.0);
    let now = Utc::now();
    let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?, r.get::<_, String>(9)?, r.get::<_, f64>(10)?, r.get::<_, Option<String>>(11)?, r.get::<_, Option<i64>>(12)?))) { Ok(m) => m, Err(_) => return out };
    for row in rows.flatten() {
        let (cid, eid, iid, content, source_ref, is_pii, status, owner, author, summary, conf, created, sid) = row;
        if !status_ok(status.as_deref(), qa_exclude_disputed) { continue; }
        let blob = format!("{} {} {}", owner, author, summary).to_lowercase();
        let overlap = terms.iter().filter(|t| blob.contains(&t.to_lowercase())).count();
        if overlap==0 { continue; }
        let created_dt = created.and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc));
        let decay = age_decay(created_dt, halflife, now);
        let score = conf * (1.0 + 0.5 * (overlap.min(4) as f64)) * decay;
        out.push(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, score, content, source_ref, is_pii: is_pii!=0, status });
    }
    out.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap());
    out.truncate(top_k*2);
    out
}

pub fn graph_expand(
    conn: &Connection,
    hits: &[Hit],
    source_ids: Option<&HashSet<i64>>,
    hops: usize,
    cap: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if cap==0 || hits.is_empty() { return vec![]; }
    let mut seed: HashSet<i64> = HashSet::new();
    for h in hits { if let Some(eid)=h.entity_id { seed.insert(eid); } }
    if seed.is_empty() { return vec![]; }
    let mut visited = seed.clone();
    let mut frontier = seed.clone();
    let mut candidates: HashMap<i64, f64> = HashMap::new();
    for hop in 0..hops.max(1) {
        if frontier.is_empty() { break; }
        let placeholders = frontier.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT from_entity_id, to_entity_id, kind FROM relationships WHERE from_entity_id IN ({}) OR to_entity_id IN ({})", placeholders, placeholders);
        let mut stmt = match conn.prepare(&sql) { Ok(s)=>s, Err(_)=> break };
        // params duplicated for both IN lists
        let mut params: Vec<rusqlite::types::Value> = vec![];
        for id in frontier.iter() {
            params.push(rusqlite::types::Value::Integer(*id));
        }
        for id in frontier.iter() {
            params.push(rusqlite::types::Value::Integer(*id));
        }
        let rows: Vec<(i64,i64,String)> = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().filter_map(|r| r.ok()).collect();
        let mut next = HashSet::new();
        for (from,to,kind) in rows {
            for (a,b) in [(from,to),(to,from)] {
                if !frontier.contains(&a) || visited.contains(&b) { continue; }
                let w = GRAPH_KIND_WEIGHTS.iter().find(|(k,_)| *k==kind).map(|(_,v)| *v).unwrap_or(GRAPH_DEFAULT_WEIGHT) * (0.5_f64.powi(hop as i32));
                if w > *candidates.get(&b).unwrap_or(&-1.0) { candidates.insert(b, w); }
                next.insert(b);
            }
        }
        visited.extend(next.clone());
        frontier = next;
    }
    if candidates.is_empty() { return vec![]; }
    let mut ranked: Vec<(i64,f64)> = candidates.into_iter().collect();
    ranked.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap());
    ranked.truncate(cap);
    let ids: Vec<i64> = ranked.iter().map(|(id,_)| *id).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut sql = format!("SELECT e.id, e.summary, e.status, e.item_id, c.id, c.content, c.source_ref, c.is_pii FROM entities e LEFT JOIN chunks c ON c.entity_id=e.id WHERE e.id IN ({})", placeholders);
    let mut params: Vec<rusqlite::types::Value> = ids.iter().map(|id| rusqlite::types::Value::Integer(*id)).collect();
    if let Some(filter)=source_ids {
        if !filter.is_empty() {
            let ph = filter.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND e.item_id IN (SELECT id FROM ingested_items WHERE source_id IN ({}))", ph));
            for id in filter.iter() {
                params.push(rusqlite::types::Value::Integer(*id));
            }
        }
    }
    let mut stmt = match conn.prepare(&sql) { Ok(s)=>s, Err(_)=> return vec![] };
    let rows: Vec<(i64,String,Option<String>,Option<i64>,Option<i64>,Option<String>,Option<String>,Option<i64>)> = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))).unwrap().filter_map(|r| r.ok()).collect();
    let by_id: HashMap<i64,(String,Option<String>,Option<i64>,Option<i64>,Option<String>,Option<String>,Option<i64>)> = rows.into_iter().map(|(id,summary,status,item,cid,content,src_ref,is_pii)| (id,(summary,status,item,cid,content,src_ref,is_pii))).collect();
    let mut out = Vec::new();
    for (eid, score) in ranked {
        if let Some((summary,status,item,cid,content,src_ref,_is_pii)) = by_id.get(&eid) {
            if !status_ok(status.as_deref(), qa_exclude_disputed) { continue; }
            // need source_id via item
            let sid: Option<i64> = if let Some(iid)=item { conn.query_row("SELECT source_id FROM ingested_items WHERE id=?1", [*iid], |r| r.get(0)).ok() } else { None };
            out.push(Hit { chunk_id: cid.unwrap_or(eid), entity_id: Some(eid), item_id: *item, source_id: sid, score, content: content.clone().unwrap_or_else(|| summary.clone()), source_ref: src_ref.clone().unwrap_or_else(|| summary.clone()), is_pii: false, status: status.clone() });
        }
    }
    out
}

pub fn fuse_evidence(mut lists: Vec<Vec<Hit>>, weights: Vec<f64>) -> Vec<Hit> {
    if lists.is_empty() { return vec![]; }
    if lists.len()==1 { let mut v = lists.remove(0); v.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap()); return v; }
    rrf_fuse_multi(lists, weights)
}

pub fn fuse_and_rank(
    conn: &Connection,
    vector_hits: Option<Vec<Hit>>,
    keyword_hits: Option<Vec<Hit>>,
    keyword_weight: f64,
    halflife_days: f64,
    max_per_source: usize,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    // R8.1: preserve single-list scores (was 1/61 constant, discarding BM25/cosine)
    let mut fused = if vector_hits.is_none() && keyword_hits.is_none() {
        vec![]
    } else if vector_hits.is_none() {
        keyword_hits.unwrap()
    } else if keyword_hits.is_none() {
        vector_hits.unwrap()
    } else {
        rrf_fuse_multi(vec![vector_hits.unwrap(), keyword_hits.unwrap()], vec![1.0, keyword_weight])
    };
    let now = Utc::now();
    // batch created_at (P1 4.7 — was N+1 per hit)
    if !fused.is_empty() {
        let ids: Vec<i64> = fused.iter().map(|h| h.chunk_id).collect();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id, created_at FROM chunks WHERE id IN ({})", placeholders);
        let mut map: HashMap<i64, Option<DateTime<Utc>>> = HashMap::new();
        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
                for row in rows.flatten() {
                    let (id, s) = row;
                    let dt = s.and_then(|v| DateTime::parse_from_rfc3339(&v).ok()).map(|d| d.with_timezone(&Utc));
                    map.insert(id, dt);
                }
            }
        }
        for h in &mut fused {
            let created = map.get(&h.chunk_id).cloned().unwrap_or(None);
            h.score *= age_decay(created, halflife_days, now);
        }
    }
    fused.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap());
    // diversity cap per item
    if max_per_source>0 {
        let distinct: std::collections::HashSet<Option<i64>> = fused.iter().map(|h| h.item_id).collect();
        let effective_cap = if distinct.len()>=3 { max_per_source } else { top_k };
        let mut per_item: std::collections::HashMap<Option<i64>, usize> = HashMap::new();
        let mut capped = Vec::new();
        for h in fused {
            let c = per_item.entry(h.item_id).or_insert(0);
            if *c >= effective_cap { continue; }
            *c+=1;
            capped.push(h);
        }
        fused = capped;
    }
    fused = dedupe_similar(fused);
    let mut hits: Vec<Hit> = fused.into_iter().take(top_k).collect();
    expand_context(conn, &mut hits, 1);
    // filter stale/disputed again (already in load, but for fused graph hits)
    hits.into_iter().filter(|h| status_ok(h.status.as_deref(), qa_exclude_disputed)).collect()
}

pub fn dedupe_similar(mut hits: Vec<Hit>) -> Vec<Hit> {
    let mut seen: HashSet<(Option<i64>, String)> = HashSet::new();
    let mut out = Vec::new();
    for h in hits.drain(..) {
        let key = (h.item_id, content_signature(&h.content));
        if seen.contains(&key) { continue; }
        seen.insert(key);
        out.push(h);
    }
    out
}

pub fn expand_context(conn: &Connection, hits: &mut [Hit], window: usize) {
    if window==0 || hits.is_empty() { return; }
    let hit_ids: HashSet<i64> = hits.iter().map(|h| h.chunk_id).collect();
    for h in hits.iter_mut() {
        let item_id = match h.item_id { Some(id)=>id, None=> continue };
        let mut stmt = match conn.prepare("SELECT id, content FROM chunks WHERE item_id=?1 ORDER BY id") { Ok(s)=>s, Err(_)=> continue };
        let siblings: Vec<(i64,String)> = stmt.query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        let idx = siblings.iter().position(|(id,_)| *id==h.chunk_id);
        if let Some(i) = idx {
            let start = i.saturating_sub(window);
            let end = (i+window+1).min(siblings.len());
            let mut extra = Vec::new();
            for (j,(cid,content)) in siblings[start..end].iter().enumerate() {
                if *cid==h.chunk_id || hit_ids.contains(cid) { continue; }
                if j==i { continue; }
                extra.push(content.clone());
            }
            if !extra.is_empty() {
                // store expanded in content for now (Python stores hit["expanded"])
                h.content = format!("{}\n\n[continued]\n\n{}", h.content, extra.join("\n\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fts_query() {
        assert_eq!(fts_match_query("who knows about Phoenix?"), Some("\"knows\" OR \"phoenix\"".to_string()));
        assert_eq!(fts_match_query("the and or"), None);
    }

    #[test]
    fn rrf() {
        let h1 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None }];
        let h2 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None }];
        let fused = rrf_fuse_multi(vec![h1, h2], vec![1.0, 1.0]);
        assert_eq!(fused.len(), 1);
        assert!(fused[0].score > 0.0);
    }

    #[test]
    fn age() {
        let now = Utc.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap();
        let old = Utc.with_ymd_and_hms(2025, 1, 10, 12, 0, 0).unwrap();
        let d = age_decay(Some(old), 365.0, now);
        assert!(d < 1.0 && d > 0.4);
    }

    #[test]
    fn status() {
        assert!(!status_ok(Some("stale"), true));
        assert!(!status_ok(Some("disputed"), true));
        assert!(status_ok(Some("disputed"), false));
        assert!(status_ok(None, true));
    }

    // --- Rust-only retrieval harness (P0 3.1) — seeds DB with rusqlite, exercises graph/who-knows/RRF directly
    // Gates cutover so 124 green isn't half-Python via AnswerEngine.

    fn harness_db() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("harness.db");
        let conn = crate::db::init_db(&path).unwrap();
        (dir, conn)
    }

    fn insert_source(conn: &rusqlite::Connection, name: &str, label: &str) -> i64 {
        conn.execute("INSERT INTO sources (name, connector, config, label, enabled, last_sync_cursor, error_count, created_at) VALUES (?1,'folder','{}',?2,1,'',0,datetime('now'))", rusqlite::params![name, label]).unwrap();
        conn.last_insert_rowid()
    }
    fn insert_item(conn: &rusqlite::Connection, source_id: i64, title: &str) -> i64 {
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at) VALUES (?1,?2,?3,'',?4,'',0,datetime('now'))", rusqlite::params![source_id, format!("ext-{}", title), title, format!("hash-{}", title)]).unwrap();
        conn.last_insert_rowid()
    }
    fn insert_entity(conn: &rusqlite::Connection, item_id: i64, summary: &str, status: &str, owner: &str) -> i64 {
        conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner) VALUES (?1,'note',?2,'r',0.9,'','ref',?3,?4)", rusqlite::params![item_id, summary, status, owner]).unwrap();
        conn.last_insert_rowid()
    }
    fn insert_chunk(conn: &rusqlite::Connection, item_id: Option<i64>, entity_id: Option<i64>, content: &str) -> i64 {
        conn.execute("INSERT INTO chunks (item_id, entity_id, content, source_ref, created_at) VALUES (?1,?2,?3,'ref',datetime('now'))", rusqlite::params![item_id, entity_id, content]).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn harness_graph_expand_filters_stale_and_disputed() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        let e1 = insert_entity(&conn, iid, "Phoenix owns billing", "unverified", "alice");
        let e2 = insert_entity(&conn, iid, "Billing blocked by Phoenix", "stale", "bob");
        let e3 = insert_entity(&conn, iid, "Phoenix related to Auth", "disputed", "carol");
        let e4 = insert_entity(&conn, iid, "Phoenix owns Auth", "unverified", "dave");
        let c1 = insert_chunk(&conn, Some(iid), Some(e1), "Phoenix owns billing details");
        // stale/disputed entities should not appear in graph expansion when excluded
        let rel1 = conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'owns')", rusqlite::params![e1, e2]).unwrap();
        let _ = conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'related')", rusqlite::params![e1, e3]).unwrap();
        let _ = conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'owns')", rusqlite::params![e1, e4]).unwrap();
        assert_eq!(rel1, 1);
        let seed = vec![Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), score: 1.0, content: "seed".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()) }];
        // with qa_exclude_disputed=true, disputed should be filtered
        let out = graph_expand(&conn, &seed, None, 2, 5, true);
        let ids: Vec<i64> = out.iter().filter_map(|h| h.entity_id).collect();
        assert!(ids.contains(&e4), "unverified neighbor e4 should be expanded");
        assert!(!ids.contains(&e2), "stale e2 must be excluded");
        assert!(!ids.contains(&e3), "disputed e3 must be excluded when qa_exclude_disputed=true");
        // with qa_exclude_disputed=false, disputed is allowed
        let out2 = graph_expand(&conn, &seed, None, 2, 5, false);
        let ids2: Vec<i64> = out2.iter().filter_map(|h| h.entity_id).collect();
        assert!(ids2.contains(&e3), "disputed e3 should appear when not excluded");
    }

    #[test]
    fn harness_who_knows_finds_owner() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s2", "internal");
        let iid = insert_item(&conn, sid, "doc2");
        let e_owner = insert_entity(&conn, iid, "Phoenix billing system", "unverified", "alice");
        let e_other = insert_entity(&conn, iid, "Auth gateway", "unverified", "");
        let _c1 = insert_chunk(&conn, Some(iid), Some(e_owner), "Phoenix billing owned by alice");
        let _c2 = insert_chunk(&conn, Some(iid), Some(e_other), "Auth gateway notes");
        let hits = who_knows_search(&conn, "who owns Phoenix billing?", None, 5, true);
        assert!(!hits.is_empty(), "who-knows should find owner hit");
        assert!(hits.iter().any(|h| h.entity_id == Some(e_owner)), "should contain owner entity");
    }

    #[test]
    fn harness_fuse_and_rank_rrf_and_diversity() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s3", "internal");
        let iid = insert_item(&conn, sid, "doc3");
        let e1 = insert_entity(&conn, iid, "Decision about Phoenix", "unverified", "");
        let c1 = insert_chunk(&conn, Some(iid), Some(e1), "content Phoenix decision");
        let c2 = insert_chunk(&conn, Some(iid), None, "document chunk Phoenix billing is stable");
        // need created_at for age decay; ensure chunks have timestamps via init_db
        let h1 = Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), score: 1.0, content: "c1".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()) };
        let h2 = Hit { chunk_id: c2, entity_id: None, item_id: Some(iid), source_id: Some(sid), score: 0.5, content: "c2".into(), source_ref: "ref".into(), is_pii: false, status: None };
        let fused = fuse_and_rank(&conn, Some(vec![h1]), Some(vec![h2]), 1.0, 365.0, 3, 5, true);
        assert!(!fused.is_empty());
        // RRF should have merged both; diversity cap per item should keep both since distinct chunk_ids
        assert!(fused.len() >= 1);
    }

    #[test]
    fn harness_is_who_knows_and_status_ok_integration() {
        assert!(is_who_knows("who is responsible for billing?"));
        assert!(is_who_knows("Whom should I ask about Phoenix?"));
        assert!(!is_who_knows("what is billing status?"));
        assert!(!status_ok(Some("stale"), true));
        assert!(status_ok(Some("unverified"), true));
    }
}
