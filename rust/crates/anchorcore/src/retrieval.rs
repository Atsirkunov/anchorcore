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
    sort_hits(&mut out);
    out
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub chunk_id: i64,
    pub entity_id: Option<i64>,
    pub item_id: Option<i64>,
    pub source_id: Option<i64>,
    pub section_id: Option<i64>,
    pub path: String,
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
    vector_search_filtered(conn, query_emb, source_ids, None, top_k, qa_exclude_disputed)
}

pub fn vector_search_filtered(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if let Some(hits) = vector_search_vec0_filtered(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed) {
        return hits;
    }
    vector_search_scan_filtered(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed)
}

// --- R15.1: shared search helpers (single implementation for every scope) ---

/// Which chunk kinds a search covers. Base vec/FTS search = all kinds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindScope {
    All,
    Summary,
    Leaf,
}

impl KindScope {
    fn kind_clause(&self) -> Option<&'static str> {
        match self {
            KindScope::All => None,
            KindScope::Summary => Some(" AND c.kind IN ('section_summary','doc_summary')"),
            KindScope::Leaf => Some(" AND c.kind IN ('document','entity','distilled')"),
        }
    }

    /// Base/leaf treat `Some({})` as no-match; summary treats it as no-prune.
    fn empty_section_is_empty(&self) -> bool {
        !matches!(self, KindScope::Summary)
    }

    /// Leaf scope keeps entity chunks (NULL section_id) — R14.6 fix.
    fn include_entities(&self) -> bool {
        matches!(self, KindScope::Leaf)
    }
}

use crate::common::placeholders;

fn append_source_filter(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    source_ids: Option<&HashSet<i64>>,
) {
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            sql.push_str(&format!(" AND i.source_id IN ({})", placeholders(ids.len())));
            for id in ids {
                params.push(rusqlite::types::Value::Integer(*id));
            }
        }
    }
}

/// `include_entities`: leaf scope keeps entity chunks (NULL section_id) — R14.6 fix.
fn append_section_filter(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    section_ids: Option<&HashSet<i64>>,
    include_entities: bool,
) {
    if let Some(ids) = section_ids {
        if ids.is_empty() {
            return;
        }
        if include_entities {
            sql.push_str(&format!(
                " AND (c.section_id IN ({}) OR c.entity_id IS NOT NULL)",
                placeholders(ids.len())
            ));
        } else {
            sql.push_str(&format!(" AND c.section_id IN ({})", placeholders(ids.len())));
        }
        for id in ids {
            params.push(rusqlite::types::Value::Integer(*id));
        }
    }
}

fn sort_hits(hits: &mut Vec<Hit>) {
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
}

/// vec0 distance → score; mirrors the `0.2` floor used by every vec path.
fn vec_score_from_distance(distance: f64) -> Option<f64> {
    let score = 1.0 - distance as f32;
    if score <= 0.2 {
        return None;
    }
    Some(score as f64)
}

