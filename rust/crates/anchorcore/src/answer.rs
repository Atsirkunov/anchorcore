//! R2.2: Answer orchestration — port of `backend/app/answer_engine.py:169` `ask()`
//! - project scoping, public_only gate, planner, executor, graph, B30 gate, generation

use crate::embedder::Embedder;
use crate::retrieval;
use crate::settings::SettingsService;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct AskRequest {
    pub question: String,
    pub history: Option<Vec<AskTurn>>,
    pub project_id: Option<i64>,
    pub public_only: Option<bool>,
    /// Opt back into full chunk bytes on citations (default: compact
    /// 200/500-char previews). Mirrors `SearchRequest.include_content`.
    pub include_content: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AskTurn {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct AskResponse {
    pub answer: String,
    pub citations: Vec<Citation>,
    /// Machine-readable gate outcome (Phase-1 deterministic reads): None when
    /// content was returned, Some("gated_provider") when everything relevant
    /// was withheld from an untrusted answer provider.
    pub refusal: Option<String>,
    /// B58: how the answer text was produced — "composed" (an answer model
    /// wrote it), "agent_compose" (R45.2 agent mode: the caller's agent
    /// writes it from the citations, by design), "context_only" (local
    /// mode, matching context, no model ran), "none".
    pub mode: String,
    /// B58: how retrieval ranked — "hybrid" (vector + keyword on a real
    /// query embedding) or "keyword_only" (embedder unavailable).
    pub retrieval: String,
    /// R23.4: plain-language degradation warning — None only when
    /// `mode` is "composed" or (R45.2) "agent_compose". UIs render it as the banner; MCP `ask`
    /// passes it through so agents never quote context as an answer.
    pub warning: Option<String>,
}

#[derive(Serialize)]
pub struct Citation {
    pub entity_id: Option<i64>,
    /// R32.2: every citation drills into GET /items/:id (entity citations
    /// resolve via their entity — never None for stored hits).
    pub item_id: Option<i64>,
    /// R36.3: primary matched chunk (the `[match N]` mark in `content`;
    /// merged parents mark every match, this names the first-ranked one).
    pub chunk_id: Option<i64>,
    /// R36.3: returned scope — smallest fitting ancestor for parent hits,
    /// direct section for fallback hits, None when section-less.
    pub section_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub source_ref: String,
    pub path: String,
    /// R36.3: returned scope's section breadcrumb (`sections.path`), ""
    /// when section-less. Unlike `path`, never a file-path fallback.
    pub breadcrumb: String,
    pub tags: Vec<String>,
    pub score: f64,
    pub snippet: String,
    pub doc_type: String,
    /// R32.4: path-derived class — "live", or "test" for fixture/sandbox paths.
    pub env: String,
    pub conflicts_with: Vec<i64>,
    pub content: String,
    pub item_title: String,
    pub content_truncated: bool,
    /// 1-based source lines (0/0 when untracked).
    pub line_start: i64,
    pub line_end: i64,
    /// R39.1: contributing legs + ranks from the last fusion (`[]` unranked).
    pub provenance: Vec<retrieval::LegMatch>,
    /// R39.1: age-decay factor applied to `score` (1.0 = none).
    pub decay: f64,
    /// R39.1: surfaced by filename boost above the ranked list.
    pub boosted: bool,
    /// R46.9/R46.11: images on the cited chunk/section — every ask mode
    /// (agent_compose for the agent, local for the UI's source cards).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<crate::assets::AssetRef>>,
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub k: Option<usize>,
    pub project_id: Option<i64>,
    pub public_only: Option<bool>,
    /// A3 compact-by-default selector: comma list from
    /// `summary,source_ref,score,content` (or `all`). Chunk `content` is
    /// omitted unless requested — harnesses walk cheap citable hits, then
    /// drill down verbatim via get_entity/context.
    pub fields: Option<String>,
    /// A3 cursor pagination: zero-based offset into the gated hit list.
    pub cursor: Option<usize>,
    /// A3 cursor pagination: page size (defaults to `k`).
    pub limit: Option<usize>,
    /// Convenience alias for `fields: content` — agents commonly ask for the
    /// chunk bytes by this name. `fields` remains the canonical selector.
    pub include_content: Option<bool>,
    /// R32.4: env filter — "live" or "test" (case-insensitive). Anything
    /// else (or absent) means unfiltered; live-first still orders the mix.
    pub env: Option<String>,
    /// R36.6: chunk-kind filter — comma list from
    /// `document,entity,section_summary,doc_summary`
    /// (case-insensitive; unknown or empty means unfiltered, like `env`).
    pub kind: Option<String>,
    /// R36.6: item-path prefix filter — relative like the items listing, or
    /// absolute (resolved through the owning folder source's root).
    pub path_prefix: Option<String>,
    /// R36.6: code-symbol filter — keeps hits whose section title/path names
    /// the symbol (case-insensitive substring; section-less chunks drop out).
    pub symbol: Option<String>,
    /// R33.5: retrieval mode — `"exact"` selects literal-substring search
    /// (case-sensitive, document order, unranked, no embedding call);
    /// absent/unknown means ranked hybrid (env-consistent: a typo never
    /// changes the leg). Same scope/gate/filter contract either way.
    pub mode: Option<String>,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub chunk_id: Option<i64>,
    pub entity_id: Option<i64>,
    /// R36.3: returned scope — smallest fitting ancestor for parent hits,
    /// direct section for fallback hits, None when section-less.
    pub section_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub content: String,
    pub source_ref: String,
    pub path: String,
    /// R36.3: returned scope's section breadcrumb (`sections.path`), ""
    /// when section-less. Unlike `path`, never a file-path fallback.
    pub breadcrumb: String,
    pub tags: Vec<String>,
    pub source_id: Option<i64>,
    pub item_id: Option<i64>,
    pub item_title: String,
    pub doc_type: String,
    /// R32.4: path-derived class — "live", or "test" for fixture/sandbox paths.
    pub env: String,
    pub conflicts_with: Vec<i64>,
    pub score: f64,
    pub content_truncated: bool,
    /// 1-based source lines (0/0 when untracked).
    pub line_start: i64,
    pub line_end: i64,
    /// R39.1: contributing legs + ranks from the last fusion (`[]` unranked).
    pub provenance: Vec<retrieval::LegMatch>,
    /// R39.1: age-decay factor applied to `score` (1.0 = none).
    pub decay: f64,
    /// R39.1: surfaced by filename boost above the ranked list.
    pub boosted: bool,
    /// R46.9: opt-in (`fields: images` or `all`) — absent otherwise, so the
    /// default hit shape never changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<crate::assets::AssetRef>>,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// Count of hits withheld by the provider-trust gate (Phase-1).
    pub blocked: usize,
    /// Some("gated_provider") when relevant hits existed but none survived
    /// gating — lets harnesses react deterministically instead of guessing.
    pub refusal: Option<String>,
    /// A3 cursor for the next page (offset into the gated hit list), None
    /// when this page exhausts what the caller may see.
    pub next_cursor: Option<usize>,
    /// B58: how retrieval ranked — "hybrid" or "keyword_only" (see AskResponse).
    pub retrieval: String,
}

async fn rewrite_question(settings: &SettingsService, req: &AskRequest) -> String {
    // R10.1: follow-up rewrite — history + trusted gate (like Python ask: rewrite before retrieval)
    let original_question = req.question.clone();
    let history = req.history.clone().unwrap_or_default();
    let filtered_history: Vec<AskTurn> = history.into_iter().filter(|t| !t.content.trim().is_empty()).collect();
    let trusted_for_rewrite = is_answer_trusted(settings);
    if filtered_history.is_empty() || !trusted_for_rewrite {
        return original_question;
    }
    if let Some(rewritten) = rewrite_followup(settings, &original_question, &filtered_history).await {
        if !rewritten.trim().is_empty() && rewritten != original_question {
            return rewritten;
        }
    }
    original_question
}

/// R45.13: append local-model acronym expansions to the retrieval query.
/// Same trust gate as the follow-up rewrite (the question goes to the
/// answer model); acronyms the vocabulary already knows skip the call.
async fn expand_acronyms(settings: &SettingsService, question: &str, data_dir: &str) -> String {
    let acronyms = crate::query_expand::query_acronyms(question);
    if acronyms.is_empty() || !crate::query_expand::enabled(settings) || !is_answer_trusted(settings) {
        return question.to_string();
    }
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    if !crate::common::provider_is_local(&base) && settings.get("answer_api_key", None).unwrap_or_default().is_empty() {
        return question.to_string();
    }
    let data_dir = data_dir.to_string();
    let asked = acronyms.clone();
    let known = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::vocab::known_terms(&conn, &asked)
    })
    .await
    .unwrap_or_default();
    let unknown: Vec<String> = acronyms.into_iter().filter(|a| !known.contains(&a.to_lowercase())).collect();
    let extras = crate::query_expand::expand(settings, &unknown, answer_timeout(settings)).await;
    if extras.is_empty() {
        return question.to_string();
    }
    format!("{} {}", question, extras.join(" "))
}

async fn record_degraded(data_dir: &str) {
    // R10.5: only record qa warnings when retrieval actually degraded (hits empty) — not on every qa
    // Python records these only when embedding failed / no hits; previously Rust spammed 2 warnings per qa
    let data_dir = data_dir.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::init_db(&db_path) {
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning','retrieval degraded to keyword-only','qa fallback')", []);
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning','answer generation failed; returning matching context only','qa fallback')", []);
        }
    })
    .await;
}

pub(crate) async fn record_gate_event(data_dir: &str, blocked: &[String]) {
    let data_dir = data_dir.to_string();
    let blocked_len = blocked.len();
    let labels = blocked.to_vec();
    let _ = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::init_db(&db_path) {
            let msg = format!("answer gate: {} hit(s) excluded from cloud answer (sensitive/pii source)", blocked_len);
            let detail = format!("labels={:?}; set ANCHOR_CLOUD_TRUST=1 to allow", labels);
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning',?1,?2)", rusqlite::params![msg, detail]);
        }
    })
    .await;
}

fn display_source_ref(h: &crate::retrieval::Hit) -> String {
    if h.source_ref.is_empty() { format!("chunk:{}", h.chunk_id) } else { h.source_ref.clone() }
}

/// R46.12: a person's image caption cites as `caption` (its path ends in
/// `captions::CAPTION_CRUMB`), never as document text.
fn hit_kind(h: &crate::retrieval::Hit, kinds: &std::collections::HashMap<i64, String>) -> String {
    if h.entity_id.is_some() {
        "entity".to_string()
    } else if kinds.get(&h.chunk_id).is_some_and(|k| k == crate::captions::CAPTION_KIND) {
        crate::captions::CAPTION_KIND.to_string()
    } else {
        "document".to_string()
    }
}

fn rounded_score(score: f64) -> f64 {
    (score * 1000.0).round() / 1000.0
}

fn tags_for_chunk(conn: &Connection, chunk_id: i64) -> Vec<String> {
    tags_for_chunks(conn, &[chunk_id]).remove(&chunk_id).unwrap_or_default()
}

/// Single batched tags lookup — replaces per-hit `tags_for_chunk` N+1.
fn tags_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, Vec<String>> {
    let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT ct.chunk_id, t.name FROM tags t JOIN chunk_tags ct ON ct.tag_id=t.id WHERE ct.chunk_id IN ({}) ORDER BY t.name",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) {
        for row in rows.flatten() {
            out.entry(row.0).or_default().push(row.1);
        }
    }
    out
}

/// Single batched doc_type lookup — marks citations derived from code.
/// Document chunks join via `item_id`, entity chunks via their entity's item.
/// Single batched conflict lookup — drives the "contradicts #N" citation
/// warning. Links to `stale` counterparts are resolved, hence excluded.
fn conflicts_for_entities(conn: &Connection, entity_ids: &[i64]) -> std::collections::HashMap<i64, Vec<i64>> {
    let mut out: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    if entity_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT r.from_entity_id, r.to_entity_id FROM relationships r JOIN entities a ON a.id=r.from_entity_id JOIN entities b ON b.id=r.to_entity_id WHERE r.kind='conflicts_with' AND (r.from_entity_id IN ({}) OR r.to_entity_id IN ({})) AND COALESCE(a.status,'') != 'stale' AND COALESCE(b.status,'') != 'stale'",
        crate::common::placeholders(entity_ids.len()),
        crate::common::placeholders(entity_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let mut params: Vec<i64> = Vec::with_capacity(entity_ids.len() * 2);
    params.extend_from_slice(entity_ids);
    params.extend_from_slice(entity_ids);
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    }) {
        for (from, to) in rows.flatten() {
            for (a, b) in [(from, to), (to, from)] {
                if entity_ids.contains(&a) {
                    out.entry(a).or_default().push(b);
                }
            }
        }
    }
    out
}

/// Chunk-level conflict map for citations. Entity chunks attribute via their
/// own entity; document chunks (the common citation) via live entities on
/// the same item — a file containing a conflicted decision warns, naming the
/// counterpart, instead of silently disagreeing.
fn conflicts_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, Vec<i64>> {
    let mut out: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT id, entity_id, item_id FROM chunks WHERE id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows: Vec<(i64, Option<i64>, Option<i64>)> = match stmt.query_map(
        rusqlite::params_from_iter(chunk_ids.iter()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(_) => return out,
    };
    let item_ids: Vec<i64> = rows.iter().filter_map(|(_, e, i)| if e.is_none() { *i } else { None }).collect();
    let item_entities = entities_for_items(conn, &item_ids);
    let mut candidate_ids: Vec<i64> = rows.iter().filter_map(|(_, e, _)| *e).collect();
    for ids in item_entities.values() {
        candidate_ids.extend(ids.iter().copied());
    }
    let by_entity = conflicts_for_entities(conn, &candidate_ids);
    for (cid, entity_id, item_id) in rows {
        let mut warned: Vec<i64> = Vec::new();
        if let Some(eid) = entity_id {
            if let Some(ids) = by_entity.get(&eid) {
                warned.extend(ids.iter().copied());
            }
        } else if let Some(iid) = item_id {
            if let Some(eids) = item_entities.get(&iid) {
                for eid in eids {
                    if let Some(ids) = by_entity.get(eid) {
                        warned.extend(ids.iter().copied());
                    }
                }
            }
        }
        warned.sort_unstable();
        warned.dedup();
        if !warned.is_empty() {
            out.insert(cid, warned);
        }
    }
    out
}

/// Live (non-stale) entity ids per item — stale entities are resolved, so
/// their items must not warn.
fn entities_for_items(conn: &Connection, item_ids: &[i64]) -> std::collections::HashMap<i64, Vec<i64>> {
    let mut out: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    if item_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT item_id, id FROM entities WHERE item_id IN ({}) AND COALESCE(status,'') != 'stale'",
        crate::common::placeholders(item_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(item_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    }) {
        for (iid, eid) in rows.flatten() {
            out.entry(iid).or_default().push(eid);
        }
    }
    out
}

fn doc_types_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, String> {
    let mut out: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT c.id, COALESCE(i1.doc_type, i2.doc_type, '') FROM chunks c LEFT JOIN ingested_items i1 ON i1.id=c.item_id LEFT JOIN entities e ON e.id=c.entity_id LEFT JOIN ingested_items i2 ON i2.id=e.item_id WHERE c.id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) {
        for row in rows.flatten() {
            out.insert(row.0, row.1);
        }
    }
    out
}