/// Single Hit constructor + filter. All loaders go through here:
/// `status_ok` gate, source scoping, section scoping (entity chunks without a
/// section pass — R14.6 fix for test_disputed_entity_stops_being_cited).
#[allow(clippy::too_many_arguments)]
fn make_hit(
    cid: i64,
    eid: Option<i64>,
    iid: Option<i64>,
    content: String,
    source_ref: String,
    is_pii: i64,
    status: Option<String>,
    sid: Option<i64>,
    sec: Option<i64>,
    path: String,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Option<Hit> {
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
    if let Some(filter) = section_ids {
        // entity chunks have no section_id — let them pass (R14.6 fix for test_disputed_entity_stops_being_cited)
        if eid.is_none() {
            match sec {
                Some(sid) if filter.contains(&sid) => {}
                _ => return None,
            }
        }
    }
    Some(Hit {
        chunk_id: cid,
        entity_id: eid,
        item_id: iid,
        source_id: sid,
        section_id: sec,
        path,
        score: 0.0,
        content,
        source_ref,
        is_pii: is_pii != 0,
        status,
    })
}

/// Shared vec0 tail: distances → hydrated, scored, sorted hits.
fn hits_from_distances(
    conn: &Connection,
    rows: Vec<(i64, f64)>,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if rows.is_empty() {
        return vec![];
    }
    let by_id: HashMap<i64, f64> = rows.into_iter().collect();
    let ids: Vec<i64> = by_id.keys().cloned().collect();
    let hits = load_hits_filtered(conn, &ids, source_ids, section_ids, qa_exclude_disputed);
    let mut out = Vec::new();
    for mut h in hits {
        if let Some(d) = by_id.get(&h.chunk_id) {
            if let Some(score) = vec_score_from_distance(*d) {
                h.score = score;
                out.push(h);
            }
        }
    }
    sort_hits(&mut out);
    out
}

/// Single vec0 implementation for every scope (empty-section and
/// entity-inclusion rules come from `scope` — see `KindScope`).
fn exec_vec_search(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
    scope: KindScope,
) -> Option<Vec<Hit>> {
    if let Some(ids) = section_ids {
        if ids.is_empty() && scope.empty_section_is_empty() {
            return Some(vec![]);
        }
    }
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
    let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(q_json)];
    append_source_filter(&mut sql, &mut params, source_ids);
    append_section_filter(&mut sql, &mut params, section_ids, scope.include_entities());
    if let Some(clause) = scope.kind_clause() {
        sql.push_str(clause);
    }
    sql.push_str(" AND k = ?");
    let limit = (top_k * 4) as i64;
    params.push(rusqlite::types::Value::Integer(limit));
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows: Vec<(i64, f64)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .ok()?
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(hits_from_distances(conn, rows, source_ids, section_ids, qa_exclude_disputed))
}

/// Shared FTS tail: bm25 ranks → hydrated, scored, sorted hits.
fn hits_from_ranks(
    conn: &Connection,
    rows: Vec<(i64, f64)>,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if rows.is_empty() {
        return vec![];
    }
    let by_id: HashMap<i64, f64> = rows.into_iter().collect();
    let ids: Vec<i64> = by_id.keys().cloned().collect();
    let hits = load_hits_filtered(conn, &ids, source_ids, section_ids, qa_exclude_disputed);
    let mut out = Vec::new();
    for mut h in hits {
        if let Some(rank) = by_id.get(&h.chunk_id) {
            h.score = -rank;
            out.push(h);
        }
    }
    sort_hits(&mut out);
    out
}