/// Batched item provenance for search hits: `(item_title, item_path)` where
/// item_path is the item's external id (folder sources: the relative file
/// path). Document chunks join via `item_id`, entity chunks via their
/// entity's item.
fn item_meta_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, (String, String)> {
    let mut out: std::collections::HashMap<i64, (String, String)> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT c.id, COALESCE(i1.title, i2.title, ''), COALESCE(i1.external_id, i2.external_id, '') FROM chunks c LEFT JOIN ingested_items i1 ON i1.id=c.item_id LEFT JOIN entities e ON e.id=c.entity_id LEFT JOIN ingested_items i2 ON i2.id=e.item_id WHERE c.id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }) {
        for (id, title, path) in rows.flatten() {
            out.insert(id, (title, path));
        }
    }
    out
}

/// R36.3: batched section-breadcrumb lookup — one query per search/ask,
/// keyed by the hit's returned scope (`Hit.section_id`).
fn section_breadcrumbs(conn: &Connection, section_ids: &[i64]) -> std::collections::HashMap<i64, String> {
    let mut out: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if section_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT id, path FROM sections WHERE id IN ({})",
        crate::common::placeholders(section_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(section_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) {
        for (id, path) in rows.flatten() {
            out.insert(id, path);
        }
    }
    out
}

/// R36.3: match-centered preview on returned-parent content — the mark shows
/// where the match sits inside the parent; unmarked hits keep `match_snippet`.
fn hit_preview(content: &str, query: &str, chunk_id: i64, n: usize) -> String {
    crate::retrieval::content_window(content, chunk_id, n)
        .unwrap_or_else(|| crate::retrieval::match_snippet(content, query, n))
}

/// Citation count knob: `ANCHOR_CITATIONS`, default 5. The B55 golden set
/// tunes this with data instead of guesses.
fn citations_limit() -> usize {
    std::env::var("ANCHOR_CITATIONS").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(5)
}

/// Batched 1-based source line ranges for citable chunks (R38.3; 0/0 when
/// the chunker didn't track lines). One query per response, like the maps
/// above — the `Hit` struct stays untouched.
fn line_ranges_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, (i64, i64)> {
    let mut out: std::collections::HashMap<i64, (i64, i64)> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT id, line_start, line_end FROM chunks WHERE id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    }) {
        for (id, start, end) in rows.flatten() {
            out.insert(id, (start, end));
        }
    }
    out
}

fn build_citations(hits: &[crate::retrieval::Hit], data_dir: &str, want_content: bool, limit: usize, query: &str) -> Vec<Citation> {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    let ids: Vec<i64> = hits.iter().take(limit).map(|h| h.chunk_id).collect();
    let conflict_map = conflicts_for_chunks(&conn, &ids);
    let tag_map = tags_for_chunks(&conn, &ids);
    let doc_type_map = doc_types_for_chunks(&conn, &ids);
    let meta_map = item_meta_for_chunks(&conn, &ids);
    let env_map = crate::retrieval::envs_for_chunks(&conn, &ids);
    let range_map = line_ranges_for_chunks(&conn, &ids);
    let section_ids: Vec<i64> = hits.iter().take(limit).filter_map(|h| h.section_id).collect();
    let crumb_map = section_breadcrumbs(&conn, &section_ids);
    let kind_map = kinds_for_chunks(&conn, &ids);
    hits.iter().take(limit).map(|h| {
        // Path rule mirrors search hits: section breadcrumb when the chunk
        // lives in one, else the item's file path — entity citations must
        // stay attributable to their source file.
        let (item_title, item_path) = meta_map.get(&h.chunk_id).cloned().unwrap_or_default();
        let (line_start, line_end) = range_map.get(&h.chunk_id).copied().unwrap_or((0, 0));
        Citation {
            entity_id: h.entity_id,
            item_id: h.item_id,
            chunk_id: Some(h.chunk_id),
            section_id: h.section_id,
            kind: hit_kind(h, &kind_map),
            summary: hit_preview(&h.content, query, h.chunk_id, 200),
            source_ref: display_source_ref(h),
            path: if h.path.is_empty() { item_path } else { h.path.clone() },
            breadcrumb: h.section_id.and_then(|sid| crumb_map.get(&sid).cloned()).unwrap_or_default(),
            item_title,
            tags: tag_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            doc_type: doc_type_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            // No marker evidence (unknown chunk) reads as live, not test.
            env: env_map.get(&h.chunk_id).cloned().unwrap_or_else(|| "live".to_string()),
            conflicts_with: conflict_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            score: rounded_score(h.score),
            snippet: hit_preview(&h.content, query, h.chunk_id, 500),
            content: if want_content { h.content.clone() } else { String::new() },
            content_truncated: h.content_truncated,
            line_start,
            line_end,
            provenance: h.provenance.clone(),
            decay: rounded_score(h.decay),
            boosted: h.boosted,
            images: Some(crate::assets::refs_for_hit(&conn, h.chunk_id, h.section_id)),
        }
    }).collect()
}

pub async fn ask(
    settings: &SettingsService,
    embedder: &Arc<Embedder>,
    req: AskRequest,
    data_dir: &str,
) -> AskResponse {
    // R45.2: agent mode runs no chat model — not even the follow-up
    // rewrite (the agent already sends standalone questions).
    let agent_mode = crate::settings::is_agent_mode(settings);
    let question = if agent_mode { req.question.clone() } else { rewrite_question(settings, &req).await };
    // R45.13: local mode may widen the *retrieval* query with acronym
    // expansions from the answer model; the answer prompt keeps `question`.
    let retrieval_q = if agent_mode { question.clone() } else { expand_acronyms(settings, &question, data_dir).await };
    // R8.1: embed query for vector search (hybrid) — B58 strict: None when
    // remote unavailable, so retrieval degrades to honest keyword-only.
    let query_embedding = embedder.embed_query(&retrieval_q).await;
    let retrieval = if query_embedding.is_some() { "hybrid" } else { "keyword_only" };
    // Do DB retrieval in blocking thread to avoid holding !Send Connection across await
    let data_dir_string = data_dir.to_string();
    let data_dir_for_hits = data_dir_string.clone();
    let project_id = req.project_id;
    let public_only = req.public_only.unwrap_or(false);
    let history_for_gen: Vec<AskTurn> = req.history.clone().unwrap_or_default().into_iter().filter(|t| !t.content.trim().is_empty()).collect();
    let settings_clone = settings_snapshot(settings);
    let q_for_block = retrieval_q.clone();
    let hits = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir_for_hits);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        retrieve_sync(&conn, &q_for_block, project_id, public_only, &settings_clone, query_embedding)
    })
    .await
    .unwrap_or_default();

    if hits.is_empty() {
        record_degraded(&data_dir_string).await;
        return AskResponse {
            answer: "No relevant knowledge found yet. Ingest sources first.".to_string(),
            citations: vec![],
            refusal: None,
            mode: "none".to_string(),
            retrieval: retrieval.to_string(),
            warning: degraded_warning("none", None),
        };
    }

    let trusted = is_answer_trusted(settings);
    let mut hits = hits;
    if !trusted {
        let hits_for_gate = hits.clone();
        let data_dir_gate = data_dir_string.clone();
        let (filtered, blocked) = tokio::task::spawn_blocking(move || {
            let db_path = crate::db::resolve_db_path(&data_dir_gate);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            gate_answer_hits(&conn, &hits_for_gate)
        }).await.unwrap_or((vec![], vec![]));
        if !blocked.is_empty() {
            record_gate_event(&data_dir_string, &blocked).await;
            if filtered.is_empty() {
                return AskResponse {
                    answer: "Relevant knowledge was found but it is sensitive/PII and the answer provider is not approved for it. Enable a local provider, list it in ANCHOR_TRUSTED_PROVIDERS, or set ANCHOR_CLOUD_TRUST=1 to answer.".to_string(),
                    citations: vec![],
                    refusal: Some("gated_provider".to_string()),
                    mode: "none".to_string(),
                    retrieval: retrieval.to_string(),
                    warning: degraded_warning("none", Some("gated_provider")),
                };
            }
        }
        hits = filtered;
    }

    if agent_mode {
        return agent_compose_response(&hits, &data_dir_string, &question, retrieval);
    }

    // Build context and generate (no DB borrow across await) — R10.6 UTF-8 safe (was byte slice).
    // R36.3: parent hits center the window on the match mark (a head cut
    // could drop the match); unmarked hits keep the byte-identical head.
    // R43.7: whole-section assembly under a token budget (rank order kept).
    let context = build_answer_context(&hits);
    let (answer, composed) = generate_answer(settings, &question, &context, &history_for_gen).await;
    let want_content = req.include_content.unwrap_or(false);
    let citations = build_citations(&hits, &data_dir_string, want_content, citations_limit(), &question);
    let mode = if composed { "composed" } else { "context_only" };
    let warning = degraded_warning(mode, None);
    AskResponse { answer, citations, refusal: None, mode: mode.to_string(), retrieval: retrieval.to_string(), warning }
}

/// R45.2: one-line instruction carried as `answer` in `agent_compose` mode.
pub(crate) const AGENT_COMPOSE_INSTRUCTION: &str = "Compose the answer yourself from the cited passages below; cite them by source_ref.";

/// R45.2: agent mode — the agent writes the answer, by design. Same scope,
/// fusion and label gate as `composed`; citations always carry content
/// (as with `include_content`) and there is no warning: nothing degraded.
fn agent_compose_response(hits: &[crate::retrieval::Hit], data_dir: &str, question: &str, retrieval: &str) -> AskResponse {
    // R46.9: the agent sees images at question time — refs ride on citations.
    let citations = build_citations(hits, data_dir, true, citations_limit(), question);
    AskResponse {
        answer: AGENT_COMPOSE_INSTRUCTION.to_string(),
        citations,
        refusal: None,
        mode: "agent_compose".to_string(),
        retrieval: retrieval.to_string(),
        warning: None,
    }
}

/// R23.4: plain-language degradation warning — `None` only when an answer
/// model actually composed the text (`mode == "composed"`). Every other
/// mode warns, so UIs and agents can never render degraded output without
/// saying so. Surfaced as `AskResponse.warning` + MCP `ask` output.
fn degraded_warning(mode: &str, refusal: Option<&str>) -> Option<String> {
    if mode == "composed" {
        return None;
    }
    if refusal == Some("gated_provider") {
        return Some("Relevant knowledge exists but is sensitive/PII and the answer provider is not approved for it, so no answer was composed. Do not treat this as an answer.".to_string());
    }
    if mode == "none" {
        return Some("No relevant knowledge was found, so no answer was composed. Do not treat this as an answer.".to_string());
    }
    Some("No answer model ran — this is matching context, not a composed answer. Do not quote it as an answer; verify against the citations, or configure an answer model (Settings → Answer model: local Ollama or your own OpenAI-compatible endpoint).".to_string())
}

fn settings_snapshot(settings: &SettingsService) -> std::collections::HashMap<String, String> {
    // snapshot needed for blocking thread (SettingsService is Sync but we clone needed values)
    let mut m = std::collections::HashMap::new();
    for k in ["answer_base_url","answer_api_key","answer_model","ollama_base_url"] {
        if let Some(v) = settings.get(k, None) { m.insert(k.to_string(), v); }
    }
    m
}

// --- R15.2: single retrieval orchestration shared by ask + search ---

struct RetrievalConfig {
    top_k: usize,
    keyword_weight: f64,
    halflife: f64,
    max_per_source: usize,
    qa_exclude_disputed: bool,
}

impl RetrievalConfig {
    fn from_env_with_top_k(top_k: usize) -> Self {
        let qa_exclude_disputed: bool = std::env::var("ANCHOR_QA_EXCLUDE_DISPUTED")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);
        let keyword_weight: f64 = std::env::var("ANCHOR_RETRIEVAL_KEYWORD_WEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let halflife: f64 = std::env::var("ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(365.0);
        let max_per_source: usize = std::env::var("ANCHOR_RETRIEVAL_MAX_PER_SOURCE").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        Self { top_k, keyword_weight, halflife, max_per_source, qa_exclude_disputed }
    }

    fn from_env() -> Self {
        let top_k: usize = std::env::var("ANCHOR_TOP_K").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
        Self::from_env_with_top_k(top_k)
    }
}

pub(crate) fn project_scope(conn: &Connection, project_id: Option<i64>, public_only: bool) -> Option<HashSet<i64>> {
    let mut scope = project_source_ids(conn, project_id);
    if public_only {
        let public_ids = public_source_ids(conn);
        scope = match scope {
            None => Some(public_ids),
            Some(s) => Some(s.intersection(&public_ids).cloned().collect()),
        };
    }
    scope
}

/// Ranked retrieval for ask + search: scope resolution and embedding stay
/// here; the pure fusion path (TOC-or-flat legs → RRF fuse → `fuse_and_rank`
/// → keyword slots → filename boost → `parent_return` → status re-filter →
/// relevance gate) lives in `retrieval::orchestrate_fusion` (R39.12 —
/// shared with the R39.10 eval so the two can never drift).
fn orchestrate(
    conn: &Connection,
    query: &str,
    query_embedding: Option<Vec<f32>>,
    scope: Option<HashSet<i64>>,
    cfg: &RetrievalConfig,
) -> Vec<crate::retrieval::Hit> {
    retrieval::orchestrate_fusion(
        conn,
        query,
        query_embedding.as_deref(),
        scope.as_ref(),
        cfg.top_k,
        cfg.keyword_weight,
        cfg.halflife,
        cfg.max_per_source,
        cfg.qa_exclude_disputed,
    )
}

fn retrieve_sync(
    conn: &Connection,
    question: &str,
    project_id: Option<i64>,
    public_only: bool,
    _settings_map: &std::collections::HashMap<String, String>,
    query_embedding: Option<Vec<f32>>,
) -> Vec<crate::retrieval::Hit> {
    let scope = project_scope(conn, project_id, public_only);
    let cfg = RetrievalConfig::from_env();
    // B30 gate moved to ask() for recording + trusted-source hack (R6.4)
    orchestrate(conn, question, query_embedding, scope, &cfg)
}

fn project_source_ids(conn: &Connection, project_id: Option<i64>) -> Option<HashSet<i64>> {
    let pid = project_id?;
    let mut stmt = conn.prepare("SELECT source_id FROM project_sources WHERE project_id = ?1").ok()?;
    let rows: Vec<i64> = stmt.query_map([pid], |r| r.get(0)).ok()?.filter_map(|r| r.ok()).collect();
    if rows.is_empty() { Some(HashSet::new()) } else { Some(rows.into_iter().collect()) }
}

fn public_source_ids(conn: &Connection) -> HashSet<i64> {
    let mut stmt = conn.prepare("SELECT id FROM sources WHERE label = 'public'").unwrap();
    stmt.query_map([], |r| r.get::<_, i64>(0)).unwrap().filter_map(|r| r.ok()).collect()
}