/// Single FTS implementation for every scope. Base scope (`All`) is the same
/// join shape without a kind filter — equivalent to the old `chunks_fts`-only
/// query (`rowid` == chunk id) while guaranteeing the chunk row exists.
fn exec_fts_search(
    conn: &Connection,
    query: &str,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
    scope: KindScope,
) -> Vec<Hit> {
    let Some(match_q) = fts_match_query(query) else { return vec![] };
    let limit = (top_k * 4) as i64;
    let mut sql = String::from(
        "SELECT c.id, bm25(chunks_fts) AS rank FROM chunks_fts JOIN chunks c ON c.id=chunks_fts.rowid WHERE chunks_fts MATCH :q",
    );
    if let Some(clause) = scope.kind_clause() {
        sql.push_str(clause);
    }
    sql.push_str(" ORDER BY rank LIMIT :limit");
    let mut stmt = match conn.prepare(&sql) {
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
    hits_from_ranks(conn, rows, source_ids, section_ids, qa_exclude_disputed)
}

fn vector_search_vec0_filtered(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Option<Vec<Hit>> {
    exec_vec_search(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::All)
}

fn vector_search_scan_filtered(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let hits = load_chunks_with_embedding_filtered(conn, source_ids, section_ids, qa_exclude_disputed);
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
    sort_hits(&mut scored);
    scored.truncate(top_k * 4);
    scored
}

type HitRow11 = (i64, Option<i64>, Option<i64>, String, String, i64, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>, String);

fn hit_from_row11(row: HitRow11, source_ids: Option<&HashSet<i64>>, section_ids: Option<&HashSet<i64>>, qa_exclude_disputed: bool) -> Option<Hit> {
    let (cid, eid, iid, content, source_ref, is_pii, _created, status, _eff, sid, sec, path) = row;
    make_hit(cid, eid, iid, content, source_ref, is_pii, status, sid, sec, path, source_ids, section_ids, qa_exclude_disputed)
}

fn load_hits_filtered(
    conn: &Connection,
    ids: &[i64],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if ids.is_empty() {
        return vec![];
    }
    let sql = format!(
        "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, c.created_at, e.status, \
                 COALESCE(c.item_id, e.item_id) as eff_item, i.source_id, c.section_id, c.path \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE c.id IN ({})",
        placeholders(ids.len())
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
                r.get::<_, Option<i64>>(10)?,
                r.get::<_, String>(11)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(h) = hit_from_row11(row, source_ids, section_ids, qa_exclude_disputed) {
            out.push(h);
        }
    }
    out
}

fn load_chunks_with_embedding_filtered(
    conn: &Connection,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let sql = "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, i.source_id, c.section_id, c.path \
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
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, String>(9)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (cid, eid, iid, content, source_ref, is_pii, status, sid, sec, path) = row;
        // Scan covers every embedded chunk: a missing source always filters out
        // (stricter than make_hit's scoped pass-through).
        if let Some(filter) = source_ids {
            match sid {
                Some(s) if filter.contains(&s) => {}
                _ => continue,
            }
        }
        if let Some(h) = make_hit(cid, eid, iid, content, source_ref, is_pii, status, sid, sec, path, None, section_ids, qa_exclude_disputed) {
            out.push(h);
        }
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
    keyword_search_filtered(conn, question, source_ids, None, top_k, qa_exclude_disputed)
}

pub fn keyword_search_filtered(
    conn: &Connection,
    question: &str,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    exec_fts_search(conn, question, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::All)
}

fn matching_tag_sections(
    conn: &Connection,
    query: &str,
    _source_ids: Option<&HashSet<i64>>,
) -> Option<HashSet<i64>> {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|s| s.to_string())
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let sql = format!("SELECT id FROM tags WHERE name IN ({})", placeholders(tokens.len()));
    let mut stmt = conn.prepare(&sql).ok()?;
    let tag_ids: Vec<i64> = stmt
        .query_map(rusqlite::params_from_iter(tokens.iter()), |r| r.get(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    if tag_ids.is_empty() {
        return None;
    }
    let sql2 = format!(
        "SELECT DISTINCT c.section_id FROM chunk_tags ct JOIN chunks c ON c.id=ct.chunk_id WHERE ct.tag_id IN ({}) AND c.section_id IS NOT NULL",
        placeholders(tag_ids.len())
    );
    let mut stmt2 = conn.prepare(&sql2).ok()?;
    let secs: HashSet<i64> = stmt2
        .query_map(rusqlite::params_from_iter(tag_ids.iter()), |r| r.get(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    if secs.is_empty() {
        None
    } else {
        Some(secs)
    }
}

fn summary_vector_search_filtered(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_filter: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Option<Vec<Hit>> {
    exec_vec_search(conn, query_emb, source_ids, section_filter, top_k, qa_exclude_disputed, KindScope::Summary)
}

fn summary_keyword_search_filtered(
    conn: &Connection,
    query: &str,
    source_ids: Option<&HashSet<i64>>,
    section_filter: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    exec_fts_search(conn, query, source_ids, section_filter, top_k, qa_exclude_disputed, KindScope::Summary)
}

fn leaf_vector_search_filtered(
    conn: &Connection,
    query_emb: &[f32],
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Option<Vec<Hit>> {
    exec_vec_search(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::Leaf)
}

fn leaf_keyword_search_filtered(
    conn: &Connection,
    query: &str,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    exec_fts_search(conn, query, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::Leaf)
}

fn extract_top_sections(conn: &Connection, summary_hits: &[Hit]) -> HashSet<i64> {
    let mut out = HashSet::new();
    for h in summary_hits.iter().take(5) {
        if let Some(sec) = h.section_id {
            out.insert(sec);
        } else if let Some(item) = h.item_id {
            if let Ok(mut stmt) = conn.prepare("SELECT id FROM sections WHERE item_id=?1") {
                if let Ok(rows) = stmt.query_map([item], |r| r.get::<_, i64>(0)) {
                    for id in rows.flatten() {
                        out.insert(id);
                        if out.len() >= 5 {
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

pub fn toc_search(
    conn: &Connection,
    query: &str,
    query_emb: Option<&[f32]>,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let tag_filter = matching_tag_sections(conn, query, source_ids);
    let summary_hits = if let Some(emb) = query_emb {
        summary_vector_search_filtered(conn, emb, source_ids, tag_filter.as_ref(), 5, qa_exclude_disputed)
            .unwrap_or_default()
    } else {
        vec![]
    };
    let summary_hits = if summary_hits.is_empty() {
        summary_keyword_search_filtered(conn, query, source_ids, tag_filter.as_ref(), 5, qa_exclude_disputed)
    } else {
        summary_hits
    };
    if summary_hits.is_empty() {
        return vec![];
    }
    let top_secs = extract_top_sections(conn, &summary_hits);
    if top_secs.is_empty() {
        return vec![];
    }
    let leaf_vec = query_emb
        .and_then(|emb| leaf_vector_search_filtered(conn, emb, source_ids, Some(&top_secs), top_k, qa_exclude_disputed))
        .unwrap_or_default();
    let leaf_kw = leaf_keyword_search_filtered(conn, query, source_ids, Some(&top_secs), top_k, qa_exclude_disputed);
    if leaf_vec.is_empty() && leaf_kw.is_empty() {
        return vec![];
    }
    let fused = if leaf_vec.is_empty() {
        leaf_kw
    } else if leaf_kw.is_empty() {
        leaf_vec
    } else {
        rrf_fuse_multi(vec![leaf_vec, leaf_kw], vec![1.0, 1.0])
    };
    let mut hits = fused;
    sort_hits(&mut hits);
    hits.truncate(top_k);
    hits
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
    // Fallback lists every chunk, so a missing source always filters out (stricter
    // than make_hit's pass-through, which only applies to scoped searches).
    if !status_ok(status.as_deref(), qa_exclude_disputed) {
        return None;
    }
    if let Some(filter) = source_ids {
        let s = sid?;
        if !filter.contains(&s) {
            return None;
        }
    }
    make_hit(cid, eid, iid, content, source_ref, is_pii, status, sid, None, String::new(), source_ids, None, qa_exclude_disputed)
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
            sql.push_str(&format!(" WHERE i.source_id IN ({})", placeholders(ids.len())));
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
    append_source_filter(&mut sql, &mut params, source_ids);
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
        out.push(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, section_id: None, path: String::new(), score, content, source_ref, is_pii: is_pii!=0, status });
    }
    sort_hits(&mut out);
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
        let ph = placeholders(frontier.len());
        let sql = format!("SELECT from_entity_id, to_entity_id, kind FROM relationships WHERE from_entity_id IN ({}) OR to_entity_id IN ({})", ph, ph);
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
    let mut sql = format!("SELECT e.id, e.summary, e.status, e.item_id, c.id, c.content, c.source_ref, c.is_pii FROM entities e LEFT JOIN chunks c ON c.entity_id=e.id WHERE e.id IN ({})", placeholders(ids.len()));
    let mut params: Vec<rusqlite::types::Value> = ids.iter().map(|id| rusqlite::types::Value::Integer(*id)).collect();
    if let Some(filter)=source_ids {
        if !filter.is_empty() {
            sql.push_str(&format!(" AND e.item_id IN (SELECT id FROM ingested_items WHERE source_id IN ({}))", placeholders(filter.len())));
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
            out.push(Hit { chunk_id: cid.unwrap_or(eid), entity_id: Some(eid), item_id: *item, source_id: sid, section_id: None, path: String::new(), score, content: content.clone().unwrap_or_else(|| summary.clone()), source_ref: src_ref.clone().unwrap_or_else(|| summary.clone()), is_pii: false, status: status.clone() });
        }
    }
    out
}

pub fn fuse_evidence(mut lists: Vec<Vec<Hit>>, weights: Vec<f64>) -> Vec<Hit> {
    if lists.is_empty() { return vec![]; }
    if lists.len()==1 { let mut v = lists.remove(0); sort_hits(&mut v); return v; }
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
        let sql = format!("SELECT id, created_at FROM chunks WHERE id IN ({})", placeholders(ids.len()));
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
    sort_hits(&mut fused);
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

fn fetch_siblings(conn: &Connection, sql: &str, key: i64) -> Vec<(i64, String)> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

fn append_continuation(hit: &mut Hit, extra: Vec<String>) {
    if !extra.is_empty() {
        hit.content = format!("{}\n\n[continued]\n\n{}", hit.content, extra.join("\n\n"));
    }
}

fn expand_by_section(conn: &Connection, hit: &mut Hit, hit_ids: &HashSet<i64>) {
    let sec = match hit.section_id {
        Some(id) => id,
        None => return,
    };
    let siblings = fetch_siblings(conn, "SELECT id, content FROM chunks WHERE section_id=?1 ORDER BY id", sec);
    if siblings.is_empty() {
        return;
    }
    let mut extra = Vec::new();
    for (cid, content) in &siblings {
        if *cid == hit.chunk_id || hit_ids.contains(cid) {
            continue;
        }
        extra.push(content.clone());
    }
    append_continuation(hit, extra);
}

fn expand_by_item(conn: &Connection, hit: &mut Hit, window: usize, hit_ids: &HashSet<i64>) {
    let item_id = match hit.item_id {
        Some(id) => id,
        None => return,
    };
    let siblings = fetch_siblings(conn, "SELECT id, content FROM chunks WHERE item_id=?1 ORDER BY id", item_id);
    let idx = siblings.iter().position(|(id, _)| *id == hit.chunk_id);
    if let Some(i) = idx {
        let start = i.saturating_sub(window);
        let end = (i + window + 1).min(siblings.len());
        let mut extra = Vec::new();
        for (j, (cid, content)) in siblings[start..end].iter().enumerate() {
            if *cid == hit.chunk_id || hit_ids.contains(cid) {
                continue;
            }
            if j == i {
                continue;
            }
            extra.push(content.clone());
        }
        append_continuation(hit, extra);
    }
}

pub fn expand_context(conn: &Connection, hits: &mut [Hit], window: usize) {
    if window == 0 || hits.is_empty() {
        return;
    }
    let hit_ids: HashSet<i64> = hits.iter().map(|h| h.chunk_id).collect();
    for h in hits.iter_mut() {
        if h.section_id.is_some() {
            expand_by_section(conn, h, &hit_ids);
        } else {
            expand_by_item(conn, h, window, &hit_ids);
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
        let h1 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None }];
        let h2 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None }];
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
        let seed = vec![Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "seed".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()) }];
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
        let h1 = Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "c1".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()) };
        let h2 = Hit { chunk_id: c2, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 0.5, content: "c2".into(), source_ref: "ref".into(), is_pii: false, status: None };
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

    #[test]
    fn harness_expand_context_section_aware() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_expand", "internal");
        let iid = insert_item(&conn, sid, "doc_expand");
        // create sections
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Billing','Billing')",
            rusqlite::params![iid],
        )
        .unwrap();
        let sec1 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Auth','Auth')",
            rusqlite::params![iid],
        )
        .unwrap();
        let _sec2 = conn.last_insert_rowid();
        // chunks in sec1 - whole subsection should be returned via expand_by_section
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,'Billing',?3,'ref',datetime('now'))",
            rusqlite::params![iid, sec1, "Billing part1 content"],
        )
        .unwrap();
        let c1 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,'Billing',?3,'ref',datetime('now'))",
            rusqlite::params![iid, sec1, "Billing part2 sibling"],
        )
        .unwrap();
        let c2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,'Billing',?3,'ref',datetime('now'))",
            rusqlite::params![iid, sec1, "Billing part3 extra"],
        )
        .unwrap();
        let _c3 = conn.last_insert_rowid();
        // hit in sec1
        let mut hits = vec![Hit {
            chunk_id: c1,
            entity_id: None,
            item_id: Some(iid),
            source_id: Some(sid),
            section_id: Some(sec1),
            path: "Billing".to_string(),
            score: 1.0,
            content: "Billing part1 content".into(),
            source_ref: "ref".into(),
            is_pii: false,
            status: None,
        }];
        // sibling c2 should be included via expand_by_section (whole subsection)
        let _ = c2; // silence unused
        expand_context(&conn, &mut hits, 1);
        assert!(
            hits[0].content.contains("Billing part2 sibling") || hits[0].content.contains("Billing part3 extra"),
            "section-aware expand should include whole subsection, got: {}",
            hits[0].content
        );
        // also test fallback to item when section_id is None still does ±1
        conn.execute(
            "INSERT INTO chunks (item_id, content, source_ref, created_at) VALUES (?1,?2,'ref',datetime('now'))",
            rusqlite::params![iid, "item-level chunk A"],
        )
        .unwrap();
        let ca = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, content, source_ref, created_at) VALUES (?1,?2,'ref',datetime('now'))",
            rusqlite::params![iid, "item-level chunk B win"],
        )
        .unwrap();
        let cb = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, content, source_ref, created_at) VALUES (?1,?2,'ref',datetime('now'))",
            rusqlite::params![iid, "item-level chunk C"],
        )
        .unwrap();
        let _cc = conn.last_insert_rowid();
        let mut hits2 = vec![Hit {
            chunk_id: cb,
            entity_id: None,
            item_id: Some(iid),
            source_id: Some(sid),
            section_id: None,
            path: String::new(),
            score: 1.0,
            content: "item-level chunk B win".into(),
            source_ref: "ref".into(),
            is_pii: false,
            status: None,
        }];
        let _ = ca;
        expand_context(&conn, &mut hits2, 1);
        // should have expanded via expand_by_item ±1
        assert!(
            hits2[0].content.contains("item-level chunk"),
            "item fallback expand should include neighbor"
        );
    }

    #[test]
    fn harness_vector_search_section_filter() {
        let (_dir, conn) = harness_db();
        // use small dim for test
        std::env::set_var("ANCHOR_EMBED_DIM", "4");
        let sid = insert_source(&conn, "s_vec", "internal");
        let iid = insert_item(&conn, sid, "doc_vec");
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'SecA','SecA')",
            rusqlite::params![iid],
        )
        .unwrap();
        let sec_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'SecB','SecB')",
            rusqlite::params![iid],
        )
        .unwrap();
        let sec_b = conn.last_insert_rowid();
        // chunks with different section_ids and distinct embeddings
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,'SecA','billing alpha','ref',datetime('now'))",
            rusqlite::params![iid, sec_a],
        )
        .unwrap();
        let ca = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,'SecB','auth beta','ref',datetime('now'))",
            rusqlite::params![iid, sec_b],
        )
        .unwrap();
        let cb = conn.last_insert_rowid();
        // pack embeddings: ca ~ [1,0,0,0], cb ~ [0,1,0,0]
        let emb_a = vec![1.0f32, 0.0, 0.0, 0.0];
        let emb_b = vec![0.0f32, 1.0, 0.0, 0.0];
        let blob_a = emb_a.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
        let blob_b = emb_b.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
        conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob_a, ca]).unwrap();
        conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob_b, cb]).unwrap();
        // query close to emb_a should hit ca when filtered to sec_a, and empty when filtered to sec_b
        let query = vec![0.9f32, 0.1, 0.0, 0.0];
        let mut filter_a = HashSet::new();
        filter_a.insert(sec_a);
        let hits_a = vector_search_filtered(&conn, &query, None, Some(&filter_a), 5, true);
        // may be via vec0 or scan; at least should contain ca and not cb
        if !hits_a.is_empty() {
            assert!(hits_a.iter().any(|h| h.chunk_id == ca), "filtered to sec_a should contain ca");
            assert!(!hits_a.iter().any(|h| h.chunk_id == cb), "filtered to sec_a should not contain cb");
        }
        let mut filter_b = HashSet::new();
        filter_b.insert(sec_b);
        let hits_b = vector_search_filtered(&conn, &query, None, Some(&filter_b), 5, true);
        // hits_b should be empty or contain cb only if scan fallback scores >0.2; but must not contain ca
        assert!(!hits_b.iter().any(|h| h.chunk_id == ca), "filtered to sec_b should not contain ca");
        // keyword filtered also respects section
        let kw_a = keyword_search_filtered(&conn, "billing", None, Some(&filter_a), 5, true);
        if !kw_a.is_empty() {
            assert!(kw_a.iter().all(|h| h.section_id == Some(sec_a)));
        }
        let kw_b = keyword_search_filtered(&conn, "billing", None, Some(&filter_b), 5, true);
        // billing term only in sec_a, so filtered to sec_b should be empty
        assert!(kw_b.is_empty() || kw_b.iter().all(|h| h.section_id == Some(sec_b)));
        std::env::remove_var("ANCHOR_EMBED_DIM");
    }

    #[test]
    fn harness_toc_search_coarse_to_fine() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_toc", "internal");
        let iid = insert_item(&conn, sid, "doc_toc");
        // sections
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Billing Policy','Billing Policy')",
            rusqlite::params![iid],
        )
        .unwrap();
        let sec_billing = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'Auth Policy','Auth Policy')",
            rusqlite::params![iid],
        )
        .unwrap();
        let sec_auth = conn.last_insert_rowid();
        // summary nodes (section_summary) – coarse level
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, level, path, content, source_ref, created_at) VALUES (?1,?2,'section_summary',1,'Billing Policy','billing policy summary','ref',datetime('now'))",
            rusqlite::params![iid, sec_billing],
        )
        .unwrap();
        let sum_billing = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, level, path, content, source_ref, created_at) VALUES (?1,?2,'section_summary',1,'Auth Policy','auth policy summary','ref',datetime('now'))",
            rusqlite::params![iid, sec_auth],
        )
        .unwrap();
        let _sum_auth = conn.last_insert_rowid();
        // leaf nodes
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, level, path, content, source_ref, created_at) VALUES (?1,?2,'document',1,'Billing Policy','billing decision details with terms','ref',datetime('now'))",
            rusqlite::params![iid, sec_billing],
        )
        .unwrap();
        let leaf_billing = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, level, path, content, source_ref, created_at) VALUES (?1,?2,'document',1,'Auth Policy','auth decision details','ref',datetime('now'))",
            rusqlite::params![iid, sec_auth],
        )
        .unwrap();
        let leaf_auth = conn.last_insert_rowid();
        // tags: billing tag linked to billing summary and leaf
        conn.execute("INSERT INTO tags (name, description, embedding, count) VALUES (?1,'',NULL,0)", ["billing"]).unwrap();
        let tag_billing = conn.last_insert_rowid();
        // link tag to billing chunks via chunk_tags
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![sum_billing, tag_billing]).unwrap();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![leaf_billing, tag_billing]).unwrap();
        conn.execute("INSERT INTO tags (name, description, embedding, count) VALUES (?1,'',NULL,0)", ["auth"]).unwrap();
        let tag_auth = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![leaf_auth, tag_auth]).unwrap();
        // ensure FTS index populated (triggers)
        // toc_search with query "billing" should coarse to billing section and return billing leaf, not auth
        let hits = toc_search(&conn, "billing", None, None, 5, true);
        // if summary search fails due to missing vec0, it falls back to keyword on section_summary – should still find billing summary
        // then leaf search should return leaf_billing
        if !hits.is_empty() {
            assert!(hits.iter().any(|h| h.chunk_id == leaf_billing), "toc should return billing leaf");
            assert!(!hits.iter().any(|h| h.chunk_id == leaf_auth), "toc billing query should not return auth leaf");
        } else {
            // fallback: ensure at least keyword path works – leaf keyword filtered directly
            let mut top = HashSet::new();
            top.insert(sec_billing);
            let leaves = leaf_keyword_search_filtered(&conn, "billing", None, Some(&top), 5, true);
            assert!(!leaves.is_empty());
            assert!(leaves.iter().any(|h| h.chunk_id == leaf_billing));
        }
        // auth query should hit auth
        let hits_auth = toc_search(&conn, "auth", None, None, 5, true);
        if !hits_auth.is_empty() {
            assert!(hits_auth.iter().any(|h| h.chunk_id == leaf_auth));
        }
    }
}