fn gate_answer_hits(conn: &Connection, hits: &[retrieval::Hit]) -> (Vec<retrieval::Hit>, Vec<String>) {
    let sids: HashSet<i64> = hits.iter().filter_map(|h| h.source_id).collect();
    let mut labels: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if !sids.is_empty() {
        let sql = format!("SELECT id, label FROM sources WHERE id IN ({})", crate::common::placeholders(sids.len()));
        if let Ok(mut stmt) = conn.prepare(&sql) {
            for row in stmt.query_map(rusqlite::params_from_iter(sids.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
                labels.insert(row.0, row.1);
            }
        }
    }
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    for h in hits {
        // R35.1: label-only — the chunk-`is_pii` half is gone with auto-detect.
        // Manual item marks gate cloud flows via a follow-up (R35.2).
        let label = h.source_id.and_then(|id| labels.get(&id).cloned()).unwrap_or_else(|| "internal".to_string());
        if label == "sensitive" || label == "pii" {
            blocked.push(label);
        } else {
            allowed.push(h.clone());
        }
    }
    (allowed, blocked)
}

/// R45.2: answers are ready in agent mode with no model at all — the agent
/// composes. Local mode keeps the model probe's verdict.
pub(crate) fn answer_ready(settings: &SettingsService, model_ready: bool) -> bool {
    model_ready || crate::settings::is_agent_mode(settings)
}

pub(crate) fn is_answer_trusted(settings: &SettingsService) -> bool {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    crate::common::provider_trusted(&base)
}

pub async fn search(
    settings: &SettingsService,
    embedder: &Arc<Embedder>,
    req: SearchRequest,
    data_dir: &str,
) -> SearchResponse {
    // R33.5: exact mode skips the embedding call entirely (nothing to
    // rank) and reports its own leg — never hybrid/keyword_only.
    let is_exact = is_exact_mode(&req.mode);
    let query_for_embed = req.query.clone();
    let query_embedding = embed_unless_exact(embedder, &query_for_embed, is_exact).await;
    // B58 strict: None when remote unavailable → honest keyword-only.
    let retrieval = retrieval_label(is_exact, query_embedding.is_some());
    let data_dir_string = data_dir.to_string();
    let data_dir_for_block = data_dir_string.clone();
    let query = req.query.clone();
    // R40.1: `limit` pages over the `k`-sized candidate list, so a bare
    // `limit: 50` must widen retrieval too — otherwise the default k=8
    // silently caps every page (1.0.29 live report: limit 20/50/100 → 7).
    let want = req.limit.unwrap_or(0).saturating_add(req.cursor.unwrap_or(0));
    let k = req.k.unwrap_or(8).max(want).clamp(1, 50);
    let project_id = req.project_id;
    let public_only = req.public_only.unwrap_or(false);
    // A3: compact-by-default — full chunk bytes only on explicit opt-in.
    let want_content = req.include_content.unwrap_or(false)
        || req
            .fields
            .as_deref()
            .map(|f| f.split(',').any(|t| { let t = t.trim(); t == "content" || t == "all" }))
            .unwrap_or(false);
    let want_images = wants_field(&req.fields, "images");
    // A3: cursor pagination over the gated list (deterministic full walks).
    let limit = req.limit.unwrap_or(k).clamp(1, 50);
    let cursor = req.cursor.unwrap_or(0);
    let filters = SearchFilters::from_request(&req);
    // Match-centered summaries need the query after `query` moves into the
    // blocking closure below.
    let query_text = req.query.clone();
    let _settings_clone = settings_snapshot(settings);
    let hits = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir_for_block);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        // Same downstream tail (provider gate, pagination, hit mapping) —
        // only the candidate leg differs.
        if is_exact {
            exact_search_sync_inner(&conn, &query, k, project_id, public_only, &filters)
        } else {
            search_sync_inner(&conn, &query, k, project_id, public_only, query_embedding, &filters)
        }
    })
    .await
    .unwrap_or_default();
    // Same provider-trust gate as ask — an untrusted provider must not
    // receive sensitive/pii-source content through the deterministic read path.
    let mut hits = hits;
    let mut blocked_count = 0usize;
    let mut refusal: Option<String> = None;
    if !is_answer_trusted(settings) && !hits.is_empty() {
        let hits_for_gate = hits.clone();
        let data_dir_gate = data_dir_string.clone();
        let (filtered, blocked) = tokio::task::spawn_blocking(move || {
            let db_path = crate::db::resolve_db_path(&data_dir_gate);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            gate_answer_hits(&conn, &hits_for_gate)
        })
        .await
        .unwrap_or((vec![], vec![]));
        blocked_count = blocked.len();
        if !blocked.is_empty() {
            record_gate_event(&data_dir_string, &blocked).await;
            if filtered.is_empty() {
                refusal = Some("gated_provider".to_string());
            }
        }
        hits = filtered;
    }
    // Paginate after gating so a harness sees a stable walk of exactly what
    // it may read; blocked/refusal still describe the whole gated set.
    let total_allowed = hits.len();
    let page: Vec<crate::retrieval::Hit> = hits.into_iter().skip(cursor).take(limit).collect();
    let next_cursor = if cursor + page.len() < total_allowed { Some(cursor + page.len()) } else { None };
    let db_path = crate::db::resolve_db_path(&data_dir_string);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    let ids: Vec<i64> = page.iter().map(|h| h.chunk_id).collect();
    let conflict_map = conflicts_for_chunks(&conn, &ids);
    let tag_map = tags_for_chunks(&conn, &ids);
    let doc_type_map = doc_types_for_chunks(&conn, &ids);
    let meta_map = item_meta_for_chunks(&conn, &ids);
    let env_map = crate::retrieval::envs_for_chunks(&conn, &ids);
    let range_map = line_ranges_for_chunks(&conn, &ids);
    let section_ids: Vec<i64> = page.iter().filter_map(|h| h.section_id).collect();
    let crumb_map = section_breadcrumbs(&conn, &section_ids);
    let kind_map = kinds_for_chunks(&conn, &ids);
    let hits = page.into_iter().map(|h| {
        // path = section breadcrumb when the chunk lives in one; flat
        // documents (no sections) fall back to the item's path within its
        // source (folder: relative file path; opaque connectors: id).
        let (item_title, item_path) = meta_map.get(&h.chunk_id).cloned().unwrap_or_default();
        let (line_start, line_end) = range_map.get(&h.chunk_id).copied().unwrap_or((0, 0));
        SearchHit {
            chunk_id: Some(h.chunk_id),
            entity_id: h.entity_id,
            section_id: h.section_id,
            kind: hit_kind(&h, &kind_map),
            summary: hit_preview(&h.content, &query_text, h.chunk_id, 500),
            content: if want_content { h.content.clone() } else { String::new() },
            content_truncated: h.content_truncated,
            source_ref: display_source_ref(&h),
            path: if h.path.is_empty() { item_path } else { h.path.clone() },
            breadcrumb: h.section_id.and_then(|sid| crumb_map.get(&sid).cloned()).unwrap_or_default(),
            tags: tag_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            doc_type: doc_type_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            // No marker evidence (unknown chunk) reads as live, not test.
            env: env_map.get(&h.chunk_id).cloned().unwrap_or_else(|| "live".to_string()),
            conflicts_with: conflict_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            source_id: h.source_id,
            item_id: h.item_id,
            item_title,
            score: rounded_score(h.score),
            line_start,
            line_end,
            provenance: h.provenance.clone(),
            decay: rounded_score(h.decay),
            boosted: h.boosted,
            images: want_images.then(|| crate::assets::refs_for_hit(&conn, h.chunk_id, h.section_id)),
        }
    }).collect();
    SearchResponse { hits, blocked: blocked_count, refusal, next_cursor, retrieval: retrieval.to_string() }
}

/// `fields` selector: `name` itself or `all` (comma list, trimmed).
fn wants_field(fields: &Option<String>, name: &str) -> bool {
    fields.as_deref().is_some_and(|f| f.split(',').any(|t| matches!(t.trim(), "all") || t.trim() == name))
}

/// R32.4: the `env` search filter — "live" or "test" (case-insensitive);
/// anything else means unfiltered.
fn normalize_env_filter(env: Option<String>) -> Option<String> {
    env.and_then(|e| {
        let v = e.trim().to_lowercase();
        if v == "live" || v == "test" {
            Some(v)
        } else {
            None
        }
    })
}

/// R36.6 normalized post-rank search filters. All cuts run after ranking on
/// existing columns only, AND-combined — ranking scores never change, so the
/// filters are retrieval-score-neutral by construction.
struct SearchFilters {
    env: Option<String>,
    kinds: Option<Vec<String>>,
    path_prefix: Option<String>,
    symbol: Option<String>,
}

impl SearchFilters {
    fn from_request(req: &SearchRequest) -> Self {
        Self {
            env: normalize_env_filter(req.env.clone()),
            kinds: normalize_kind_filter(req.kind.clone()),
            path_prefix: normalize_prefix_filter(req.path_prefix.clone()),
            symbol: normalize_symbol_filter(req.symbol.clone()),
        }
    }

    fn any(&self) -> bool {
        self.env.is_some() || self.kinds.is_some() || self.path_prefix.is_some() || self.symbol.is_some()
    }
}

/// Comma list of chunk kinds; unknown tokens drop out, and an empty result
/// means unfiltered (env-consistent — a typo never hides everything).
fn normalize_kind_filter(kind: Option<String>) -> Option<Vec<String>> {
    let raw = kind?;
    let kinds: Vec<String> = raw
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| matches!(t.as_str(), "document" | "entity" | "section_summary" | "doc_summary" | "caption"))
        .collect();
    if kinds.is_empty() { None } else { Some(kinds) }
}

fn normalize_prefix_filter(prefix: Option<String>) -> Option<String> {
    let p = prefix?;
    if p.trim().is_empty() { None } else { Some(p) }
}

/// Lowercased at the boundary so matching is a plain `contains`.
fn normalize_symbol_filter(symbol: Option<String>) -> Option<String> {
    let s = symbol?;
    let t = s.trim().to_lowercase();
    if t.is_empty() { None } else { Some(t) }
}

/// Single batched chunk-kind lookup for the `kind` filter.
fn kinds_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, String> {
    let mut out: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!("SELECT id, kind FROM chunks WHERE id IN ({})", crate::common::placeholders(chunk_ids.len()));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) {
        for (id, kind) in rows.flatten() {
            out.insert(id, kind.to_lowercase());
        }
    }
    out
}

/// Single batched item-path lookup for the `path_prefix` filter:
/// `(external_id, source_id)` per chunk — document chunks join via `item_id`,
/// entity chunks via their entity's item (same joins as `item_meta_for_chunks`).
fn item_paths_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, (String, Option<i64>)> {
    let mut out: std::collections::HashMap<i64, (String, Option<i64>)> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT c.id, COALESCE(i1.external_id, i2.external_id, ''), COALESCE(i1.source_id, i2.source_id) FROM chunks c LEFT JOIN ingested_items i1 ON i1.id=c.item_id LEFT JOIN entities e ON e.id=c.entity_id LEFT JOIN ingested_items i2 ON i2.id=e.item_id WHERE c.id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?))
    }) {
        for (id, path, sid) in rows.flatten() {
            out.insert(id, (path, sid));
        }
    }
    out
}

/// Single batched section-title lookup for the `symbol` filter:
/// `(title, path)` per chunk — empty pair when the chunk has no section.
fn section_titles_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, (String, String)> {
    let mut out: std::collections::HashMap<i64, (String, String)> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT c.id, COALESCE(s.title, ''), COALESCE(s.path, '') FROM chunks c LEFT JOIN sections s ON s.id=c.section_id WHERE c.id IN ({})",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }) {
        for (id, title, path) in rows.flatten() {
            out.insert(id, (title, path));
        }
    }
    out
}

fn filter_hits_by_env(conn: &Connection, mut hits: Vec<crate::retrieval::Hit>, env: &str) -> Vec<crate::retrieval::Hit> {
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let classes = crate::retrieval::envs_for_chunks(conn, &ids);
    hits.retain(|h| classes.get(&h.chunk_id).map(|c| c.as_str() == env).unwrap_or(false));
    hits
}

fn filter_hits_by_kind(conn: &Connection, mut hits: Vec<crate::retrieval::Hit>, kinds: &[String]) -> Vec<crate::retrieval::Hit> {
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let map = kinds_for_chunks(conn, &ids);
    hits.retain(|h| map.get(&h.chunk_id).map(|k| kinds.contains(k)).unwrap_or(false));
    hits
}

fn filter_hits_by_prefix(conn: &Connection, mut hits: Vec<crate::retrieval::Hit>, prefix: &str) -> Vec<crate::retrieval::Hit> {
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let map = item_paths_for_chunks(conn, &ids);
    let roots = crate::items::folder_roots(conn);
    hits.retain(|h| {
        map.get(&h.chunk_id).map(|(ext, sid)| crate::items::item_under_prefix(ext, *sid, &roots, prefix)).unwrap_or(false)
    });
    hits
}

fn filter_hits_by_symbol(conn: &Connection, mut hits: Vec<crate::retrieval::Hit>, symbol: &str) -> Vec<crate::retrieval::Hit> {
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let map = section_titles_for_chunks(conn, &ids);
    hits.retain(|h| {
        map.get(&h.chunk_id)
            .map(|(title, path)| title.to_lowercase().contains(symbol) || path.to_lowercase().contains(symbol))
            .unwrap_or(false)
    });
    hits
}

fn apply_search_filters(conn: &Connection, hits: Vec<crate::retrieval::Hit>, filters: &SearchFilters, k: usize) -> Vec<crate::retrieval::Hit> {
    let mut hits = hits;
    if let Some(env) = filters.env.as_deref() {
        hits = filter_hits_by_env(conn, hits, env);
    }
    if let Some(kinds) = filters.kinds.as_deref() {
        hits = filter_hits_by_kind(conn, hits, kinds);
    }
    if let Some(prefix) = filters.path_prefix.as_deref() {
        hits = filter_hits_by_prefix(conn, hits, prefix);
    }
    if let Some(symbol) = filters.symbol.as_deref() {
        hits = filter_hits_by_symbol(conn, hits, symbol);
    }
    hits.truncate(k);
    hits
}

fn search_sync_inner(
    conn: &Connection,
    query: &str,
    k: usize,
    project_id: Option<i64>,
    public_only: bool,
    query_embedding: Option<Vec<f32>>,
    filters: &SearchFilters,
) -> Vec<crate::retrieval::Hit> {
    let scope = project_scope(conn, project_id, public_only);
    // Widen the pool when filtering: cuts run after ranking, so the fetch
    // must survive them (live-first ranking keeps fixtures in play).
    let fetch_k = if filters.any() { (k * 5).clamp(k, 50) } else { k };
    let cfg = RetrievalConfig::from_env_with_top_k(fetch_k);
    let hits = orchestrate(conn, query, query_embedding, scope, &cfg);
    if !filters.any() {
        return hits;
    }
    apply_search_filters(conn, hits, filters, k)
}

/// R33.5: `"exact"` (case-insensitive, like the env filter) selects the
/// literal-substring leg; anything else means ranked.
fn is_exact_mode(mode: &Option<String>) -> bool {
    mode.as_deref().map(|m| m.trim().eq_ignore_ascii_case("exact")).unwrap_or(false)
}

/// R33.5: exact mode needs no embedding (nothing to rank) — skip the call.
async fn embed_unless_exact(embedder: &Embedder, query: &str, is_exact: bool) -> Option<Vec<f32>> {
    if is_exact { None } else { embedder.embed_query(query).await }
}

/// R33.5: exact mode reports its own leg — never hybrid/keyword_only.
fn retrieval_label(is_exact: bool, embedded: bool) -> &'static str {
    if is_exact {
        "exact"
    } else if embedded {
        "hybrid"
    } else {
        "keyword_only"
    }
}

/// R33.5: exact leg of `/qa/search` — same scope/gate/filter contract as
/// the ranked leg, but the literal-substring candidate set (no embedding,
/// no RRF, no parent expansion or filename boost: the matching leaf
/// chunks themselves, in document order).
fn exact_search_sync_inner(
    conn: &Connection,
    needle: &str,
    k: usize,
    project_id: Option<i64>,
    public_only: bool,
    filters: &SearchFilters,
) -> Vec<crate::retrieval::Hit> {
    let scope = project_scope(conn, project_id, public_only);
    let fetch_k = if filters.any() { (k * 5).clamp(k, 50) } else { k };
    let cfg = RetrievalConfig::from_env_with_top_k(fetch_k);
    let hits = crate::retrieval::exact_search(conn, needle, scope.as_ref(), fetch_k, cfg.qa_exclude_disputed);
    if !filters.any() {
        return hits;
    }
    apply_search_filters(conn, hits, filters, k)
}

fn turn_text(turn: &AskTurn) -> String {
    format!("{}: {}", turn.role, turn.content)
}

/// R43.7: answer-model HTTP timeout, shared by generation and the
/// follow-up rewrite (one knob — both call the same model endpoint, so a
/// laptop-slow model must not fail one path but not the other). Wires the
/// existing `answer_timeout` setting (seconds, default 120: prompt eval +
/// generation on a laptop regularly exceed the old 10s); env
/// `ANCHOR_ANSWER_TIMEOUT` works via the generic settings fallback.
/// Fractional values are honored (handy for hermetic timeout tests);
/// missing/bogus/non-positive values fall back to the default; capped at
/// one hour (matches the `ollama.rs` long-poll ceiling).
fn answer_timeout(settings: &SettingsService) -> std::time::Duration {
    const DEFAULT_SECS: f64 = 120.0;
    const MAX_SECS: f64 = 3600.0;
    let secs = settings.get_float("answer_timeout", DEFAULT_SECS);
    let secs = if secs.is_finite() && secs > 0.0 { secs } else { DEFAULT_SECS };
    std::time::Duration::from_secs_f64(secs.min(MAX_SECS))
}

/// R43.7: generation-context token budget (chars/4 estimate). Per-hit
/// windows are 2000 chars and `ask` fetches top_k (default 8), so default
/// prompts fit untouched — the budget only bites when `ANCHOR_TOP_K` is
/// raised or windows grow, bounding prompt growth instead of overflowing
/// a small local context window.
const ANSWER_CONTEXT_TOKEN_BUDGET: usize = 6000;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

/// R43.7: ranked, whole-section context assembly. Sections keep their
/// `[S..]` rank numbering (contiguous from S1, matching citation order);
/// trailing sections that would exceed the token budget are dropped, so
/// the model always sees the top-ranked evidence first.
fn build_answer_context(hits: &[retrieval::Hit]) -> String {
    let budget_chars = ANSWER_CONTEXT_TOKEN_BUDGET * CHARS_PER_TOKEN_ESTIMATE;
    let mut sections: Vec<String> = Vec::with_capacity(hits.len());
    let mut used_chars = 0usize;
    for (i, h) in hits.iter().enumerate() {
        let window = crate::retrieval::content_window(&h.content, h.chunk_id, 2000)
            .unwrap_or_else(|| h.content.chars().take(2000).collect::<String>());
        // Marks locate the match for agents; a small answer model copies them
        // into prose ("[S1] [match 7]"), so the generation context drops them.
        let section = format!("[S{}] {}", i + 1, crate::retrieval::strip_match_marks(&window));
        let add = section.chars().count() + if sections.is_empty() { 0 } else { 2 };
        if used_chars + add > budget_chars {
            if sections.is_empty() {
                // Unreachable while the per-hit window (2000) stays below
                // the budget, but the cap must hold even if the window
                // grows later — hard-truncate, UTF-8 safe.
                sections.push(section.chars().take(budget_chars).collect());
            }
            break;
        }
        used_chars += add;
        sections.push(section);
    }
    sections.join("\n\n")
}

async fn llm_rewrite(settings: &SettingsService, transcript: &str, question: &str) -> Option<String> {
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(answer_timeout(settings)).build().ok()?;
    let mut headers = reqwest::header::HeaderMap::new();
    if !key.is_empty() {
        if let Ok(h) = format!("Bearer {}", key).parse() {
            headers.insert(reqwest::header::AUTHORIZATION, h);
        }
    }
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You rewrite a user's follow-up question into a standalone question that includes all context from the conversation needed to answer it alone. Respond with ONLY the rewritten question, no preamble."},
            {"role": "user", "content": format!("Conversation:\n{}\n\nFollow-up: {}", transcript, question)}
        ],
        "temperature": 0.0,
        "max_tokens": 120
    });
    let resp = client.post(&url).headers(headers).json(&payload).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let j = resp.json::<serde_json::Value>().await.ok()?;
    let content = j.pointer("/choices/0/message/content").and_then(|v| v.as_str())?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(500).collect())
    }
}

fn heuristic_rewrite(question: &str, history: &[AskTurn]) -> Option<String> {
    // Fallback heuristic — only on pronoun/coref, not blanket len<40 (R11.4)
    let q_lower = question.to_lowercase();
    let is_followup = q_lower.split_whitespace().any(|w| matches!(w, "it" | "its" | "this" | "that" | "these" | "those" | "they" | "them" | "their" | "itself"))
        || q_lower.contains("its ") || q_lower.contains(" for it");
    if !is_followup {
        return None;
    }
    let last_user = history.iter().rev().find(|t| t.role == "user").map(|t| t.content.clone()).or_else(|| history.last().map(|t| t.content.clone()))?;
    Some(format!("{} {}", last_user, question).chars().take(500).collect())
}

async fn rewrite_followup(settings: &SettingsService, question: &str, history: &[AskTurn]) -> Option<String> {
    if history.is_empty() {
        return None;
    }
    // B30: don't rewrite to untrusted cloud — Python returns None when no key and non-local
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let is_local = crate::common::provider_is_local(&base);
    // if no history or same as question, skip
    let transcript = history.iter().rev().take(6).rev().map(turn_text).collect::<Vec<_>>().join("\n");
    if transcript.is_empty() {
        return None;
    }
    // R11.4: Python returns None without a model — don't pollute new-topic short questions
    if !is_local && key.is_empty() {
        return None;
    }
    if base.starts_with("http") {
        if let Some(r) = llm_rewrite(settings, &transcript, question).await {
            return Some(r);
        }
    }
    heuristic_rewrite(question, history)
}

/// B58: returns (answer text, composed) — `composed` is true only when an
/// answer model actually wrote the text. Every other path returns matching
/// context with `false`, so callers can report `mode` truthfully.
async fn generate_answer(settings: &SettingsService, question: &str, context: &str, history: &[AskTurn]) -> (String, bool) {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let is_local = crate::common::provider_is_local(&base);
    if !is_local && key.is_empty() {
        return (format!("Answer for: {}\n\n[No model key configured. Matching context:]\n\n{}", question, context.chars().take(1500).collect::<String>()), false);
    }
    // For R2.2, we do not yet call LLM; return context stub (will be wired in R3.4)
    // Try to call LLM if configured (best-effort, like Python fallback)
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(answer_timeout(settings)).build() {
        Ok(c) => c,
        Err(_) => return (format!("Answer for: {}\n\n{}", question, context.chars().take(1500).collect::<String>()), false),
    };
    let mut headers = reqwest::header::HeaderMap::new();
    if !key.is_empty() {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", key).parse().unwrap());
    }
    let user_content = if history.is_empty() {
        format!("Question: {}\n\nContext:\n{}", question, context)
    } else {
        // R11.7: Python parity — only last 6 turns (like backend/app/answer_engine.py:_generate history[-6:])
        let start = history.len().saturating_sub(6);
        let transcript = history[start..].iter().map(turn_text).collect::<Vec<_>>().join("\n");
        format!("Conversation so far:\n{}\n\nQuestion: {}\n\nContext:\n{}", transcript, question, context)
    };
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are AnchorCore, answer using only context with citations [S1] etc."},
            {"role": "user", "content": user_content}
        ],
        "temperature": 0.2
    });
    match client.post(&url).headers(headers).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(j) = resp.json::<serde_json::Value>().await {
                if let Some(content) = j.pointer("/choices/0/message/content").and_then(|v| v.as_str()) {
                    return (content.to_string(), true);
                }
            }
            (format!("Answer for: {}\n\n{}", question, context.chars().take(1500).collect::<String>()), false)
        }
        _ => (format!("Answer for: {}\n\n[Answer model unreachable; showing context]\n\n{}", question, context.chars().take(1500).collect::<String>()), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Seed a temp DB: one ordinary + one sensitive source, each with an item,
    /// a chunk with distinctive content, and an entity. Returns (data_dir, conn).
    fn seed_gate_db() -> (tempfile::TempDir, Connection, Arc<SettingsService>) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", [])
            .unwrap();
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','secret','sensitive')", [])
            .unwrap();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let sen: i64 = conn.query_row("SELECT id FROM sources WHERE name='secret'", [], |r| r.get(0)).unwrap();
        for (sid, word) in [(ord, "sunshine"), (sen, "moonvault")] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','t','docs/f.md',?2)",
                rusqlite::params![sid, format!("quarterly {} report numbers", word)],
            )
            .unwrap();
            let item: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'f.md',?2)",
                rusqlite::params![item, format!("quarterly {} report numbers", word)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision',?2)",
                rusqlite::params![item, format!("{} entity", word)],
            )
            .unwrap();
        }
        let store = crate::secrets::SecretStore::new_no_keyring(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(dir.path());
        // Hermetic embeddings: closed port refuses instantly, so tests run
        // keyword-only (B58 strict embed_query) instead of stalling on DNS/connect.
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        (dir, conn, settings)
    }

    fn mk_hit(source_id: i64, pii: bool) -> crate::retrieval::Hit {
        crate::retrieval::Hit {
            chunk_id: 1,
            entity_id: None,
            item_id: None,
            source_id: Some(source_id),
            section_id: None,
            path: String::new(),
            score: 1.0,
            content: "x".to_string(),
            source_ref: "f.md".to_string(),
            is_pii: pii,
            status: None,
            content_truncated: false,
            provenance: Vec::new(),
            decay: 1.0,
            boosted: false,
            keyword_matched: false,
            cosine: None,
        }
    }

    #[test]
    fn line_ranges_batch_lookup() {
        // R38.3: one query maps chunk ids to 1-based ranges; unknown ids and
        // untracked (0/0) chunks degrade instead of erroring.
        let (_dir, conn, _settings) = seed_gate_db();
        let item: i64 = conn.query_row("SELECT id FROM ingested_items LIMIT 1", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (item_id, source_ref, content, line_start, line_end) VALUES (?1,'r','a',3,9)", [item]).unwrap();
        let ranged = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'r','b')", [item]).unwrap();
        let plain = conn.last_insert_rowid();
        let map = line_ranges_for_chunks(&conn, &[ranged, plain, 999_999]);
        assert_eq!(map.get(&ranged).copied(), Some((3, 9)));
        assert_eq!(map.get(&plain).copied(), Some((0, 0)));
        assert!(!map.contains_key(&999_999));
        assert!(line_ranges_for_chunks(&conn, &[]).is_empty());
    }

    #[test]
    fn conflicts_for_chunks_warns_doc_chunks_via_item() {
        let (_dir, conn, _settings) = seed_gate_db();
        let a: i64 = conn.query_row("SELECT id FROM entities WHERE summary='sunshine entity'", [], |r| r.get(0)).unwrap();
        let b: i64 = conn.query_row("SELECT id FROM entities WHERE summary='moonvault entity'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'conflicts_with')", rusqlite::params![a, b]).unwrap();
        let sun_doc: i64 = conn.query_row("SELECT id FROM chunks WHERE item_id IS NOT NULL AND content LIKE 'quarterly sunshine%'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','ref','sunshine entity')", [a]).unwrap();
        let sun_ent: i64 = conn.last_insert_rowid();
        let map = conflicts_for_chunks(&conn, &[sun_doc, sun_ent]);
        assert_eq!(map.get(&sun_doc).cloned().unwrap_or_default(), vec![b], "doc chunk warns via its item");
        assert_eq!(map.get(&sun_ent).cloned().unwrap_or_default(), vec![b], "entity chunk warns via its entity");
        // stale own side resolves the warning for the whole item
        conn.execute("UPDATE entities SET status='stale' WHERE id=?1", [a]).unwrap();
        let map_stale = conflicts_for_chunks(&conn, &[sun_doc, sun_ent]);
        assert!(map_stale.get(&sun_doc).cloned().unwrap_or_default().is_empty());
        assert!(conflicts_for_chunks(&conn, &[]).is_empty());
    }

    #[test]
    fn conflicts_resolve_both_directions_and_skip_stale() {
        let (_dir, conn, _settings) = seed_gate_db();
        let a: i64 = conn.query_row("SELECT id FROM entities WHERE summary='sunshine entity'", [], |r| r.get(0)).unwrap();
        let b: i64 = conn.query_row("SELECT id FROM entities WHERE summary='moonvault entity'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind, note) VALUES (?1,?2,'conflicts_with','x vs y')", rusqlite::params![a, b]).unwrap();
        let map = conflicts_for_entities(&conn, &[a]);
        assert_eq!(map.get(&a).cloned().unwrap_or_default(), vec![b]);
        let map_b = conflicts_for_entities(&conn, &[b]);
        assert_eq!(map_b.get(&b).cloned().unwrap_or_default(), vec![a]);
        // stale counterpart = resolved: no warning
        conn.execute("UPDATE entities SET status='stale' WHERE id=?1", [b]).unwrap();
        let map_stale = conflicts_for_entities(&conn, &[a]);
        assert!(map_stale.get(&a).cloned().unwrap_or_default().is_empty());
        assert!(conflicts_for_entities(&conn, &[]).is_empty());
    }

    #[test]
    fn chunk_doc_types_resolve_for_doc_and_entity_chunks() {
        let (_dir, conn, _settings) = seed_gate_db();
        conn.execute("UPDATE ingested_items SET doc_type='code' WHERE text LIKE 'quarterly sunshine%'", []).unwrap();
        let eid: i64 = conn.query_row("SELECT id FROM entities WHERE summary='sunshine entity'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','main.rs','sunshine entity')", [eid]).unwrap();
        let sun_doc: i64 = conn.query_row("SELECT id FROM chunks WHERE item_id IS NOT NULL AND content LIKE 'quarterly sunshine%'", [], |r| r.get(0)).unwrap();
        let sun_ent: i64 = conn.query_row("SELECT id FROM chunks WHERE entity_id=?1", [eid], |r| r.get(0)).unwrap();
        let moon_doc: i64 = conn.query_row("SELECT id FROM chunks WHERE content LIKE 'quarterly moonvault%'", [], |r| r.get(0)).unwrap();
        let map = doc_types_for_chunks(&conn, &[sun_doc, sun_ent, moon_doc]);
        assert_eq!(map.get(&sun_doc).cloned().unwrap_or_default(), "code");
        assert_eq!(map.get(&sun_ent).cloned().unwrap_or_default(), "code");
        assert_eq!(map.get(&moon_doc).cloned().unwrap_or_default(), "");
        assert!(doc_types_for_chunks(&conn, &[]).is_empty());
    }

    #[test]
    fn gate_answer_hits_filters_by_source_label_only() {
        // R35.1: the chunk-`is_pii` half is gone with auto-detect — a flagged
        // chunk on an ordinary source no longer blocks (manual marks gate
        // cloud flows via R35.2).
        let (_dir, conn, _settings) = seed_gate_db();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let sen: i64 = conn.query_row("SELECT id FROM sources WHERE name='secret'", [], |r| r.get(0)).unwrap();
        let (allowed, blocked) = gate_answer_hits(&conn, &[mk_hit(ord, false), mk_hit(sen, false), mk_hit(ord, true)]);
        assert_eq!(allowed.len(), 2);
        assert_eq!(blocked, vec!["sensitive".to_string()]);
    }

    #[tokio::test]
    async fn search_gated_provider_refusal() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        // Default settings: example remote base is untrusted (no ANCHOR_CLOUD_TRUST here).
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert!(res.hits.is_empty(), "sensitive hit must be withheld, got {:?}", res.hits.len());
        assert_eq!(res.refusal.as_deref(), Some("gated_provider"));
        assert!(res.blocked >= 1);
    }

    #[tokio::test]
    async fn search_trusted_provider_returns_hits() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        // Localhost is trusted in every environment — deterministic trust.
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        assert!(is_answer_trusted(&settings));
        let req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert!(!res.hits.is_empty(), "trusted provider must see the sensitive hit");
        assert!(res.refusal.is_none());
        assert_eq!(res.blocked, 0);
        // R39.1: provenance + decay + boosted ride on every search hit.
        for h in &res.hits {
            assert!(!h.provenance.is_empty(), "keyword single-list stamps its leg");
            assert_eq!(h.provenance[0].leg, "keyword");
            assert!((h.decay - 1.0).abs() < 1e-6, "fresh chunk decay must be ~1.0");
            assert!(!h.boosted, "no filename mention — nothing boosted");
        }
    }

    #[tokio::test]
    async fn search_compact_by_default_full_on_opt_in() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let base = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let compact = search(&settings, &embedder, base, &data_dir).await;
        assert!(!compact.hits.is_empty());
        assert!(compact.hits.iter().all(|h| h.content.is_empty()), "default must omit chunk bytes");
        assert!(compact.hits.iter().all(|h| !h.summary.is_empty() && !h.source_ref.is_empty()));
        assert!(compact.hits.iter().all(|h| !h.item_title.is_empty()), "item_title must carry provenance");
        assert!(compact.hits.iter().all(|h| h.path == "docs/f.md"), "flat docs fall back to the item path");
        let full_req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("summary,source_ref,score,content".to_string()), cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let full = search(&settings, &embedder, full_req, &data_dir).await;
        assert!(full.hits.iter().all(|h| !h.content.is_empty()), "fields incl. content must return bytes");
        // agent-friendly alias: include_content=true is fields=content
        let alias_req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: Some(true), env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let alias = search(&settings, &embedder, alias_req, &data_dir).await;
        assert!(alias.hits.iter().all(|h| !h.content.is_empty()), "include_content=true must return bytes");
    }

    #[tokio::test]
    async fn search_cursor_pagination_walks_all_hits() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        // Both seeded chunks share these words, so one page of 1 must leave a next cursor.
        let mk = |cursor: Option<usize>| SearchRequest { query: "quarterly report numbers".to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("all".to_string()), cursor, limit: Some(1), include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let p1 = search(&settings, &embedder, mk(None), &data_dir).await;
        assert_eq!(p1.hits.len(), 1, "page size 1 must return exactly one hit");
        let next = p1.next_cursor.expect("first page must carry a cursor when hits remain");
        let p2 = search(&settings, &embedder, mk(Some(next)), &data_dir).await;
        assert_eq!(p2.hits.len(), 1);
        assert!(p2.next_cursor.is_none(), "second page must exhaust the two seeded hits");
        let mut ids: Vec<Option<i64>> = vec![p1.hits[0].chunk_id, p2.hits[0].chunk_id];
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 2, "two pages must cover two distinct chunks");
    }

    #[tokio::test]
    async fn search_limit_without_k_widens_retrieval() {
        // R40.1: `limit` pages over the `k`-sized candidate list — a bare
        // `limit: 20` must widen retrieval past the default k=8 (1.0.29
        // live report: limit 20/50/100 still returned 7).
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        for n in 0..12 {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?3,'x')",
                rusqlite::params![sid, format!("n{n}.md"), format!("docs/n{n}.md")],
            )
            .unwrap();
            let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title=?1", [format!("n{n}.md")], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'ref',?2)",
                rusqlite::params![iid, format!("quasarion ledger entry number {n} filler words")],
            )
            .unwrap();
        }
        let data_dir = dir.path().to_string_lossy().to_string();
        let store = crate::secrets::SecretStore::new_no_keyring(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(dir.path());
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let mk = |k: Option<usize>, cursor: Option<usize>, limit: Option<usize>| SearchRequest {
            query: "quasarion ledger".to_string(), k, project_id: None, public_only: None,
            fields: None, cursor, limit, include_content: None, env: None, kind: None,
            path_prefix: None, symbol: None, mode: None,
        };
        let res = search(&settings, &embedder, mk(None, None, Some(20)), &data_dir).await;
        assert_eq!(res.hits.len(), 12, "bare limit=20 must see all 12 matches, got {}", res.hits.len());
        assert!(res.next_cursor.is_none());
        // Cursor + limit widen together: page [10..15) of the same 12.
        let page = search(&settings, &embedder, mk(None, Some(10), Some(5)), &data_dir).await;
        assert_eq!(page.hits.len(), 2, "cursor=10 limit=5 over 12 hits must return 2, got {}", page.hits.len());
    }

    #[tokio::test]
    async fn ask_reports_context_only_without_model() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        // Untrusted base + no key: ordinary hit survives the gate, but no
        // model can compose — the response must say so (B58).
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let req = AskRequest { question: "sunshine".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert!(!res.citations.is_empty(), "keyword hit on ordinary source must survive");
        assert_eq!(res.mode, "context_only");
        assert_eq!(res.retrieval, "keyword_only");
        // R23.4: degraded output always warns — UIs banner it, MCP passes it.
        let warning = res.warning.clone().unwrap_or_default();
        assert!(warning.contains("No answer model ran"), "warning: {warning}");
        // R39.1: citations carry the same provenance as search hits.
        assert_eq!(res.citations[0].provenance.len(), 1);
        assert_eq!(res.citations[0].provenance[0].leg, "keyword");
        assert!(!res.citations[0].boosted);
        assert!(res.answer.contains("No model key"), "answer: {}", res.answer.chars().take(120).collect::<String>());
        assert!(res.refusal.is_none());
    }

    #[tokio::test]
    async fn ask_no_match_returns_none_without_citations() {
        // F-07: a question matching nothing must not be answered from the
        // newest documents — no evidence, no citations, mode "none"
        // (search likewise returns no hits).
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        let req = AskRequest { question: "xqzt blorpt wumpus".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.mode, "none");
        assert!(res.citations.is_empty(), "no-match ask must cite nothing");
        assert_eq!(res.retrieval, "keyword_only");
        // R23.4: even an empty answer warns — nothing renders silently.
        let warning = res.warning.clone().unwrap_or_default();
        assert!(warning.contains("No relevant knowledge was found"), "warning: {warning}");
        let sreq = SearchRequest { query: "xqzt blorpt wumpus".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let sres = search(&settings, &embedder, sreq, &data_dir).await;
        assert!(sres.hits.is_empty(), "no-match search must return no hits");
    }

    #[test]
    fn degraded_warning_only_composed_is_silent() {
        // R23.4: the contract in one place — composed stays quiet, every
        // other mode warns in plain language (gated refusal names the gate).
        assert!(degraded_warning("composed", None).is_none());
        let ctx = degraded_warning("context_only", None).expect("context_only must warn");
        assert!(ctx.contains("No answer model ran"), "ctx: {ctx}");
        assert!(ctx.contains("Do not quote it as an answer"), "ctx: {ctx}");
        let none = degraded_warning("none", None).expect("none must warn");
        assert!(none.contains("No relevant knowledge was found"), "none: {none}");
        let gated = degraded_warning("none", Some("gated_provider")).expect("gated refusal must warn");
        assert!(gated.contains("not approved"), "gated: {gated}");
    }

    #[tokio::test]
    async fn ask_gated_refusal_carries_warning() {
        // R23.4: the gate refuses AND warns — agents must not fill the gap.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let req = AskRequest { question: "moonvault".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.mode, "none");
        assert_eq!(res.refusal.as_deref(), Some("gated_provider"));
        assert!(res.citations.is_empty());
        let warning = res.warning.clone().unwrap_or_default();
        assert!(warning.contains("not approved"), "warning: {warning}");
    }

    #[tokio::test]
    async fn ask_composed_carries_no_warning() {
        // R23.4: the one-flow probe target — a working endpoint composes
        // with no warning (hermetic mock OpenAI-compatible server).
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"choices":[{"message":{"content":"mocked answer"}}]}"#;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
        });
        settings.set("answer_base_url", &format!("http://127.0.0.1:{}", port), &conn).unwrap();
        let req = AskRequest { question: "sunshine".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.mode, "composed");
        assert!(res.warning.is_none(), "composed must not warn, got {:?}", res.warning);
        assert_eq!(res.answer, "mocked answer");
        assert!(!res.citations.is_empty());
    }

    #[tokio::test]
    async fn local_mode_expands_unknown_acronyms_once_and_agent_mode_never() {
        // R45.13: opt-in (owner 2026-10-21) — unset makes no call; when on,
        // one budgeted call per new acronym set, cached; known vocabulary
        // skips the call; agent mode and the off switch make none.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = vec![0u8; 16384];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"choices":[{"message":{"content":"{\"QZX\": \"quarterly zebra index\"}"}}]}"#;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
        });
        settings.set("answer_base_url", &base, &conn).unwrap();
        settings.set("answer_model", "expand-test-model", &conn).unwrap();
        let n = || calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(expand_acronyms(&settings, "the QQX trend", &data_dir).await, "the QQX trend");
        assert_eq!(n(), 0, "unset switch makes no call (opt-in)");
        settings.set("acronym_expansion", "true", &conn).unwrap();
        let q = expand_acronyms(&settings, "what is the QZX trend", &data_dir).await;
        assert_eq!(q, "what is the QZX trend quarterly zebra index");
        assert_eq!(n(), 1);
        assert_eq!(expand_acronyms(&settings, "QZX again", &data_dir).await, "QZX again quarterly zebra index");
        assert_eq!(n(), 1, "cached per acronym set");
        crate::vocab::add_manual(&conn, "kyc", "know your customer").unwrap();
        assert_eq!(expand_acronyms(&settings, "our KYC rules", &data_dir).await, "our KYC rules");
        assert_eq!(n(), 1, "known vocabulary skips the call");
        settings.set("llm_mode", "agent", &conn).unwrap();
        assert_eq!(expand_acronyms(&settings, "the WQV metric", &data_dir).await, "the WQV metric");
        settings.set("llm_mode", "local", &conn).unwrap();
        settings.set("acronym_expansion", "false", &conn).unwrap();
        assert_eq!(expand_acronyms(&settings, "the WQV metric", &data_dir).await, "the WQV metric");
        assert_eq!(n(), 1, "agent mode + off switch make no calls");
        task.abort();
    }

    /// R45.2: counting OpenAI-compatible mock — every accepted connection is
    /// one chat call; aborted on drop so the accept loop never wedges teardown.
    struct ChatCounter {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        base: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for ChatCounter {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl ChatCounter {
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    async fn spawn_chat_counter() -> ChatCounter {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = vec![0u8; 16384];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"choices":[{"message":{"content":"mocked answer"}}]}"#;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
        });
        ChatCounter { calls, base, task }
    }

    fn followup_req(question: &str, public_only: Option<bool>) -> AskRequest {
        AskRequest {
            question: question.to_string(),
            history: Some(vec![
                AskTurn { role: "user".to_string(), content: "tell me about the quarterly report".to_string() },
                AskTurn { role: "assistant".to_string(), content: "it covers numbers".to_string() },
            ]),
            project_id: None,
            public_only,
            include_content: None,
        }
    }

    #[tokio::test]
    async fn agent_mode_ask_makes_zero_chat_calls() {
        // R45.2: same reachable local model in both modes — local mode calls
        // it (rewrite + generate), agent mode never does, history or not.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let mock = spawn_chat_counter().await;
        settings.set("answer_base_url", &mock.base, &conn).unwrap();

        // No history here: a local rewrite would turn the question into the
        // mock's canned text and miss retrieval.
        let mut plain = followup_req("sunshine", None);
        plain.history = None;
        let local = ask(&settings, &embedder, plain, &data_dir).await;
        assert_eq!(local.mode, "composed");
        let local_calls = mock.calls();
        assert!(local_calls >= 1, "local mode must reach the mock, got {local_calls}");

        settings.set("llm_mode", "agent", &conn).unwrap();
        let res = ask(&settings, &embedder, followup_req("sunshine numbers for it", None), &data_dir).await;
        assert_eq!(mock.calls(), local_calls, "agent mode must make zero chat calls");
        assert_eq!(res.mode, "agent_compose");
        assert!(res.warning.is_none(), "agent_compose must not warn: {:?}", res.warning);
        assert!(res.refusal.is_none());
        assert_eq!(res.answer, AGENT_COMPOSE_INSTRUCTION);
        assert!(!res.citations.is_empty(), "agent_compose must cite");
        // Content rides along by default (the agent composes from it).
        assert!(res.citations.iter().all(|c| c.content.contains("report numbers")), "citations must carry content");
        assert!(res.citations.iter().any(|c| c.content.contains("sunshine")));
    }

    #[tokio::test]
    async fn agent_mode_public_only_cites_public_sources_only() {
        // R45.2: `/qa/public` (public_only) keeps its scope in agent mode.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','pub','public')", []).unwrap();
        let pubsid: i64 = conn.query_row("SELECT id FROM sources WHERE name='pub'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h2','p','docs/p.md','sunshine public digest')",
            [pubsid],
        ).unwrap();
        let item: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1", [pubsid], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'p.md','sunshine public digest')", [item]).unwrap();
        settings.set("llm_mode", "agent", &conn).unwrap();
        let req = AskRequest { question: "sunshine".to_string(), history: None, project_id: None, public_only: Some(true), include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.mode, "agent_compose");
        assert!(!res.citations.is_empty());
        assert!(res.citations.iter().all(|c| c.content.contains("public")), "only public sources may be cited");
    }

    #[tokio::test]
    async fn agent_mode_keeps_gated_provider_refusal() {
        // R45.2: the label gate still refuses in agent mode (same trust
        // verdict as search — owner decision 7.2.5 may change both together).
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        settings.set("llm_mode", "agent", &conn).unwrap();
        let req = AskRequest { question: "moonvault".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.mode, "none");
        assert_eq!(res.refusal.as_deref(), Some("gated_provider"));
        assert!(res.citations.is_empty());
        assert!(res.warning.unwrap_or_default().contains("not approved"));
    }

    #[tokio::test]
    async fn agent_mode_search_is_unchanged() {
        // R45.2: `/qa/search` is mode-independent — same hits either way.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let mk = || SearchRequest {
            query: "quarterly report".to_string(), k: Some(8), project_id: None, public_only: None,
            fields: Some("content".to_string()), cursor: None, limit: None, include_content: None, env: None,
            kind: None, path_prefix: None, symbol: None, mode: None,
        };
        let ids = |r: &SearchResponse| r.hits.iter().map(|h| (h.chunk_id, h.content.clone())).collect::<Vec<_>>();
        let local = search(&settings, &embedder, mk(), &data_dir).await;
        settings.set("llm_mode", "agent", &conn).unwrap();
        let agent = search(&settings, &embedder, mk(), &data_dir).await;
        assert!(!local.hits.is_empty());
        assert_eq!(ids(&local), ids(&agent));
        assert_eq!(local.blocked, agent.blocked);
        assert_eq!(local.refusal, agent.refusal);
    }

    #[tokio::test]
    async fn answer_ready_follows_llm_mode() {
        let (_dir, conn, settings) = seed_gate_db();
        assert!(!answer_ready(&settings, false), "local mode keeps the probe verdict");
        assert!(answer_ready(&settings, true));
        settings.set("llm_mode", "agent", &conn).unwrap();
        assert!(answer_ready(&settings, false), "agent mode needs no answer model");
    }

    #[tokio::test]
    async fn search_reports_keyword_only_without_embedder() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let req = SearchRequest { query: "sunshine".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert!(!res.hits.is_empty(), "keyword search must find the seeded chunk");
        assert_eq!(res.retrieval, "keyword_only");
    }

    // --- R33.5: exact-string search over /qa/search ---

    fn exact_req(query: &str) -> SearchRequest {
        SearchRequest { query: query.to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("all".to_string()), cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: Some("exact".to_string()) }
    }

    #[test]
    fn exact_mode_selector_is_case_insensitive_and_defaults_ranked() {
        assert!(!is_exact_mode(&None));
        assert!(is_exact_mode(&Some("exact".to_string())));
        assert!(is_exact_mode(&Some("  EXACT ".to_string())));
        assert!(!is_exact_mode(&Some("fuzzy".to_string())), "unknown modes stay ranked, like unknown env/kind unfilter");
        assert!(!is_exact_mode(&Some(String::new())));
    }

    #[tokio::test]
    async fn search_exact_mode_returns_unranked_verbatim_hits() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let res = search(&settings, &embedder, exact_req("moonvault"), &data_dir).await;
        assert_eq!(res.hits.len(), 1, "only the sensitive chunk holds the needle");
        assert_eq!(res.retrieval, "exact");
        let h = &res.hits[0];
        assert_eq!(h.score, 1.0, "unranked uniform score");
        assert_eq!(h.content, "quarterly moonvault report numbers", "verbatim stored bytes");
        assert!(h.summary.contains("moonvault"), "summary centers on the match: {}", h.summary);
        assert_eq!(h.path, "docs/f.md", "flat docs fall back to the item path");
        assert!(h.item_id.is_some() && h.chunk_id.is_some(), "exact hits drill down like ranked ones");
        assert!(res.refusal.is_none() && res.blocked == 0);
        // Case-sensitive at the API boundary too.
        let miss = search(&settings, &embedder, exact_req("Moonvault"), &data_dir).await;
        assert!(miss.hits.is_empty(), "Moonvault (capital M) is absent verbatim");
        assert_eq!(miss.retrieval, "exact");
    }

    #[tokio::test]
    async fn search_exact_mode_gated_provider_refusal() {
        // Gating parity with the ranked leg: same gate, same tail.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let res = search(&settings, &embedder, exact_req("moonvault"), &data_dir).await;
        assert!(res.hits.is_empty(), "sensitive hit must be withheld, got {:?}", res.hits.len());
        assert_eq!(res.refusal.as_deref(), Some("gated_provider"));
        assert!(res.blocked >= 1);
        assert_eq!(res.retrieval, "exact");
        // The ordinary-source needle still reads through the same gate.
        let ok = search(&settings, &embedder, exact_req("sunshine"), &data_dir).await;
        assert_eq!(ok.hits.len(), 1);
        assert!(ok.refusal.is_none());
    }

    #[tokio::test]
    async fn search_exact_mode_scope_parity() {
        // Project scope + public_only cut the exact leg like the ranked one.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO projects (name) VALUES ('p1')", []).unwrap();
        let pid: i64 = conn.last_insert_rowid();
        conn.execute("INSERT INTO project_sources (project_id, source_id) VALUES (?1,?2)", rusqlite::params![pid, ord]).unwrap();
        // Both seeded chunks hold "quarterly"; the project holds one source.
        let mut scoped = exact_req("quarterly");
        scoped.project_id = Some(pid);
        let res = search(&settings, &embedder, scoped, &data_dir).await;
        assert_eq!(res.hits.len(), 1);
        assert!(res.hits[0].content.contains("sunshine"));
        // No public sources seeded: public_only empties without a refusal
        // (empty scope, not a gate — same as ranked).
        let mut pub_only = exact_req("quarterly");
        pub_only.public_only = Some(true);
        let pub_res = search(&settings, &embedder, pub_only, &data_dir).await;
        assert!(pub_res.hits.is_empty());
        assert!(pub_res.refusal.is_none());
    }

    #[tokio::test]
    async fn search_exact_mode_unknown_mode_stays_ranked() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let mut req = exact_req("sunshine");
        req.mode = Some("fuzzy".to_string());
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.retrieval, "keyword_only", "unknown mode falls back to the ranked leg");
        assert!(!res.hits.is_empty());
    }

    #[tokio::test]
    async fn search_exact_mode_cap_and_pagination() {
        // Both seeded chunks hold "quarterly": k caps, cursor walks.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let mut capped = exact_req("quarterly");
        capped.k = Some(1);
        let cap = search(&settings, &embedder, capped, &data_dir).await;
        assert_eq!(cap.hits.len(), 1, "k caps the exact leg like the ranked one");
        assert!(cap.next_cursor.is_none(), "a capped fetch has no second page");
        // Fetch-wide, then walk one per page.
        let mut p1req = exact_req("quarterly");
        p1req.limit = Some(1);
        let p1 = search(&settings, &embedder, p1req, &data_dir).await;
        assert_eq!(p1.hits.len(), 1);
        let next = p1.next_cursor.expect("one more exact hit remains");
        let mut page2 = exact_req("quarterly");
        page2.cursor = Some(next);
        page2.limit = Some(1);
        let p2 = search(&settings, &embedder, page2, &data_dir).await;
        assert_eq!(p2.hits.len(), 1);
        assert!(p2.next_cursor.is_none(), "second page exhausts the two exact hits");
        assert_ne!(p1.hits[0].chunk_id, p2.hits[0].chunk_id);
    }

    #[tokio::test]
    async fn generate_answer_falls_back_without_key() {
        let (_dir, _conn, settings) = seed_gate_db();
        let (text, composed) = generate_answer(&settings, "q", "ctx", &[]).await;
        assert!(!composed);
        assert!(text.contains("No model key"), "text: {}", text.chars().take(120).collect::<String>());
    }

    #[tokio::test]
    async fn generate_answer_composed_true_with_mock_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, conn, settings) = seed_gate_db();
        // Minimal hermetic OpenAI-compatible server: one canned completion.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"choices":[{"message":{"content":"mocked answer"}}]}"#;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
        });
        settings.set("answer_base_url", &format!("http://127.0.0.1:{}", port), &conn).unwrap();
        let (text, composed) = generate_answer(&settings, "q", "ctx", &[]).await;
        assert!(composed, "mock model must yield composed=true, got: {}", text.chars().take(120).collect::<String>());
        assert_eq!(text, "mocked answer");
    }

    #[test]
    fn answer_timeout_default_clamp_and_fractional() {
        // R43.7: the `answer_timeout` setting (seconds) defaults to 120,
        // honors fractional values, and falls back on bogus input.
        let saved = std::env::var("ANCHOR_ANSWER_TIMEOUT").ok();
        std::env::remove_var("ANCHOR_ANSWER_TIMEOUT");
        let (_dir, conn, settings) = seed_gate_db();
        assert_eq!(answer_timeout(&settings), std::time::Duration::from_secs(120));
        settings.set("answer_timeout", "0.1", &conn).unwrap();
        assert_eq!(answer_timeout(&settings), std::time::Duration::from_millis(100));
        settings.set("answer_timeout", "bogus", &conn).unwrap();
        assert_eq!(answer_timeout(&settings), std::time::Duration::from_secs(120));
        settings.set("answer_timeout", "0", &conn).unwrap();
        assert_eq!(answer_timeout(&settings), std::time::Duration::from_secs(120));
        settings.set("answer_timeout", "99999", &conn).unwrap();
        assert_eq!(answer_timeout(&settings), std::time::Duration::from_secs(3600));
        if let Some(v) = saved {
            std::env::set_var("ANCHOR_ANSWER_TIMEOUT", v);
        }
    }

    #[tokio::test]
    async fn generate_answer_timeout_falls_back_gracefully() {
        // R43.7: a model slower than `answer_timeout` must degrade to the
        // unreachable-model fallback (composed=false), not hang or error.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, conn, settings) = seed_gate_db();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                // Far slower than the 100ms client timeout below.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let body = r#"{"choices":[{"message":{"content":"too slow"}}]}"#;
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
        });
        settings.set("answer_base_url", &format!("http://127.0.0.1:{}", port), &conn).unwrap();
        settings.set("answer_timeout", "0.1", &conn).unwrap();
        let (text, composed) = generate_answer(&settings, "q", "ctx", &[]).await;
        assert!(!composed, "slow model must not compose, got: {}", text.chars().take(120).collect::<String>());
        assert!(text.contains("Answer model unreachable"), "text: {}", text.chars().take(160).collect::<String>());
    }

    #[test]
    fn build_answer_context_drops_match_marks() {
        let mut h = mk_hit(1, false);
        h.content = "intro\n[match 7]\nOwner: Sarah.\n[/match 7]\ntail".to_string();
        let ctx = build_answer_context(&[h]);
        assert_eq!(ctx, "[S1] intro\nOwner: Sarah.\ntail");
        assert!(!ctx.contains("[match"), "marks leaked into the answer context: {}", ctx);
    }

    #[test]
    fn build_answer_context_enforces_token_budget() {
        // R43.7: short hits assemble verbatim in rank order; past the
        // 6000-token (24000-char) budget, trailing sections drop while
        // numbering stays contiguous from S1.
        let mut short = mk_hit(1, false);
        short.content = "alpha".to_string();
        let mut short2 = mk_hit(1, false);
        short2.content = "beta".to_string();
        assert_eq!(build_answer_context(&[short, short2]), "[S1] alpha\n\n[S2] beta");
        assert!(build_answer_context(&[]).is_empty());
        let long: Vec<crate::retrieval::Hit> = (0..15)
            .map(|_| {
                let mut h = mk_hit(1, false);
                h.content = "w".repeat(5000);
                h
            })
            .collect();
        let ctx = build_answer_context(&long);
        let budget_chars = ANSWER_CONTEXT_TOKEN_BUDGET * CHARS_PER_TOKEN_ESTIMATE;
        assert!(ctx.chars().count() <= budget_chars, "len {}", ctx.chars().count());
        assert!(ctx.starts_with("[S1] www"), "head: {}", ctx.chars().take(12).collect::<String>());
        assert!(ctx.contains("[S11]"), "must keep ~11 sections, got {} chars", ctx.chars().count());
        assert!(!ctx.contains("[S12]"), "trailing sections must drop past the budget");
    }

    #[test]
    fn build_citations_content_opt_in_and_snippet_floor() {
        let (dir, _conn, _settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut hit = mk_hit(1, false);
        hit.content = "lorem ipsum dolor ".repeat(60); // ~1000 chars
        let compact = build_citations(std::slice::from_ref(&hit), &data_dir, false, 5, "");
        assert_eq!(compact.len(), 1);
        assert!(compact[0].content.is_empty(), "content stays empty unless requested");
        assert_eq!(compact[0].snippet.chars().count(), 500, "snippet floor matches search summaries");
        assert_eq!(compact[0].item_title, "t");
        assert_eq!(compact[0].path, "docs/f.md", "pathless chunks fall back to the item path");
        let full = build_citations(std::slice::from_ref(&hit), &data_dir, true, 5, "");
        assert_eq!(full[0].content, hit.content, "opt-in returns verbatim bytes");
    }

    #[test]
    fn build_citations_centers_snippet_on_match() {
        // A head-only preview of a long hit hides the match (1.0.24 retest) —
        // summary + snippet must window on the query terms instead.
        let (dir, _conn, _settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let mut hit = mk_hit(1, false);
        hit.content = format!("{}moonvault{}", "ab ".repeat(400), " cd".repeat(400));
        let cites = build_citations(std::slice::from_ref(&hit), &data_dir, false, 5, "moonvault");
        assert_eq!(cites.len(), 1);
        assert!(cites[0].snippet.contains("moonvault"), "snippet: {}", cites[0].snippet.chars().take(60).collect::<String>());
        assert!(cites[0].summary.contains("moonvault"), "summary: {}", cites[0].summary.chars().take(60).collect::<String>());
    }

    #[test]
    fn build_citations_respects_limit() {
        let (dir, _conn, _settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let hits = vec![mk_hit(1, false), mk_hit(1, false), mk_hit(1, false)];
        assert_eq!(build_citations(&hits, &data_dir, false, 2, "").len(), 2);
        assert_eq!(build_citations(&hits, &data_dir, false, 9, "").len(), 3);
    }

    #[test]
    fn citations_limit_defaults_to_five() {
        // No other test touches ANCHOR_CITATIONS, so set/remove is safe.
        std::env::remove_var("ANCHOR_CITATIONS");
        assert_eq!(citations_limit(), 5);
        std::env::set_var("ANCHOR_CITATIONS", "2");
        assert_eq!(citations_limit(), 2);
        std::env::set_var("ANCHOR_CITATIONS", "bogus");
        assert_eq!(citations_limit(), 5);
        std::env::set_var("ANCHOR_CITATIONS", "0");
        assert_eq!(citations_limit(), 5);
        std::env::remove_var("ANCHOR_CITATIONS");
    }

    #[tokio::test]
    async fn ask_passes_include_content_through() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let req = AskRequest { question: "sunshine".to_string(), history: None, project_id: None, public_only: None, include_content: Some(true) };
        let res = ask(&settings, &embedder, req, &data_dir).await;
        assert!(!res.citations.is_empty());
        assert!(res.citations.iter().all(|c| !c.content.is_empty()), "opt-in ask must carry chunk bytes");
        let req2 = AskRequest { question: "sunshine".to_string(), history: None, project_id: None, public_only: None, include_content: None };
        let res2 = ask(&settings, &embedder, req2, &data_dir).await;
        assert!(res2.citations.iter().all(|c| c.content.is_empty()), "default ask stays compact");
    }

    #[test]
    fn normalize_env_filter_values() {
        assert_eq!(normalize_env_filter(None), None);
        assert_eq!(normalize_env_filter(Some("live".to_string())), Some("live".to_string()));
        assert_eq!(normalize_env_filter(Some(" TEST ".to_string())), Some("test".to_string()));
        assert!(normalize_env_filter(Some("prod".to_string())).is_none(), "unknown classes mean unfiltered");
        assert!(normalize_env_filter(Some(String::new())).is_none());
    }

    #[test]
    fn build_citations_carries_item_id_and_env() {
        let (dir, conn, _settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let (cid, iid): (i64, i64) = conn
            .query_row("SELECT id, item_id FROM chunks WHERE content LIKE 'quarterly sunshine%'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        let mut hit = mk_hit(1, false);
        hit.chunk_id = cid;
        hit.item_id = Some(iid);
        hit.content = "quarterly sunshine report numbers".to_string();
        let cites = build_citations(std::slice::from_ref(&hit), &data_dir, false, 5, "");
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].item_id, Some(iid), "citation must drill into GET /items/:id");
        assert_eq!(cites[0].env, "live", "docs/f.md carries no test marker");
    }

    #[tokio::test]
    async fn search_env_filter_orders_and_cuts_by_class() {
        // R33.3 end to end (was R32.2–R32.4 with a test entity row): a live
        // doc chunk plus a test doc chunk sharing the query terms. Plain
        // search orders live first; env=live/test cuts to one class; every
        // hit carries an item_id and an env class. A test entity row sits
        // in the DB sharing the terms and must never surface.
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','routing-config.yml','cfg/routing-config.yml','live treasury routes acme config content')",
            [ord],
        )
        .unwrap();
        let live_iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE external_id='cfg/routing-config.yml'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'routing-config.yml','live treasury routes acme config content')",
            [live_iid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','payment-routes.test.yml','cfg/payment-routes.test.yml','test treasury routes content')",
            [ord],
        )
        .unwrap();
        let test_iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE external_id='cfg/payment-routes.test.yml'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'payment-routes.test.yml','test treasury routes content')",
            [test_iid],
        )
        .unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','test treasury routes content')", [test_iid]).unwrap();
        let eid: i64 = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','payment-routes.test.yml','test treasury routes content')", [eid]).unwrap();

        let mk = |env: Option<&str>| SearchRequest {
            query: "treasury routes".to_string(),
            k: Some(8),
            project_id: None,
            public_only: None,
            fields: None,
            cursor: None,
            limit: None,
            include_content: None,
            env: env.map(|s| s.to_string()),
            kind: None,
            path_prefix: None,
            symbol: None,
            mode: None,
        };
        let plain = search(&settings, &embedder, mk(None), &data_dir).await;
        assert_eq!(plain.hits.len(), 2, "both classes match the shared terms; the entity row stays out");
        assert!(plain.hits.iter().all(|h| h.item_id.is_some()), "every hit drills into its item");
        assert!(plain.hits.iter().all(|h| h.entity_id.is_none()), "no entity rows in retrieval");
        assert_eq!(plain.hits[0].env, "live", "live-first must surface the live chunk first");
        assert_eq!(plain.hits[0].item_id, Some(live_iid));
        assert_eq!(plain.hits[1].env, "test");
        assert_eq!(plain.hits[1].item_id, Some(test_iid));

        let live_only = search(&settings, &embedder, mk(Some("live")), &data_dir).await;
        assert_eq!(live_only.hits.len(), 1);
        assert_eq!(live_only.hits[0].item_id, Some(live_iid));
        assert_eq!(live_only.hits[0].env, "live");

        let test_only = search(&settings, &embedder, mk(Some("TEST")), &data_dir).await;
        assert_eq!(test_only.hits.len(), 1);
        assert_eq!(test_only.hits[0].item_id, Some(test_iid));
        assert_eq!(test_only.hits[0].env, "test");
    }

    #[test]
    fn normalize_r36_filters() {
        assert!(normalize_kind_filter(None).is_none());
        assert!(normalize_kind_filter(Some(String::new())).is_none());
        assert!(normalize_kind_filter(Some("banana".to_string())).is_none(), "unknown kinds mean unfiltered, like env");
        assert_eq!(normalize_kind_filter(Some("document".to_string())), Some(vec!["document".to_string()]));
        assert_eq!(
            normalize_kind_filter(Some(" Entity , DOC_SUMMARY, banana ".to_string())),
            Some(vec!["entity".to_string(), "doc_summary".to_string()]),
            "case/space tolerant, unknown tokens dropped"
        );
        assert!(normalize_prefix_filter(None).is_none());
        assert!(normalize_prefix_filter(Some("  ".to_string())).is_none());
        assert_eq!(normalize_prefix_filter(Some("cfg/".to_string())), Some("cfg/".to_string()));
        assert!(normalize_symbol_filter(None).is_none());
        assert!(normalize_symbol_filter(Some(String::new())).is_none());
        assert_eq!(normalize_symbol_filter(Some("  Fetch_User ".to_string())), Some("fetch_user".to_string()));
        let req = SearchRequest {
            query: "q".to_string(),
            k: None,
            project_id: None,
            public_only: None,
            fields: None,
            cursor: None,
            limit: None,
            include_content: None,
            env: Some("live".to_string()),
            kind: None,
            path_prefix: None,
            symbol: None,
            mode: None,
        };
        assert!(SearchFilters::from_request(&req).any());
        let req_none = SearchRequest {
            query: "q".to_string(),
            k: None,
            project_id: None,
            public_only: None,
            fields: None,
            cursor: None,
            limit: None,
            include_content: None,
            env: None,
            kind: Some("banana".to_string()),
            path_prefix: None,
            symbol: None,
            mode: None,
        };
        assert!(!SearchFilters::from_request(&req_none).any(), "unknown-only kind normalizes away");
    }

    #[test]
    fn batched_filter_lookups_handle_empty() {
        let (_dir, conn, _settings) = seed_gate_db();
        assert!(kinds_for_chunks(&conn, &[]).is_empty());
        assert!(item_paths_for_chunks(&conn, &[]).is_empty());
        assert!(section_titles_for_chunks(&conn, &[]).is_empty());
        assert!(section_breadcrumbs(&conn, &[]).is_empty());
    }

    /// Six chunks sharing the query terms `acme ledger`: doc chunks on
    /// `cfg/a.yml` + `docs/b.md`, an entity chunk on the cfg item (seeded
    /// but never retrievable since R33.3 — the exclusion proof), and three
    /// chunks on `src/code.py` (symbol section, other section, flat).
    fn seed_filter_db() -> (tempfile::TempDir, Connection, Arc<SettingsService>, String) {
        let dir = tempdir().unwrap();
        // The corpus dir only lends its path string for root matching (the
        // folder root is config text — existence is never checked), so it can
        // drop right away.
        let root = tempdir().unwrap().path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        let cfg = serde_json::json!({"path": root}).to_string();
        conn.execute("INSERT INTO sources (connector, name, config) VALUES ('folder','ordinary',?1)", [cfg]).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let mk_item = |ext: &str| {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?2,'acme ledger body')",
                rusqlite::params![sid, ext],
            )
            .unwrap();
            conn.query_row("SELECT id FROM ingested_items WHERE external_id=?1", [ext], |r| r.get::<_, i64>(0)).unwrap()
        };
        let cfg_iid = mk_item("cfg/a.yml");
        let _docs_iid = mk_item("docs/b.md");
        let code_iid = mk_item("src/code.py");
        let mk_chunk = |iid: i64, content: &str| {
            conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'ref',?2)", rusqlite::params![iid, content]).unwrap();
        };
        mk_chunk(cfg_iid, "acme ledger alpha config");
        // docs item chunk
        let docs_iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE external_id='docs/b.md'", [], |r| r.get(0)).unwrap();
        mk_chunk(docs_iid, "acme ledger beta docs");
        // entity chunk on the cfg item
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','acme ledger gamma entity')", [cfg_iid]).unwrap();
        let eid: i64 = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','ref','acme ledger gamma entity')", [eid]).unwrap();
        // code item: symbol section, other section, flat chunk
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,0,'def fetch_user','fetch_user')", [code_iid]).unwrap();
        let sym = conn.last_insert_rowid();
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,0,'Other','Other')", [code_iid]).unwrap();
        let other = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, path, source_ref, content) VALUES (?1,?2,'fetch_user','ref','acme ledger delta symbol')",
            rusqlite::params![code_iid, sym],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, path, source_ref, content) VALUES (?1,?2,'Other','ref','acme ledger epsilon misc')",
            rusqlite::params![code_iid, other],
        )
        .unwrap();
        mk_chunk(code_iid, "acme ledger zeta flat");
        let store = crate::secrets::SecretStore::new_no_keyring(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(dir.path());
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        (dir, conn, settings, root)
    }

    fn filter_req(query: &str, kind: Option<&str>, path_prefix: Option<&str>, symbol: Option<&str>) -> SearchRequest {
        SearchRequest {
            query: query.to_string(),
            k: Some(8),
            project_id: None,
            public_only: None,
            fields: Some("all".to_string()),
            cursor: None,
            limit: None,
            include_content: None,
            env: None,
            kind: kind.map(|s| s.to_string()),
            path_prefix: path_prefix.map(|s| s.to_string()),
            symbol: symbol.map(|s| s.to_string()),
            mode: None,
        }
    }

    #[tokio::test]
    async fn search_kind_filter_cuts_by_chunk_kind() {
        let (dir, conn, settings, _root) = seed_filter_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let plain = search(&settings, &embedder, filter_req("acme ledger", None, None, None), &data_dir).await;
        assert_eq!(plain.hits.len(), 5, "five document chunks match; the seeded entity chunk stays out");
        let docs = search(&settings, &embedder, filter_req("acme ledger", Some("document"), None, None), &data_dir).await;
        assert_eq!(docs.hits.len(), 5);
        assert!(docs.hits.iter().all(|h| h.entity_id.is_none()));
        let ents = search(&settings, &embedder, filter_req("acme ledger", Some("ENTITY"), None, None), &data_dir).await;
        assert!(ents.hits.is_empty(), "R33.3: entities never enter retrieval, so kind=entity matches nothing");
        let both = search(&settings, &embedder, filter_req("acme ledger", Some("document,entity"), None, None), &data_dir).await;
        assert_eq!(both.hits.len(), 5, "comma list unions over the document pool");
        let bogus = search(&settings, &embedder, filter_req("acme ledger", Some("banana"), None, None), &data_dir).await;
        assert_eq!(bogus.hits.len(), 5, "unknown kind means unfiltered, like env");
        let summaries = search(&settings, &embedder, filter_req("acme ledger", Some("section_summary"), None, None), &data_dir).await;
        assert!(summaries.hits.is_empty(), "known-but-absent kind matches nothing");
    }

    #[tokio::test]
    async fn search_path_prefix_filter_cuts_by_item_dir() {
        let (dir, conn, settings, root) = seed_filter_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        // Relative: the cfg item's doc chunk only (its entity chunk stays
        // out of retrieval since R33.3).
        let rel = search(&settings, &embedder, filter_req("acme ledger", None, Some("cfg/"), None), &data_dir).await;
        assert_eq!(rel.hits.len(), 1);
        assert!(rel.hits.iter().all(|h| h.item_title == "cfg/a.yml"));
        // Absolute through the folder root spells the same cut.
        let abs = search(&settings, &embedder, filter_req("acme ledger", None, Some(&format!("{root}/cfg")), None), &data_dir).await;
        assert_eq!(abs.hits.len(), 1);
        assert!(abs.hits.iter().all(|h| h.item_title == "cfg/a.yml"));
        let none = search(&settings, &embedder, filter_req("acme ledger", None, Some("/no/such/root"), None), &data_dir).await;
        assert!(none.hits.is_empty());
    }

    #[tokio::test]
    async fn search_symbol_filter_matches_section_title() {
        let (dir, conn, settings, _root) = seed_filter_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let sym = search(&settings, &embedder, filter_req("acme ledger", None, None, Some("fetch_user")), &data_dir).await;
        assert_eq!(sym.hits.len(), 1);
        assert_eq!(sym.hits[0].item_title, "src/code.py");
        assert!(sym.hits[0].path.contains("fetch_user"), "hit path carries the section: {}", sym.hits[0].path);
        let upper = search(&settings, &embedder, filter_req("acme ledger", None, None, Some("FETCH_USER")), &data_dir).await;
        assert_eq!(upper.hits.len(), 1, "symbol match is case-insensitive");
        let miss = search(&settings, &embedder, filter_req("acme ledger", None, None, Some("no_such_symbol")), &data_dir).await;
        assert!(miss.hits.is_empty(), "section-less chunks drop out of a symbol cut");
    }

    #[tokio::test]
    async fn search_filters_combine_with_and() {
        let (dir, conn, settings, root) = seed_filter_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        // doc ∩ symbol: only the symbol-section chunk survives.
        let both = search(&settings, &embedder, filter_req("acme ledger", Some("document"), None, Some("fetch_user")), &data_dir).await;
        assert_eq!(both.hits.len(), 1);
        assert!(both.hits[0].entity_id.is_none());
        // entity ∩ symbol: entities never enter retrieval — empty intersection.
        let empty = search(&settings, &embedder, filter_req("acme ledger", Some("entity"), None, Some("fetch_user")), &data_dir).await;
        assert!(empty.hits.is_empty());
        // kind ∩ path: kind=entity matches nothing anywhere since R33.3.
        let cfg_ent = search(&settings, &embedder, filter_req("acme ledger", Some("entity"), Some("cfg/"), None), &data_dir).await;
        assert!(cfg_ent.hits.is_empty());
        // Absolute prefix ∩ symbol: the symbol chunk lives under src/.
        let abs_sym = search(
            &settings,
            &embedder,
            filter_req("acme ledger", None, Some(&format!("{root}/src")), Some("fetch_user")),
            &data_dir,
        )
        .await;
        assert_eq!(abs_sym.hits.len(), 1);
        // Scores survive filtering untouched (score-neutral cuts).
        let plain = search(&settings, &embedder, filter_req("acme ledger", None, None, None), &data_dir).await;
        let plain_score = plain.hits.iter().find(|h| h.summary.contains("delta symbol")).map(|h| h.score);
        assert_eq!(both.hits[0].score, plain_score.unwrap_or(-1.0), "a cut must not rescore");
    }

    #[tokio::test]
    async fn search_filters_survive_parent_return() {
        // All four R36.6 cuts AND-combined on parent-returned hits: filters
        // read matched-chunk identity from the DB, so grouping never
        // confuses them.
        let (dir, conn, settings, root) = seed_filter_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let req = |env: Option<&str>, kind: Option<&str>, prefix: Option<&str>, symbol: Option<&str>| SearchRequest {
            query: "acme ledger".to_string(),
            k: Some(8),
            project_id: None,
            public_only: None,
            fields: Some("all".to_string()),
            cursor: None,
            limit: None,
            include_content: None,
            env: env.map(|s| s.to_string()),
            kind: kind.map(|s| s.to_string()),
            path_prefix: prefix.map(|s| s.to_string()),
            symbol: symbol.map(|s| s.to_string()),
            mode: None,
        };
        // Four-way: only the live doc chunk under src/ in the symbol section.
        let four = search(&settings, &embedder, req(Some("live"), Some("document"), Some("src/"), Some("fetch_user")), &data_dir).await;
        assert_eq!(four.hits.len(), 1);
        assert_eq!(four.hits[0].item_title, "src/code.py");
        assert!(four.hits[0].content.contains("[match "), "surviving hit is parent-shaped");
        // Same combo with an absolute prefix spells the same cut.
        let abs = search(&settings, &embedder, req(Some("live"), Some("document"), Some(&format!("{root}/src")), Some("fetch_user")), &data_dir).await;
        assert_eq!(abs.hits.len(), 1);
        // env=test empties the all-live seed under any combination.
        let tst = search(&settings, &embedder, req(Some("test"), Some("document"), None, None), &data_dir).await;
        assert!(tst.hits.is_empty());
        // kind ∩ env: kind=entity matches nothing since R33.3.
        let ent = search(&settings, &embedder, req(Some("live"), Some("entity"), None, None), &data_dir).await;
        assert!(ent.hits.is_empty());
    }

    /// Parent-return seed: one `docs/billing.md` item with a two-section
    /// tree sharing the query terms `tungsten invoice` — `Billing` holds
    /// two matching chunks (merge partners), `Auth` holds one.
    fn seed_parent_db() -> (tempfile::TempDir, Connection, Arc<SettingsService>) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','billing.md','docs/billing.md','tungsten invoice body')",
            [sid],
        )
        .unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE external_id='docs/billing.md'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Billing','Billing')", [iid]).unwrap();
        let bill = conn.last_insert_rowid();
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Auth','Auth')", [iid]).unwrap();
        let auth = conn.last_insert_rowid();
        for (sec, path, content) in [
            (bill, "Billing", "tungsten invoice first billing lines"),
            (bill, "Billing", "tungsten invoice second billing lines"),
            (auth, "Auth", "tungsten invoice auth lines"),
        ] {
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, path, source_ref, content) VALUES (?1,?2,?3,'billing.md',?4)",
                rusqlite::params![iid, sec, path, content],
            )
            .unwrap();
        }
        let store = crate::secrets::SecretStore::new_no_keyring(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(dir.path());
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        (dir, conn, settings)
    }

    #[tokio::test]
    async fn search_groups_matches_by_parent_section() {
        // End to end: three chunk matches become two parent hits, and every
        // hit carries the full R36.3 field set.
        let (dir, conn, settings) = seed_parent_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let req = SearchRequest { query: "tungsten invoice".to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("all".to_string()), cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None, symbol: None, mode: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert_eq!(res.hits.len(), 2, "two Billing matches merge into one parent");
        let bill = res.hits.iter().find(|h| h.breadcrumb == "Billing").expect("billing parent");
        assert!(bill.chunk_id.is_some() && bill.item_id.is_some() && bill.section_id.is_some(), "ids carried");
        assert_eq!(bill.path, "Billing");
        assert_eq!(bill.item_title, "billing.md");
        assert_eq!(bill.env, "live");
        assert!(!bill.content_truncated, "whole section returned");
        assert_eq!(bill.content.matches("[match ").count(), 2, "every match marked, got:\n{}", bill.content);
        assert!(bill.summary.contains("[match "), "summary windows on the mark: {}", bill.summary);
        let auth = res.hits.iter().find(|h| h.breadcrumb == "Auth").expect("auth parent");
        assert_eq!(auth.content.matches("[match ").count(), 1);
    }

    fn seed_floor_db() -> (tempfile::TempDir, Connection) {
        // Item A matches at summary AND leaf; item B matches ONLY mid-body
        // (its head-extract summary sees nothing) — the ai-03 shape.
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','floor')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='floor'", [], |r| r.get(0)).unwrap();
        for (title, ext, sum, body) in [
            ("alpha.md", "docs/alpha.md", "alpha alpha project kickoff", "alpha first lines here"),
            ("beta.md", "docs/beta.md", "nothing relevant here at all", "buried alpha mention mid-body lines"),
        ] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?3,'x')",
                rusqlite::params![sid, title, ext],
            )
            .unwrap();
            let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title=?1", [title], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'S','S')", [iid]).unwrap();
            let sec = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, kind, source_ref, content) VALUES (?1,?2,'section_summary',?3,?4)",
                rusqlite::params![iid, sec, ext, sum],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,?3,?4)",
                rusqlite::params![iid, sec, ext, body],
            )
            .unwrap();
        }
        (dir, conn)
    }

    #[test]
    fn orchestrate_flat_floor_rescues_mid_body_match() {
        // Differential: TOC fencing alone excludes B (its summary matches
        // nothing); the flat recall floor brings B back without losing A.
        let (_dir, conn) = seed_floor_db();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        let toc = retrieval::toc_search(&conn, "alpha", None, None, 8, true);
        assert!(!toc.is_empty(), "fence active: A matches at summary stage");
        assert!(toc.iter().all(|h| h.item_id != Some(beta_id(&conn))), "TOC alone misses B: {:?}", toc.iter().map(|h| h.item_id).collect::<Vec<_>>());
        let hits = orchestrate(&conn, "alpha", None, None, &cfg);
        let items: Vec<Option<i64>> = hits.iter().map(|h| h.item_id).collect();
        assert!(items.contains(&Some(alpha_id(&conn))), "A kept: {items:?}");
        assert!(items.contains(&Some(beta_id(&conn))), "floor rescues B: {items:?}");
    }

    fn alpha_id(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM ingested_items WHERE title='alpha.md'", [], |r| r.get(0)).unwrap()
    }

    fn beta_id(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM ingested_items WHERE title='beta.md'", [], |r| r.get(0)).unwrap()
    }

    fn seed_slots_db() -> (tempfile::TempDir, Connection) {
        // 8 live single-leaf docs + 1 fixture doc (10 TOC leaves) + B,
        // fenced out of TOC but the floor keyword rank-1. B is deliberately
        // fixture-pathed: a live B would lock out too (9th live at the cut),
        // but the final list would be all-live and the live-first assert
        // vacuous — a test-pathed B proves the slots coexist with live-first
        // instead of punching through it.
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let docs = [
            ("a1.md", "docs/a1.md", 1),
            ("a2.md", "docs/a2.md", 1),
            ("a3.md", "docs/a3.md", 1),
            ("a4.md", "docs/a4.md", 1),
            ("a5.md", "docs/a5.md", 1),
            ("a6.md", "docs/a6.md", 1),
            ("a7.md", "docs/a7.md", 1),
            ("a8.md", "docs/a8.md", 1),
            ("t1.md", "tests/t1.md", 2),
            ("b.md", "tests/b.md", 1),
        ];
        for (title, ext, leaves) in docs {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?3,'x')",
                rusqlite::params![sid, title, ext],
            )
            .unwrap();
            let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title=?1", [title], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'S','S')", [iid]).unwrap();
            let sec = conn.last_insert_rowid();
            let summary = if title == "b.md" {
                "nothing relevant here at all".to_string()
            } else {
                format!("alpha project notes {title}")
            };
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, kind, source_ref, content) VALUES (?1,?2,'section_summary',?3,?4)",
                rusqlite::params![iid, sec, ext, summary],
            )
            .unwrap();
            for k in 0..leaves {
                let body = if title == "b.md" {
                    "zephyr zephyr zephyr special mid-body signal".to_string()
                } else {
                    format!("alpha body {title} leaf {k} filler words padding text")
                };
                conn.execute(
                    "INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![iid, sec, ext, body],
                )
                .unwrap();
            }
        }
        (dir, conn)
    }

    fn assert_live_first(conn: &Connection, hits: &[crate::retrieval::Hit]) {
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        let classes = crate::retrieval::envs_for_chunks(conn, &ids);
        let mut seen_test = false;
        for h in hits {
            let env = classes.get(&h.chunk_id).cloned().unwrap_or_else(|| "live".to_string());
            if env == "test" {
                seen_test = true;
            } else {
                assert!(!seen_test, "live hit after a test hit — live-first broken");
            }
        }
    }

    #[test]
    fn orchestrate_keyword_slots_rescue_floor_rank_one() {
        // R39.2: 8+ TOC hits at weight 1.0 lock the floor-only keyword
        // rank-1 (B) out of the top-8 cut; the reserved slots (default 2)
        // pull it back via ordering. 0 disables.
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_KEYWORD_SLOTS");
        let (_dir, conn) = seed_slots_db();
        let b: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title='b.md'", [], |r| r.get(0)).unwrap();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        let toc = retrieval::toc_search(&conn, "alpha zephyr", None, None, 8, true);
        assert!(toc.len() >= 8, "need 8+ TOC hits for the lockout, got {}", toc.len());
        assert!(!toc.iter().any(|h| h.item_id == Some(b)), "B must be fenced out of TOC");
        let floor = retrieval::keyword_search(&conn, "alpha zephyr", None, 8, true);
        assert_eq!(floor[0].item_id, Some(b), "B must be the floor-only keyword rank-1");
        let hits = orchestrate(&conn, "alpha zephyr", None, None, &cfg);
        assert!(hits.len() <= 8, "slots displace, never grow: {}", hits.len());
        let b_hit = hits.iter().find(|h| h.item_id == Some(b)).expect("floor rank-1 must take a reserved slot");
        assert_eq!(b_hit.provenance.len(), 1);
        assert_eq!(b_hit.provenance[0].leg, "keyword");
        assert_eq!(b_hit.provenance[0].rank, 1);
        assert_live_first(&conn, &hits);
        // 0 disables: B stays locked out.
        std::env::set_var("ANCHOR_KEYWORD_SLOTS", "0");
        let off = orchestrate(&conn, "alpha zephyr", None, None, &cfg);
        assert!(!off.iter().any(|h| h.item_id == Some(b)), "slots=0 must leave B locked out");
        std::env::remove_var("ANCHOR_KEYWORD_SLOTS");
    }

    #[test]
    fn build_citations_carries_parent_shape() {
        let (dir, conn, _settings) = seed_parent_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let (cid, iid, sec): (i64, i64, i64) = conn
            .query_row(
                "SELECT c.id, c.item_id, c.section_id FROM chunks c WHERE c.content LIKE 'tungsten invoice first%'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let mut hit = mk_hit(1, false);
        hit.chunk_id = cid;
        hit.item_id = Some(iid);
        hit.section_id = Some(sec);
        hit.path = "Billing".to_string();
        // Terms early, mark late: the preview must window on the mark (a
        // vector-style match), not on the first term occurrence.
        hit.content = format!("tungsten early {}[match {}]\nlate match bytes\n[/match {}]", "zz ".repeat(400), cid, cid);
        let cites = build_citations(std::slice::from_ref(&hit), &data_dir, true, 5, "tungsten");
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].chunk_id, Some(cid));
        assert_eq!(cites[0].section_id, Some(sec));
        assert_eq!(cites[0].breadcrumb, "Billing");
        assert_eq!(cites[0].path, "Billing");
        assert_eq!(cites[0].item_id, Some(iid));
        assert!(cites[0].summary.contains(&format!("[match {}]", cid)), "summary: {}", cites[0].summary);
        assert!(!cites[0].summary.contains("tungsten early"), "summary must not pull to early terms: {}", cites[0].summary);
        assert!(cites[0].snippet.contains(&format!("[match {}]", cid)));
        assert_eq!(cites[0].content, hit.content, "opt-in returns verbatim parent bytes");
    }

    #[test]
    fn hit_preview_centers_marked_parents_on_the_mark() {
        let content = format!("tungsten early {}[match 9]\nlate match bytes\n[/match 9]", "zz ".repeat(400));
        let s = hit_preview(&content, "tungsten", 9, 200);
        assert!(s.contains("[match 9]"), "marked parents window on the mark, got: {s}");
        assert!(!s.contains("tungsten early"), "terms outside the mark don't pull the window");
        // Unmarked content keeps match_snippet behavior.
        assert_eq!(hit_preview("plain content", "zzz", 1, 50), crate::retrieval::match_snippet("plain content", "zzz", 50));
    }

    /// Unit 768-vector with exact cosine `c` against e0.
    fn unit_with_first_cos(c: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 768];
        v[0] = c;
        v[1] = (1.0 - c * c).sqrt();
        v
    }

    #[test]
    fn orchestrate_relevance_gate_hybrid_and_keyword_paths() {
        // R39.3: hybrid evidence under the cosine floor (0.30, no BM25
        // match) gates to empty; near-identical cosine (0.99) passes, and
        // the keyword-only path passes on its BM25 match. 768-dim crafted
        // vectors = default ANCHOR_EMBED_DIM, so no env games.
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_MIN_RELEVANCE_COSINE");
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','gated.md','docs/gated.md','treasury routes content here')",
            [sid],
        )
        .unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title='gated.md'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'gated.md','treasury routes content here')", [iid])
            .unwrap();
        let cid: i64 = conn.last_insert_rowid();
        let mut e0 = vec![0.0f32; 768];
        e0[0] = 1.0;
        conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![crate::embedder::pack_single(&e0), cid]).unwrap();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        // Hybrid, low cosine, no keyword match → empty (ask: mode "none").
        let low = orchestrate(&conn, "zzqxqjsupercal", Some(unit_with_first_cos(0.30)), None, &cfg);
        assert!(low.is_empty(), "0.30-cosine vector-only evidence must gate, got {:?}", low.iter().map(|h| h.chunk_id).collect::<Vec<_>>());
        // Hybrid, high cosine → passes.
        let high = orchestrate(&conn, "zzqxqjsupercal", Some(unit_with_first_cos(0.99)), None, &cfg);
        assert_eq!(high.len(), 1);
        assert_eq!(high[0].chunk_id, cid);
        // Keyword-only path: the BM25 match passes with no cosine at all.
        let kw = orchestrate(&conn, "treasury routes", None, None, &cfg);
        assert_eq!(kw.len(), 1);
        assert_eq!(kw[0].chunk_id, cid);
    }

    #[test]
    fn orchestrate_marks_filename_boosted_hits() {
        // R39.1: a bare-filename query matches no content — the surfaced
        // chunk arrives via filename boost: boosted=true, empty provenance
        // (it bypassed ranking), and it survives the relevance gate (R39.3)
        // as explicit intent.
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','routing-config.yml','docs/routing-config.yml','treasury config content here')",
            [sid],
        )
        .unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title='routing-config.yml'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'routing-config.yml','treasury config content here')", [iid])
            .unwrap();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        let hits = orchestrate(&conn, "routing-config.yml", None, None, &cfg);
        assert_eq!(hits.len(), 1, "bare filename must surface its file");
        assert!(hits[0].boosted);
        assert!(hits[0].provenance.is_empty(), "boosted hits bypass ranking");
    }

    #[test]
    fn orchestrate_never_returns_entity_hits() {
        // R33.3: end of the retrieval stack — a one-line entity paraphrase
        // matching every term plus a linked neighbor (graph bait) must both
        // stay out of ask/search evidence; the document chunk cites alone.
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','note.md','docs/note.md','tungsten ledger body')",
            [sid],
        )
        .unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title='note.md'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'note.md','tungsten ledger document body words')",
            [iid],
        )
        .unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','tungsten ledger')", [iid]).unwrap();
        let e1: i64 = conn.last_insert_rowid();
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'note','tungsten ledger neighbor')", [iid]).unwrap();
        let e2: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','note.md','tungsten ledger')",
            [e1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'related')",
            rusqlite::params![e1, e2],
        )
        .unwrap();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        let hits = orchestrate(&conn, "tungsten ledger", None, None, &cfg);
        assert!(!hits.is_empty(), "the document chunk must still match");
        assert!(
            hits.iter().all(|h| h.entity_id.is_none()),
            "no entity chunk — direct or graph neighbor — may surface: {:?}",
            hits.iter().map(|h| (h.chunk_id, h.entity_id)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn orchestrate_boost_never_surfaces_disputed_chunks() {
        // R39.5 regression: a disputed chunk reachable ONLY via filename
        // boost (a bare-filename query matches no content) must not surface —
        // and the `0` opt-out proves the cfg flag actually threads into the
        // boost instead of the old hardcoded value.
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::set_var("ANCHOR_QA_EXCLUDE_DISPUTED", "1");
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h','routing-config.yml','docs/routing-config.yml','treasury config content here')",
            [sid],
        )
        .unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title='routing-config.yml'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary, status) VALUES (?1,'note','treasury config','disputed')", [iid]).unwrap();
        let eid: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, entity_id, source_ref, content) VALUES (?1,?2,'routing-config.yml','treasury config content here')",
            rusqlite::params![iid, eid],
        )
        .unwrap();
        let cfg = RetrievalConfig::from_env_with_top_k(8);
        assert!(cfg.qa_exclude_disputed);
        let hits = orchestrate(&conn, "routing-config.yml", None, None, &cfg);
        assert!(hits.is_empty(), "disputed-only boost must surface nothing, got {:?}", hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>());
        std::env::set_var("ANCHOR_QA_EXCLUDE_DISPUTED", "0");
        let cfg_off = RetrievalConfig::from_env_with_top_k(8);
        assert!(!cfg_off.qa_exclude_disputed);
        let off = orchestrate(&conn, "routing-config.yml", None, None, &cfg_off);
        assert_eq!(off.len(), 1, "opt-out must surface the disputed chunk via boost");
        assert!(off[0].boosted);
        std::env::remove_var("ANCHOR_QA_EXCLUDE_DISPUTED");
    }

    #[test]
    fn orchestrate_seeded_status_corpus_returns_full_top_k() {
        // R39.5 pin: live + stale + disputed chunks all matching — the fused
        // list still fills top_k with live hits (excluded rows never eat
        // slots, and the post-parent_return re-filter keeps only them out).
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_QA_EXCLUDE_DISPUTED");
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        for n in 1..=5 {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?3,'x')",
                rusqlite::params![sid, format!("live{n}.md"), format!("docs/live{n}.md")],
            )
            .unwrap();
            let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title=?1", [format!("live{n}.md")], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'ref','tungsten ledger body words here')", [iid]).unwrap();
        }
        for (title, status) in [("stale.md", "stale"), ("disputed.md", "disputed")] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, external_id, text) VALUES (?1,'h',?2,?3,'x')",
                rusqlite::params![sid, title, format!("docs/{title}")],
            )
            .unwrap();
            let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE title=?1", [title], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO entities (item_id, kind, summary, status) VALUES (?1,'note','tungsten ledger',?2)", rusqlite::params![iid, status]).unwrap();
            let eid: i64 = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO chunks (item_id, entity_id, source_ref, content) VALUES (?1,?2,'ref','tungsten ledger')",
                rusqlite::params![iid, eid],
            )
            .unwrap();
        }
        let cfg = RetrievalConfig::from_env_with_top_k(3);
        assert!(cfg.qa_exclude_disputed);
        let hits = orchestrate(&conn, "tungsten ledger", None, None, &cfg);
        assert_eq!(hits.len(), 3, "seeded corpus must still fill top_k, got {:?}", hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>());
        assert!(
            hits.iter().all(|h| h.status.as_deref() != Some("stale") && h.status.as_deref() != Some("disputed")),
            "no excluded status may surface: {:?}",
            hits.iter().map(|h| (h.chunk_id, h.status.clone())).collect::<Vec<_>>()
        );
    }
}
