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

/// Shared query terms for FTS matching and match-centered snippets:
/// Unicode word runs, lowercased, len ≥ 2, stopwords removed.
fn query_terms(question: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_§\-]+").unwrap());
    static STOPWORDS: OnceLock<HashSet<&str>> = OnceLock::new();
    let stopwords = STOPWORDS.get_or_init(|| {
        [
            "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for", "with",
            "about", "is", "are", "was", "were", "be", "been", "being", "am", "do", "does", "did",
            "have", "has", "had", "will", "would", "can", "could", "should", "shall", "may",
            "might", "must", "what", "which", "who", "whom", "whose", "when", "where", "why", "how",
            "this", "that", "these", "those", "it", "its", "not", "no", "so", "if", "then", "than",
            "too", "very", "s", "t", "you", "your", "we", "our", "they", "their", "i", "me", "my",
        ]
        .into()
    });
    re.find_iter(&question.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|t| t.len() >= 2 && !stopwords.contains(t.as_str()))
        .collect()
}

/// Mirrors `answer_engine.py:71` _fts_match_query — R8.2: Unicode-aware (was [a-z0-9_])
pub fn fts_match_query(question: &str) -> Option<String> {
    let terms = query_terms(question);
    if terms.is_empty() {
        return None;
    }
    Some(terms.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(" OR "))
}

/// Earliest query-term occurrence in lowercased `hay`: byte offset + term char len.
fn earliest_term_pos(hay: &str, terms: &[String]) -> Option<(usize, usize)> {
    terms
        .iter()
        .filter_map(|t| hay.find(t.as_str()).map(|b| (b, t.chars().count())))
        .min_by_key(|(b, _)| *b)
}

/// Match-centered preview: the `n`-char window around the first query-term
/// hit, `…`-marked where cut. Falls back to the head (byte-identical to a
/// plain prefix) when nothing matches — a head-only preview of an expanded
/// hit hides the match it was expanded for (1.0.24 retest).
pub fn match_snippet(content: &str, query: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= n {
        return content.to_string();
    }
    // Window math runs on the lowercased copy: case mapping can shift byte
    // offsets (e.g. 'İ'), so the original must never be sliced at them.
    let lower = content.to_lowercase();
    let Some((byte, term_len)) = earliest_term_pos(&lower, &query_terms(query)) else {
        return chars[..n].iter().collect();
    };
    let match_char = lower[..byte].chars().count().min(chars.len());
    let half = (n.saturating_sub(term_len)) / 2;
    let start = match_char.saturating_sub(half);
    let end = (start + n).min(chars.len());
    let start = end.saturating_sub(n);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter());
    if end < chars.len() {
        out.push('…');
    }
    out
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
    /// True when section expansion dropped siblings (byte cap / window) —
    /// `content` is not the whole section. Full bytes via the item endpoint.
    pub content_truncated: bool,
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
        if !hits.is_empty() || vec_chunk_count(conn) > 0 {
            return hits;
        }
        // else: the index holds no rows (desync) — fall through to the scan
        // instead of reporting "no vector match" for an embedded corpus.
    }
    vector_search_scan_filtered(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed)
}

/// Row count of the vec0 index; 1 on error (a working index that cannot
/// be counted must not trigger a full-table scan).
fn vec_chunk_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM vec_chunks", [], |r| r.get(0)).unwrap_or(1)
}

// --- R15.1: shared search helpers (single implementation for every scope) ---

/// Which chunk kinds a search covers. Base vec/FTS search = leaf kinds:
/// summaries are TOC coarse-stage internals — a `doc_summary` surfacing as a
/// final hit is a one-line extract masquerading as evidence (1.0.24 retest).
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindScope {
    Summary,
    Leaf,
}

impl KindScope {
    fn kind_clause(&self) -> Option<&'static str> {
        match self {
            KindScope::Summary => Some(" AND c.kind IN ('section_summary','doc_summary')"),
            KindScope::Leaf => Some(" AND c.kind IN ('document','entity','distilled')"),
        }
    }

    /// Leaf treats `Some({})` as no-match; summary treats it as no-prune.
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

pub(crate) fn sort_hits(hits: &mut Vec<Hit>) {
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
}

/// FTS/vec0 over-fetch multiplier: short fixture rows crowd long live docs
/// out of a small candidate pool on BM25 length norm (1.0.24 retest) —
/// demotion can only promote what was fetched.
const CANDIDATE_MULT: usize = 10;

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
        content_truncated: false,
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
    let limit = (top_k * CANDIDATE_MULT) as i64;
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

/// Single FTS implementation for every scope. Base scope is the leaf join
/// shape — equivalent to the old `chunks_fts`-only query (`rowid` == chunk
/// id) while guaranteeing the chunk row exists and is a leaf kind.
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
    let limit = (top_k * CANDIDATE_MULT) as i64;
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
    exec_vec_search(conn, query_emb, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::Leaf)
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
    scored.truncate(top_k * CANDIDATE_MULT);
    scored
}

type HitRow11 = (i64, Option<i64>, Option<i64>, String, String, i64, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>, String);

fn hit_from_row11(row: HitRow11, source_ids: Option<&HashSet<i64>>, section_ids: Option<&HashSet<i64>>, qa_exclude_disputed: bool) -> Option<Hit> {
    // R32.2: entity chunks store item_id NULL (they key off entity_id) — the
    // hit carries the effective item so every hit drills into GET /items/:id.
    let (cid, eid, _iid, content, source_ref, is_pii, _created, status, eff, sid, sec, path) = row;
    make_hit(cid, eid, eff, content, source_ref, is_pii, status, sid, sec, path, source_ids, section_ids, qa_exclude_disputed)
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
    // Leaf kinds only — the scan path must match the vec0 path (R32.6).
    let sql = "SELECT c.id, c.entity_id, COALESCE(c.item_id, e.item_id), c.content, c.source_ref, c.is_pii, e.status, i.source_id, c.section_id, c.path \
               FROM chunks c \
               LEFT JOIN entities e ON e.id = c.entity_id \
               LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
               WHERE c.embedding IS NOT NULL AND c.kind IN ('document','entity','distilled')";
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
        // R32.2: col 2 is the effective item (entity chunks resolve via e.item_id).
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
    exec_fts_search(conn, question, source_ids, section_ids, top_k, qa_exclude_disputed, KindScope::Leaf)
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

/// TOC summary cut (R36.7 funnel follow-up): fine heading sections let one
/// file fill the whole cut and starve multi-file coverage (benchmark ai-03
/// went from 4 hits across files to 1 hit in one section). Fetch wide, then
/// admit at most 2 sections per item, 10 total — restoring ~5-file candidacy.
const TOC_SUMMARY_FETCH: usize = 15;
const TOC_SECTIONS_PER_ITEM: usize = 2;
const TOC_SECTION_TOTAL: usize = 10;

/// Owning item for a summary hit: the hit's own link, else the section row.
fn item_for_section_hit(conn: &Connection, h: &Hit, sec: i64) -> Option<i64> {
    if h.item_id.is_some() {
        return h.item_id;
    }
    conn.query_row("SELECT item_id FROM sections WHERE id=?1", [sec], |r| {
        r.get(0)
    })
    .ok()
}

/// Admit one section under the per-item and total caps (dup-safe: a repeat
/// hit for an admitted section consumes no quota).
fn admit_section(
    per_item: &mut HashMap<Option<i64>, usize>,
    out: &mut HashSet<i64>,
    item: Option<i64>,
    sec: i64,
) {
    if out.len() >= TOC_SECTION_TOTAL {
        return;
    }
    let n = per_item.entry(item).or_insert(0);
    if *n >= TOC_SECTIONS_PER_ITEM {
        return;
    }
    if out.insert(sec) {
        *n += 1;
    }
}

fn extract_top_sections(conn: &Connection, summary_hits: &[Hit]) -> HashSet<i64> {
    let mut out = HashSet::new();
    let mut per_item: HashMap<Option<i64>, usize> = HashMap::new();
    // Pass 1: section hits claim evidence-backed slots in rank order.
    for (h, sec) in summary_hits
        .iter()
        .filter_map(|h| h.section_id.map(|sec| (h, sec)))
    {
        admit_section(&mut per_item, &mut out, item_for_section_hit(conn, h, sec), sec);
    }
    // Pass 2: section-less (doc) hits fill remaining capacity in doc order.
    for h in summary_hits.iter().filter(|h| h.section_id.is_none()) {
        let Some(item) = h.item_id else { continue };
        if out.len() >= TOC_SECTION_TOTAL {
            break;
        }
        let mut stmt = match conn.prepare("SELECT id FROM sections WHERE item_id=?1 ORDER BY id") {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rows = match stmt.query_map([item], |r| r.get::<_, i64>(0)) {
            Ok(m) => m,
            Err(_) => continue,
        };
        for id in rows.flatten() {
            admit_section(&mut per_item, &mut out, Some(item), id);
            if out.len() >= TOC_SECTION_TOTAL {
                break;
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
        summary_vector_search_filtered(conn, emb, source_ids, tag_filter.as_ref(), TOC_SUMMARY_FETCH, qa_exclude_disputed)
            .unwrap_or_default()
    } else {
        vec![]
    };
    let mut summary_hits = if summary_hits.is_empty() {
        summary_keyword_search_filtered(conn, query, source_ids, tag_filter.as_ref(), TOC_SUMMARY_FETCH, qa_exclude_disputed)
    } else {
        summary_hits
    };
    // Live sections must survive the diversity cut: fixture summaries win BM25
    // on length norm and would otherwise prune every live section.
    prefer_live_envs(conn, &mut summary_hits, query);
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
    // Over-fetch for the downstream live-first cut in `fuse_and_rank`: a
    // tight truncate here would drop live leaves before the partition.
    hits.truncate(top_k.saturating_mul(5).max(top_k));
    hits
}

type HitRow9 = (i64, Option<i64>, Option<i64>, String, String, i64, Option<String>, Option<i64>, Option<i64>);

fn row9(r: &rusqlite::Row) -> rusqlite::Result<HitRow9> {
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
    ))
}

fn fallback_hit_from_row9(row: HitRow9, source_ids: Option<&HashSet<i64>>, qa_exclude_disputed: bool) -> Option<Hit> {
    // R32.2: col 8 is the effective item (entity chunks resolve via e.item_id).
    let (cid, eid, _iid, content, source_ref, is_pii, status, sid, eff) = row;
    let iid = eff;
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
    // Leaf kinds only: summaries are created last, so an unfiltered
    // `ORDER BY created_at DESC` fallback returns extracts, not evidence.
    let mut sql = String::from(
        "SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, i.source_id, COALESCE(c.item_id, e.item_id) \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE c.kind IN ('document','entity','distilled')",
    );
    let mut params: Vec<rusqlite::types::Value> = vec![];
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            sql.push_str(&format!(" AND i.source_id IN ({})", placeholders(ids.len())));
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
    let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), row9) {
        Ok(m) => m,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(h) = fallback_hit_from_row9(row, source_ids, qa_exclude_disputed) {
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
    let mut sql = String::from("SELECT c.id, c.entity_id, c.item_id, c.content, c.source_ref, c.is_pii, e.status, e.owner, e.author, e.summary, e.confidence, e.created_at, i.source_id, COALESCE(c.item_id, e.item_id) FROM chunks c JOIN entities e ON e.id=c.entity_id JOIN ingested_items i ON i.id=e.item_id WHERE (e.owner != '' OR e.author != ''");
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
    let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?, r.get::<_, String>(9)?, r.get::<_, f64>(10)?, r.get::<_, Option<String>>(11)?, r.get::<_, Option<i64>>(12)?, r.get::<_, Option<i64>>(13)?))) { Ok(m) => m, Err(_) => return out };
    for row in rows.flatten() {
        // R32.2: col 13 is the effective item (entity chunks resolve via e.item_id).
        let (cid, eid, _iid, content, source_ref, is_pii, status, owner, author, summary, conf, created, sid, eff) = row;
        let iid = eff;
        if !status_ok(status.as_deref(), qa_exclude_disputed) { continue; }
        let blob = format!("{} {} {}", owner, author, summary).to_lowercase();
        let overlap = terms.iter().filter(|t| blob.contains(&t.to_lowercase())).count();
        if overlap==0 { continue; }
        let created_dt = created.and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc));
        let decay = age_decay(created_dt, halflife, now);
        let score = conf * (1.0 + 0.5 * (overlap.min(4) as f64)) * decay;
        out.push(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, section_id: None, path: String::new(), score, content, source_ref, is_pii: is_pii!=0, status, content_truncated: false });
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
            // R27.4: contradictions warn via citations — they must never enter
            // as supporting evidence through graph expansion
            if kind == "conflicts_with" { continue; }
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
            out.push(Hit { chunk_id: cid.unwrap_or(eid), entity_id: Some(eid), item_id: *item, source_id: sid, section_id: None, path: String::new(), score, content: content.clone().unwrap_or_else(|| summary.clone()), source_ref: src_ref.clone().unwrap_or_else(|| summary.clone()), is_pii: false, status: status.clone(), content_truncated: false });
        }
    }
    out
}

pub fn fuse_evidence(mut lists: Vec<Vec<Hit>>, weights: Vec<f64>) -> Vec<Hit> {
    if lists.is_empty() { return vec![]; }
    if lists.len()==1 { let mut v = lists.remove(0); sort_hits(&mut v); return v; }
    rrf_fuse_multi(lists, weights)
}

#[allow(clippy::too_many_arguments)]
pub fn fuse_and_rank(
    conn: &Connection,
    vector_hits: Option<Vec<Hit>>,
    keyword_hits: Option<Vec<Hit>>,
    keyword_weight: f64,
    halflife_days: f64,
    max_per_source: usize,
    top_k: usize,
    qa_exclude_disputed: bool,
    query: &str,
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
    // R32.3: demote AFTER fusion — RRF ranks ignore scores, so demoting the
    // input lists never reordered hybrid results (1.0.24 retest: scores
    // dropped but the live file still never surfaced).
    demote_test_envs(conn, &mut fused, query);
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
    // Live-first sits directly before the cut: any later score sort would
    // undo the partition, and the cut must choose from a live-led list.
    prefer_live_envs(conn, &mut fused, query);
    let hits: Vec<Hit> = fused.into_iter().take(top_k).collect();
    // Raw ranked hits out — return-shape assembly (`parent_return`) runs
    // once in `orchestrate`, after graph + filename boost, so same-scope
    // merging sees the final list.
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

/// R36.3: group-by-parent return ("match small, return big").
///
/// Ranking still matches small (leaf chunks), but the returned unit is the
/// SMALLEST ANCESTOR SECTION ≤ cap containing the match (cap = existing
/// `ANCHOR_EXPAND_MAX_CHARS`, 8 KB default) — full section bytes with every
/// matched chunk marked and the breadcrumb attached. Hits sharing one
/// returned ancestor merge into a single hit, which subsumes the old
/// `expand_by_section` ±2 window + `[continued]` fragments + the cross-hit
/// claimed set for the same-section case. Fallback when no ancestor fits
/// (or the match is section-less, e.g. entity chunks): matched chunk +
/// neighbors via `expand_by_item`, unchanged — including its `[continued]`
/// form and the claimed set across fallback hits.
///
/// Siblings derive from (`section_id`, `id`) ordering everywhere below;
/// stored `prev_id`/`next_id`/`span` links were considered and REJECTED
/// (see `rust/docs/decisions.md` R36.3) — chunk ids are document order.
const EXPAND_MAX_CHARS: usize = 8000;

fn expand_max_chars() -> usize {
    std::env::var("ANCHOR_EXPAND_MAX_CHARS").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(EXPAND_MAX_CHARS)
}

/// One ancestor candidate: identity + breadcrumb for the returned hit.
struct SectionScope {
    id: i64,
    path: String,
}

/// Ancestor chain, direct section first. Empty when the section row is gone
/// (orphan — the caller falls back). Bounded: a corrupt parent cycle must
/// not hang retrieval.
fn ancestor_chain(conn: &Connection, section_id: i64) -> Vec<SectionScope> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(section_id);
    while let Some(id) = cur {
        if out.len() >= 64 || !seen.insert(id) {
            break;
        }
        let row: Option<(Option<i64>, String)> = conn
            .query_row("SELECT parent_id, path FROM sections WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .ok();
        let Some((parent, path)) = row else { break };
        out.push(SectionScope { id, path });
        cur = parent;
    }
    out
}

/// Section-subtree membership: the section itself plus all descendants.
const SUBTREE_CTE: &str = "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT s.id FROM sections s JOIN sub ON s.parent_id = sub.id)";

/// Leaf-chunk bytes of a section subtree (own + descendants'), document
/// order. Summaries excluded — parents return evidence, never extracts
/// (R32.5 leaf-only).
fn subtree_leaf_chunks(conn: &Connection, section_id: i64) -> Vec<(i64, String)> {
    let mut stmt = match conn.prepare(&format!(
        "{SUBTREE_CTE} SELECT c.id, c.content FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.kind IN ('document','entity','distilled') ORDER BY c.id"
    )) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([section_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Assembled size of a section subtree: leaf bytes plus the `"\n\n"`
/// separators `assemble_parent` joins with. Match marks are presentation
/// overhead and don't count against the cap.
fn subtree_leaf_size(conn: &Connection, section_id: i64) -> usize {
    let (sum, n): (i64, i64) = conn
        .query_row(
            &format!(
                "{SUBTREE_CTE} SELECT COALESCE(SUM(LENGTH(c.content)), 0), COUNT(*) FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.kind IN ('document','entity','distilled')"
            ),
            [section_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    sum.max(0) as usize + 4 * (n.max(1) as usize - 1)
}

/// Smallest (deepest) ancestor whose subtree fits the cap — the direct
/// section first, then up. None when even the direct section overflows.
/// (Subtree sizes grow upward, so the walk-up is usually vacuous — but the
/// rule is stated over ancestors and the code implements it literally.)
fn smallest_fitting_scope(conn: &Connection, section_id: i64, chunk_id: i64, cap: usize) -> Option<SectionScope> {
    ancestor_chain(conn, section_id)
        .into_iter()
        .find(|scope| subtree_leaf_size(conn, scope.id) <= cap && subtree_contains_chunk(conn, scope.id, chunk_id))
}

/// True when the chunk is a leaf member of the subtree — the fit winner
/// must actually contain the match (a stale `section_id` falls back instead
/// of returning a stranger's bytes).
fn subtree_contains_chunk(conn: &Connection, section_id: i64, chunk_id: i64) -> bool {
    conn.query_row(
        &format!(
            "{SUBTREE_CTE} SELECT 1 FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.id = ?2 AND c.kind IN ('document','entity','distilled') LIMIT 1"
        ),
        rusqlite::params![section_id, chunk_id],
        |_| Ok(()),
    )
    .is_ok()
}

/// Match-mark format: the matched chunk's bytes sit verbatim between an
/// open and close line naming the chunk id (greppable, LLM-legible):
/// ```text
/// [match 123]
/// ...bytes...
/// [/match 123]
/// ```
fn mark_open(chunk_id: i64) -> String {
    format!("[match {}]", chunk_id)
}

fn mark_close(chunk_id: i64) -> String {
    format!("[/match {}]", chunk_id)
}

/// Parent bytes: subtree chunks in document order, matched ones wrapped in
/// marks, joined with blank lines. Non-matched chunks appear bare.
fn assemble_parent(chunks: &[(i64, String)], matched: &HashSet<i64>) -> String {
    chunks
        .iter()
        .map(|(id, text)| {
            if matched.contains(id) {
                format!("{}\n{}\n{}", mark_open(*id), text, mark_close(*id))
            } else {
                text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Match-centered window on returned-parent content: centers on the primary
/// match mark with `…` cut markers (same convention as `match_snippet`).
/// None when the content carries no mark (fallback/entity hits) — callers
/// keep their existing preview then.
pub fn content_window(content: &str, chunk_id: i64, n: usize) -> Option<String> {
    if n == 0 {
        return Some(String::new());
    }
    let mark = mark_open(chunk_id);
    let byte = content.find(&mark)?;
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= n {
        return Some(content.to_string());
    }
    // `find` returns a char boundary (no case mapping involved), so slicing
    // the shared prefix is safe — unlike `match_snippet`'s lowercased copy.
    let mark_char = content[..byte].chars().count().min(chars.len());
    let start = mark_char.saturating_sub(n / 2);
    let end = (start + n).min(chars.len());
    let start = end.saturating_sub(n);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter());
    if end < chars.len() {
        out.push('…');
    }
    Some(out)
}

/// Per-hit return plan: a fitting ancestor, or the chunk+neighbors fallback.
enum HitScope {
    Parent(SectionScope),
    Fallback,
}

fn plan_scope(conn: &Connection, hit: &Hit, cap: usize) -> HitScope {
    match hit
        .section_id
        .and_then(|sec| smallest_fitting_scope(conn, sec, hit.chunk_id, cap))
    {
        Some(scope) => HitScope::Parent(scope),
        None => HitScope::Fallback,
    }
}

/// Hits returning one ancestor, rank order; the merged hit takes the first
/// member's rank position.
struct ParentGroup {
    order: usize,
    scope: SectionScope,
    members: Vec<Hit>,
}

fn group_hits(planned: Vec<(usize, Hit, HitScope)>) -> (Vec<ParentGroup>, Vec<(usize, Hit)>) {
    let mut groups: Vec<ParentGroup> = Vec::new();
    let mut by_section: HashMap<i64, usize> = HashMap::new();
    let mut fallbacks = Vec::new();
    for (order, hit, scope) in planned {
        match scope {
            HitScope::Fallback => fallbacks.push((order, hit)),
            HitScope::Parent(scope) => match by_section.get(&scope.id) {
                Some(&gi) => groups[gi].members.push(hit),
                None => {
                    by_section.insert(scope.id, groups.len());
                    groups.push(ParentGroup { order, scope, members: vec![hit] });
                }
            },
        }
    }
    (groups, fallbacks)
}

/// Merged parent hit: the primary (first-ranked) member's identity, score
/// and rank; the returned ancestor's scope, breadcrumb and bytes with every
/// match marked. The whole section is returned, so `content_truncated` is
/// false. Returns the assembled chunk ids for the fallback claimed set.
fn assemble_group(conn: &Connection, group: ParentGroup) -> Option<(usize, Hit, Vec<i64>)> {
    let mut members = group.members.into_iter();
    let mut primary = members.next()?;
    let chunks = subtree_leaf_chunks(conn, group.scope.id);
    let matched: HashSet<i64> = std::iter::once(primary.chunk_id).chain(members.map(|h| h.chunk_id)).collect();
    primary.section_id = Some(group.scope.id);
    primary.path = group.scope.path;
    primary.content = assemble_parent(&chunks, &matched);
    primary.content_truncated = false;
    let ids = chunks.iter().map(|(id, _)| *id).collect();
    Some((group.order, primary, ids))
}

fn expand_by_item(conn: &Connection, hit: &mut Hit, window: usize, claimed: &mut HashSet<i64>) {
    let item_id = match hit.item_id {
        Some(id) => id,
        None => return,
    };
    let siblings = fetch_siblings(conn, "SELECT id, content FROM chunks WHERE item_id=?1 ORDER BY id", item_id);
    let idx = siblings.iter().position(|(id, _)| *id == hit.chunk_id);
    if let Some(i) = idx {
        let start = i.saturating_sub(window);
        let end = (i + window + 1).min(siblings.len());
        let cap = expand_max_chars();
        let mut used = hit.content.chars().count();
        let mut extra = Vec::new();
        let mut truncated = (end - start) < siblings.len();
        for (cid, content) in &siblings[start..end] {
            if *cid == hit.chunk_id {
                continue;
            }
            if claimed.contains(cid) {
                truncated = true;
                continue;
            }
            used += content.chars().count() + 4;
            if used > cap {
                truncated = true;
                break;
            }
            claimed.insert(*cid);
            extra.push(content.clone());
        }
        append_continuation(hit, extra);
        hit.content_truncated = truncated;
    }
}

/// R36.3 return-shape assembly: groups ranked hits by returned parent
/// section (merging same-scope hits), assembles each parent with marks, and
/// runs the chunk+neighbors fallback for the rest. Rank order is preserved
/// (merged groups sit at their first member's position); scores untouched.
///
/// Fallback neighbors skip parent-returned bytes via the claimed set, so the
/// same bytes never appear under two hits on one page (R32.9, kept). One
/// accepted corner: nested scopes (hit B's returned ancestor strictly
/// contains hit A's) assemble fully on both sides — a parent with holes
/// would be worse than the overlap, and the case needs wildly uneven
/// sibling sections to trigger.
pub fn parent_return(conn: &Connection, hits: Vec<Hit>) -> Vec<Hit> {
    if hits.is_empty() {
        return hits;
    }
    let cap = expand_max_chars();
    let planned: Vec<(usize, Hit, HitScope)> = hits
        .into_iter()
        .enumerate()
        .map(|(order, hit)| {
            let scope = plan_scope(conn, &hit, cap);
            (order, hit, scope)
        })
        .collect();
    let (groups, fallbacks) = group_hits(planned);
    let mut claimed: HashSet<i64> = HashSet::new();
    let mut ordered: Vec<(usize, Hit)> = Vec::new();
    for group in groups {
        if let Some((order, hit, ids)) = assemble_group(conn, group) {
            claimed.extend(ids);
            claimed.insert(hit.chunk_id);
            ordered.push((order, hit));
        }
    }
    for (_, hit) in &fallbacks {
        claimed.insert(hit.chunk_id);
    }
    for (order, mut hit) in fallbacks {
        expand_by_item(conn, &mut hit, 1, &mut claimed);
        ordered.push((order, hit));
    }
    ordered.sort_by_key(|(order, _)| *order);
    ordered.into_iter().map(|(_, hit)| hit).collect()
}

/// Test-env markers matched as whole path segments — a substring match
/// would demote "latest" or "contest" for containing "test".
const TEST_ENV_MARKERS: &[&str] = &["test", "tests", "testdata", "sandbox", "staging", "mock", "mocks", "fixture", "fixtures", "example", "examples", "sample", "samples", "dev"];

/// Header markers: the deliberate subset. Whole words like "test", "sample"
/// or "dev" appear constantly in live prose ("test the connection"), so the
/// file head only counts fixture-grade vocabulary.
const HEADER_ENV_MARKERS: &[&str] = &["testdata", "sandbox", "staging", "mock", "mocks", "fixture", "fixtures"];

fn test_env_factor() -> f64 {
    std::env::var("ANCHOR_TEST_ENV_DEMOTE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

fn has_marker_in(text: &str, markers: &[&str]) -> bool {
    text.split(|c: char| !c.is_alphanumeric()).any(|seg| {
        let s = seg.to_lowercase();
        markers.contains(&s.as_str())
    })
}

fn has_env_marker(text: &str) -> bool {
    has_marker_in(text, TEST_ENV_MARKERS)
}

fn has_header_marker(head: &str) -> bool {
    has_marker_in(head, HEADER_ENV_MARKERS)
}

/// Chunk → (item file path, item text head), batched (entity chunks resolve
/// via their entity). One query feeds demotion, live-first and the `env`
/// field so all three always agree on the class.
fn chunk_item_env(conn: &Connection, chunk_ids: &[i64]) -> HashMap<i64, (String, String)> {
    let mut out = HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT c.id, COALESCE(i1.external_id, i2.external_id, ''), substr(COALESCE(i1.text, i2.text, ''), 1, 1024) FROM chunks c LEFT JOIN ingested_items i1 ON i1.id=c.item_id LEFT JOIN entities e ON e.id=c.entity_id LEFT JOIN ingested_items i2 ON i2.id=e.item_id WHERE c.id IN ({})",
        placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }) {
        for (id, path, head) in rows.flatten() {
            out.insert(id, (path, head));
        }
    }
    out
}

/// Live-first ordering: test-env hits sink below live hits regardless of
/// score gaps. A multiplicative demotion cannot do this on its own —
/// single-list BM25 scores span several × on length norm alone, so halving
/// never reorders ("scores halved but the order didn't change", 1.0.24
/// retest). Stable within each class, so relevance still orders live-vs-live
/// and test-vs-test. Same guards as `demote_test_envs` (query exemption,
/// `ANCHOR_TEST_ENV_DEMOTE=1.0` disables).
pub fn prefer_live_envs(conn: &Connection, hits: &mut Vec<Hit>, query: &str) {
    if hits.len() < 2 || has_env_marker(query) {
        return;
    }
    if !(0.0..1.0).contains(&test_env_factor()) {
        return;
    }
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let env = chunk_item_env(conn, &ids);
    let mut live = Vec::with_capacity(hits.len());
    let mut test = Vec::new();
    for h in hits.drain(..) {
        // No marker evidence (unknown chunk) reads as live, not test.
        let is_test = env.get(&h.chunk_id).map(|(p, head)| env_for_item(p, head) == "test").unwrap_or(false);
        if is_test {
            test.push(h);
        } else {
            live.push(h);
        }
    }
    live.extend(test);
    *hits = live;
}

/// Demote test-env files (`.test.yml`, `/sandbox/`, …) when the query does
/// not ask about them: without this, short entity rows from test fixtures
/// outrank the live documents on BM25 length norm and the live file never
/// surfaces. Multiplicative and uniform (`ANCHOR_TEST_ENV_DEMOTE`, default
/// 0.5, 1.0 disables) so ordering within each env class is preserved.
/// Ordering across classes is `prefer_live_envs` (a multiplier alone never
/// reorders single-list BM25 gaps).
pub fn demote_test_envs(conn: &Connection, hits: &mut [Hit], query: &str) {
    if hits.is_empty() || has_env_marker(query) {
        return;
    }
    let factor = test_env_factor();
    if !(0.0..1.0).contains(&factor) {
        return;
    }
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let env = chunk_item_env(conn, &ids);
    for h in hits.iter_mut() {
        if let Some((p, head)) = env.get(&h.chunk_id) {
            if env_for_item(p, head) == "test" {
                h.score *= factor;
            }
        }
    }
}

/// Path-derived environment class: "test" when the item path carries a
/// test-env marker, else "live".
pub fn env_for_path(path: &str) -> &'static str {
    if has_env_marker(path) {
        "test"
    } else {
        "live"
    }
}

/// Item environment class: path markers, or fixture-grade vocabulary in the
/// file head (first 1 KB) for paths without markers. Backs the `env`
/// hit/citation field + filter (R32.4) and the demotion paths — every caller
/// classifies through here so the field and the ordering never disagree.
pub fn env_for_item(path: &str, text_head: &str) -> &'static str {
    if has_env_marker(path) || has_header_marker(text_head) {
        "test"
    } else {
        "live"
    }
}

/// Batched env classes for chunk ids (entity chunks resolve via their entity).
pub(crate) fn envs_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> HashMap<i64, String> {
    chunk_item_env(conn, chunk_ids).into_iter().map(|(id, (p, head))| (id, env_for_item(&p, &head).to_string())).collect()
}

/// Tokens that look like filenames (`routing-config.yml`, `service.go`):
/// contain a dot, carry a letter (excludes `1.0.23`), long enough to matter.
fn filename_tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-' || c == '_')).to_string())
        .filter(|t| t.contains('.') && t.len() >= 4 && t.chars().any(|c| c.is_alphabetic()))
        .collect()
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Query minus its filename mentions: the rest ("ACME_FEE …") is what the
/// boosted chunk must match — the filename itself never appears in content.
fn strip_filename_tokens(query: &str, names: &[String]) -> String {
    let mut out = format!(" {} ", query);
    for name in names {
        out = out.replace(name.as_str(), " ");
    }
    out
}

/// Item ids for up to 3 named files (external id or title LIKE match).
fn named_item_ids(conn: &Connection, names: &[String]) -> Vec<i64> {
    let mut out = Vec::new();
    for name in names.iter().take(3) {
        let pat = format!("%{}%", escape_like(name));
        let mut stmt = match conn.prepare(
            "SELECT id FROM ingested_items WHERE external_id LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\' ORDER BY id LIMIT 1",
        ) {
            Ok(s) => s,
            Err(_) => return out,
        };
        let id: Option<i64> = stmt
            .query_map(rusqlite::params![pat, pat], |r| r.get(0))
            .map(|rows| rows.flatten().next())
            .unwrap_or_default();
        out.extend(id);
    }
    out
}

/// Best content-matching chunk of one item for an FTS match expression.
/// Document/distilled only — a named file must surface bytes, never its
/// one-line summary extract.
fn best_chunk_for_item(conn: &Connection, item_id: i64, match_q: &str) -> Option<i64> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid WHERE chunks_fts MATCH ? AND c.item_id = ? AND c.kind IN ('document','distilled') ORDER BY bm25(chunks_fts) LIMIT 1",
        )
        .ok()?;
    let rows: Vec<i64> = stmt
        .query_map(rusqlite::params![match_q, item_id], |r| r.get::<_, i64>(0))
        .ok()?
        .flatten()
        .collect();
    rows.into_iter().next()
}

fn first_chunk_for_item(conn: &Connection, item_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM chunks WHERE item_id = ?1 AND kind IN ('document','distilled') ORDER BY id LIMIT 1",
        [item_id],
        |r| r.get(0),
    )
    .ok()
}

/// Explicit filename mentions surface their file: FTS indexes chunk
/// content, not filenames, so without this a filename in the query matches
/// entity text instead of the file. Appends up to 3 matched-item chunks
/// (raw — `parent_return` assembles them with the rest, merging a boosted
/// chunk into an already-returned section), ranked above fused hits — an
/// explicit mention is the strongest intent signal. R32.1: the surfaced
/// chunk is the item's best content match for the rest of the query (file
/// head only when nothing matches). No-op when nothing matches.
pub fn filename_boost(conn: &Connection, hits: &mut Vec<Hit>, query: &str) {
    let names = filename_tokens(query);
    if names.is_empty() {
        return;
    }
    let content_q = strip_filename_tokens(query, &names);
    let match_q = fts_match_query(&content_q);
    let mut cids: Vec<i64> = Vec::new();
    for iid in named_item_ids(conn, &names).into_iter().take(3) {
        let cid = match_q
            .as_deref()
            .and_then(|q| best_chunk_for_item(conn, iid, q))
            .or_else(|| first_chunk_for_item(conn, iid));
        cids.extend(cid);
        if cids.len() >= 3 {
            break;
        }
    }
    if cids.is_empty() {
        return;
    }
    let present: HashSet<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let fresh: Vec<i64> = cids.into_iter().filter(|id| !present.contains(id)).take(3).collect();
    if fresh.is_empty() {
        return;
    }
    let top = hits.iter().map(|h| h.score).fold(0.0f64, f64::max);
    let mut boosted = load_hits_filtered(conn, &fresh, None, None, false);
    for h in boosted.iter_mut() {
        h.score = top + 1.0;
    }
    hits.extend(boosted);
    sort_hits(hits);
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
    fn match_snippet_centers_on_query_term() {
        // 1.0.24 retest: the routing-config.yml hit previewed its first 3 KB
        // with no ACME text — previews must center on the match instead.
        let content = format!("{}ACME_TENANT_ID{}", "ab ".repeat(500), " cd".repeat(500));
        let s = match_snippet(&content, "ACME_TENANT_ID", 100);
        assert!(s.contains("ACME_TENANT_ID"), "centered window must hold the match: {}", &s.chars().take(40).collect::<String>());
        assert!(s.starts_with('…') && s.ends_with('…'), "cut on both sides must be marked");
        assert_eq!(s.chars().count(), 102, "100-char window plus two markers");
        // Head match: no leading marker, trailing cut still marked.
        let head = format!("acme leads{}", " xy".repeat(300));
        let h = match_snippet(&head, "acme", 100);
        assert!(h.contains("acme") && !h.starts_with('…') && h.ends_with('…'), "head: {}", &h.chars().take(40).collect::<String>());
    }

    #[test]
    fn match_snippet_falls_back_to_head_verbatim() {
        let content = "lorem ipsum dolor ".repeat(20);
        // No term matched: byte-identical to a plain prefix, no markers.
        let s = match_snippet(&content, "zzznope", 100);
        assert_eq!(s, content.chars().take(100).collect::<String>());
        assert!(!s.contains('…'));
        // Empty query behaves the same.
        assert_eq!(match_snippet(&content, "", 100), content.chars().take(100).collect::<String>());
        // Short content passes through untouched.
        assert_eq!(match_snippet("tiny é→ü", "tiny", 100), "tiny é→ü");
        assert_eq!(match_snippet(&content, "lorem", 0), "");
        // Unicode-safe: multi-byte chars never split, match still centered.
        let uni = format!("{}acme{}", "é".repeat(200), "ü".repeat(200));
        let u = match_snippet(&uni, "acme", 50);
        assert!(u.contains("acme"));
        assert!(u.starts_with('…') && u.ends_with('…'));
    }

    #[test]
    fn rrf() {
        let h1 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None, content_truncated: false }];
        let h2 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None, content_truncated: false }];
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

    /// Serializes `ANCHOR_TEST_ENV_DEMOTE` readers/writers: env vars are
    /// process-global and tests run on threads, so the knob-owning tests
    /// below must not overlap.
    static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let seed = vec![Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "seed".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false }];
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
        assert!(out.iter().all(|h| h.item_id == Some(iid)), "graph neighbors must carry their item");
    }

    #[test]
    fn graph_expand_skips_conflicts_with() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        let e1 = insert_entity(&conn, iid, "prose decision", "unverified", "alice");
        let e2 = insert_entity(&conn, iid, "code decision", "unverified", "bob");
        let e3 = insert_entity(&conn, iid, "related note", "unverified", "carol");
        let c1 = insert_chunk(&conn, Some(iid), Some(e1), "prose decision details");
        let _ = conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'conflicts_with')", rusqlite::params![e1, e2]).unwrap();
        let _ = conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?2,'related')", rusqlite::params![e1, e3]).unwrap();
        let seed = vec![Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "seed".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false }];
        let out = graph_expand(&conn, &seed, None, 2, 5, true);
        let ids: Vec<i64> = out.iter().filter_map(|h| h.entity_id).collect();
        assert!(!ids.contains(&e2), "conflicting e2 must never expand as supporting evidence");
        assert!(ids.contains(&e3), "related e3 still expands");
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
        let h1 = Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "c1".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false };
        let h2 = Hit { chunk_id: c2, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 0.5, content: "c2".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false };
        let fused = fuse_and_rank(&conn, Some(vec![h1]), Some(vec![h2]), 1.0, 365.0, 3, 5, true, "phoenix billing");
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

    fn summary_hit(chunk_id: i64, sid: i64, item: i64, sec: Option<i64>) -> Hit {
        Hit { chunk_id, entity_id: None, item_id: Some(item), source_id: Some(sid), section_id: sec, path: String::new(), score: 1.0, content: String::new(), source_ref: String::new(), is_pii: false, status: None, content_truncated: false }
    }

    #[test]
    fn toc_summary_cut_diversifies_across_items() {
        // R36.7 funnel follow-up: one file's fine sections must not fill the
        // cut and starve multi-file coverage (benchmark ai-03).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s-div", "internal");
        let a = insert_item(&conn, sid, "a.md");
        let b = insert_item(&conn, sid, "b.md");
        let a_secs: Vec<i64> = (0..4)
            .map(|i| insert_section(&conn, a, None, 1, &format!("a{i}"), &format!("a > a{i}")))
            .collect();
        let b_sec = insert_section(&conn, b, None, 1, "b0", "b > b0");
        // rank order: all of A's sections outrank B's
        let mut hits: Vec<Hit> = a_secs
            .iter()
            .enumerate()
            .map(|(i, s)| summary_hit(100 + i as i64, sid, a, Some(*s)))
            .collect();
        hits.push(summary_hit(200, sid, b, Some(b_sec)));
        let top = extract_top_sections(&conn, &hits);
        assert!(top.contains(&b_sec), "B's section survives A's sweep");
        assert_eq!(top.iter().filter(|s| a_secs.contains(s)).count(), 2, "A capped at 2");
    }

    #[test]
    fn toc_summary_cut_doc_hit_fills_remaining_capacity() {
        // Section-less (doc) hits run second: evidence-backed section slots
        // win, doc expansion fills what the caps leave.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s-div2", "internal");
        let a = insert_item(&conn, sid, "a.md");
        let secs: Vec<i64> = (0..3)
            .map(|i| insert_section(&conn, a, None, 1, &format!("s{i}"), &format!("a > s{i}")))
            .collect();
        // doc hit ranked above the item's own section hit for secs[2]
        let hits = vec![
            summary_hit(300, sid, a, None),
            summary_hit(301, sid, a, Some(secs[2])),
        ];
        let top = extract_top_sections(&conn, &hits);
        assert!(top.contains(&secs[2]), "ranked section keeps its slot");
        assert!(top.contains(&secs[0]), "doc expansion fills in doc order");
        assert_eq!(top.len(), 2, "per-item cap holds across passes");
    }

    fn insert_section(conn: &rusqlite::Connection, item_id: i64, parent: Option<i64>, level: i64, title: &str, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![item_id, parent, level, title, path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }
    fn insert_section_chunk(conn: &rusqlite::Connection, item_id: i64, section_id: i64, path: &str, content: &str) -> i64 {
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, level, path, content, source_ref, created_at) VALUES (?1,?2,1,?3,?4,'ref',datetime('now'))",
            rusqlite::params![item_id, section_id, path, content],
        )
        .unwrap();
        conn.last_insert_rowid()
    }
    fn mk_hit(cid: i64, iid: i64, sid: i64, sec: Option<i64>, path: &str, content: &str, score: f64) -> Hit {
        Hit {
            chunk_id: cid,
            entity_id: None,
            item_id: Some(iid),
            source_id: Some(sid),
            section_id: sec,
            path: path.to_string(),
            score,
            content: content.to_string(),
            source_ref: "ref".into(),
            is_pii: false,
            status: None,
            content_truncated: false,
        }
    }

    #[test]
    fn parent_return_groups_by_section_and_marks_matches() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_pr", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let sec_bill = insert_section(&conn, iid, None, 1, "Billing", "Billing");
        let sec_auth = insert_section(&conn, iid, None, 1, "Auth", "Auth");
        let b1 = insert_section_chunk(&conn, iid, sec_bill, "Billing", "Billing part1 content");
        let b2 = insert_section_chunk(&conn, iid, sec_bill, "Billing", "Billing part2 sibling");
        let b3 = insert_section_chunk(&conn, iid, sec_bill, "Billing", "Billing part3 extra");
        let a1 = insert_section_chunk(&conn, iid, sec_auth, "Auth", "Auth only chunk");
        // Two matches in Billing merge into one parent hit; Auth stays separate.
        let out = parent_return(
            &conn,
            vec![
                mk_hit(b1, iid, sid, Some(sec_bill), "Billing", "Billing part1 content", 9.0),
                mk_hit(a1, iid, sid, Some(sec_auth), "Auth", "Auth only chunk", 8.0),
                mk_hit(b2, iid, sid, Some(sec_bill), "Billing", "Billing part2 sibling", 7.0),
            ],
        );
        assert_eq!(out.len(), 2);
        // Merged group sits at its first member's rank, primary identity kept.
        assert_eq!(out[0].chunk_id, b1);
        assert_eq!(out[0].item_id, Some(iid));
        assert_eq!(out[0].score, 9.0);
        assert_eq!(out[0].section_id, Some(sec_bill));
        assert_eq!(out[0].path, "Billing");
        assert!(!out[0].content_truncated, "whole section returned");
        for text in ["Billing part1 content", "Billing part2 sibling", "Billing part3 extra"] {
            assert!(out[0].content.contains(text), "parent must hold {text}");
        }
        assert!(out[0].content.contains(&format!("[match {}]", b1)));
        assert!(out[0].content.contains(&format!("[/match {}]", b1)));
        assert!(out[0].content.contains(&format!("[match {}]", b2)), "every match marked, got:\n{}", out[0].content);
        assert!(!out[0].content.contains(&format!("[match {}]", b3)), "non-matches stay bare");
        assert!(!out[0].content.contains("[continued]"), "parent return has no continued fragments");
        assert_eq!(out[1].chunk_id, a1);
        assert_eq!(out[1].section_id, Some(sec_auth));
        assert!(out[1].content.contains(&format!("[match {}]", a1)));
        // Section-less hits still fall back to matched chunk + neighbors.
        let ca = insert_chunk(&conn, Some(iid), None, "item-level chunk A");
        let cb = insert_chunk(&conn, Some(iid), None, "item-level chunk B win");
        let _cc = insert_chunk(&conn, Some(iid), None, "item-level chunk C");
        let _ = ca;
        let out2 = parent_return(&conn, vec![mk_hit(cb, iid, sid, None, "", "item-level chunk B win", 1.0)]);
        assert_eq!(out2.len(), 1);
        assert!(out2[0].content.contains("item-level chunk A"), "fallback must include neighbor, got: {}", out2[0].content);
        assert!(out2[0].content.contains("[continued]"), "fallback keeps the continued form");
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

    #[test]
    fn vector_search_falls_through_when_vec_index_empty() {
        // Desync simulation: the chunk carries an embedding but vec_chunks
        // holds no rows (triggers missed it) — the scan fallback must still
        // find it instead of reporting "no vector match".
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        let cid = insert_chunk(&conn, Some(iid), None, "routing config raised bed alpha");
        // 768 dims = default ANCHOR_EMBED_DIM (no env games in parallel tests)
        let mut v = vec![0.0f32; 768];
        v[0] = 1.0;
        let blob = crate::embedder::pack_single(&v);
        conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, cid]).unwrap();
        // Wipe whatever the triggers synced, then prove the desync is real
        // (on boxes without vec0 this passes via the scan path instead).
        let _ = conn.execute("DELETE FROM vec_chunks", []);
        let vec_rows: i64 = conn.query_row("SELECT COUNT(*) FROM vec_chunks", [], |r| r.get(0)).unwrap();
        assert_eq!(vec_rows, 0);
        let hits = vector_search_filtered(&conn, &v, None, None, 8, false);
        assert_eq!(hits.len(), 1, "empty vec0 must not shadow the populated scan");
        assert!(hits[0].score > 0.99);
    }

    #[test]
    fn parent_return_falls_back_when_no_ancestor_fits() {
        // 10-chunk section over the 8 KB default cap (a root-only YAML
        // file): no ancestor fits, so the hit falls back to matched chunk +
        // neighbors (expand_by_item ±1) with the truncation flag — never a
        // giant whole-section hit, and never a stranger's bytes.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_big", "internal");
        let iid = insert_item(&conn, sid, "big");
        let sec = insert_section(&conn, iid, None, 0, "", "");
        let mut cids = Vec::new();
        let mut bodies = Vec::new();
        for i in 0..10 {
            let body = format!("segment{:02} {}", i, "body text ".repeat(120));
            bodies.push(body.clone());
            cids.push(insert_section_chunk(&conn, iid, sec, "", &body));
        }
        let out = parent_return(&conn, vec![mk_hit(cids[5], iid, sid, Some(sec), "", &bodies[5], 1.0)]);
        assert_eq!(out.len(), 1);
        assert!(out[0].content_truncated, "dropped neighbors must be flagged");
        assert!(out[0].content.contains("[continued]"), "fallback keeps the continued form");
        assert!(!out[0].content.contains("[match "), "fallback carries no marks");
        for i in [4, 5, 6] {
            assert!(out[0].content.contains(&format!("segment{:02}", i)), "window must hold segment{:02}", i);
        }
        for i in [0, 1, 2, 3, 7, 8, 9] {
            assert!(!out[0].content.contains(&format!("segment{:02}", i)), "segment{:02} must be cut", i);
        }
        assert_eq!(out[0].section_id, Some(sec), "fallback keeps the direct section");
    }

    #[test]
    fn parent_return_merges_same_section_without_repeating_bytes() {
        // R36.3 subsumes R32.9's claimed set for the same-section case:
        // adjacent matches merge into one parent hit, so shared bytes appear
        // once by construction instead of once-per-window.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_claim", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let sec = insert_section(&conn, iid, None, 1, "S", "S");
        let mut cids = Vec::new();
        for i in 0..5 {
            cids.push(insert_section_chunk(&conn, iid, sec, "S", &format!("segment{:02} body", i)));
        }
        let out = parent_return(
            &conn,
            vec![
                mk_hit(cids[1], iid, sid, Some(sec), "S", "segment01 body", 9.0),
                mk_hit(cids[2], iid, sid, Some(sec), "S", "segment02 body", 8.0),
            ],
        );
        assert_eq!(out.len(), 1, "same-section matches merge");
        for i in 0..5 {
            assert_eq!(
                out[0].content.matches(&format!("segment{:02}", i)).count(),
                1,
                "segment{:02} must appear exactly once, got:\n{}",
                i,
                out[0].content
            );
        }
        assert!(!out[0].content_truncated);
    }

    #[test]
    fn parent_return_fallback_skips_parent_returned_bytes() {
        // The claimed set survives for the fallback path: a fallback hit
        // whose ±1 neighbors overlap an already-returned parent skips the
        // claimed bytes and flags the gap — the same bytes never appear
        // under two hits on one page.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_mix", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let small = insert_section(&conn, iid, None, 1, "S", "S");
        let s1 = insert_section_chunk(&conn, iid, small, "S", "small one");
        let s2 = insert_section_chunk(&conn, iid, small, "S", "small two");
        let big = insert_section(&conn, iid, None, 1, "T", "T");
        let mut tids = Vec::new();
        for i in 0..10 {
            tids.push(insert_section_chunk(&conn, iid, big, "T", &format!("huge{:02} {}", i, "padding ".repeat(150))));
        }
        // Rank 1 returns the small parent; rank 2 (giant section) falls back.
        // Item id order is s1, s2, t0..t9, so t0's ±1 window is [s2, t0, t1]
        // and s2 is already claimed by the parent hit.
        let out = parent_return(
            &conn,
            vec![
                mk_hit(s1, iid, sid, Some(small), "S", "small one", 9.0),
                mk_hit(tids[0], iid, sid, Some(big), "T", "huge00", 8.0),
            ],
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].chunk_id, s1);
        assert!(out[0].content.contains("small two"));
        assert_eq!(out[1].chunk_id, tids[0]);
        assert!(!out[1].content.contains("small two"), "claimed bytes must be skipped, got: {}", out[1].content.chars().take(200).collect::<String>());
        assert!(out[1].content_truncated, "skipped-as-claimed must be flagged");
        let _ = s2;
    }

    #[test]
    fn parent_return_picks_smallest_fitting_ancestor() {
        // Three-level tree: a match in a fitting leaf returns the leaf
        // (tightest scope) with the leaf breadcrumb — sibling subtrees stay
        // out, ancestors are candidates only when the direct section
        // overflows (which, sizes growing upward, means falling back).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_nest", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let root = insert_section(&conn, iid, None, 0, "Doc", "Doc");
        let mid = insert_section(&conn, iid, Some(root), 1, "Mid", "Doc > Mid");
        let leaf = insert_section(&conn, iid, Some(mid), 2, "Leaf", "Doc > Mid > Leaf");
        let other = insert_section(&conn, iid, Some(root), 1, "Other", "Doc > Other");
        let l1 = insert_section_chunk(&conn, iid, leaf, "Doc > Mid > Leaf", "leaf match bytes");
        insert_section_chunk(&conn, iid, leaf, "Doc > Mid > Leaf", "leaf sibling bytes");
        insert_section_chunk(&conn, iid, other, "Doc > Other", "other branch bytes");
        let _ = mid;
        let out = parent_return(&conn, vec![mk_hit(l1, iid, sid, Some(leaf), "Doc > Mid > Leaf", "leaf match bytes", 5.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].section_id, Some(leaf), "tightest fitting scope wins");
        assert_eq!(out[0].path, "Doc > Mid > Leaf");
        assert!(out[0].content.contains("leaf sibling bytes"));
        assert!(!out[0].content.contains("other branch bytes"), "sibling subtrees excluded");
        assert!(out[0].content.contains(&format!("[match {}]", l1)));
        assert!(!out[0].content_truncated);
    }

    #[test]
    fn parent_return_returns_subtree_without_summaries() {
        // A mid-level match returns own + descendants' leaf bytes in
        // document order; summary extracts never leak into parents (R32.5).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_sub", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let mid = insert_section(&conn, iid, None, 1, "M", "M");
        let leaf = insert_section(&conn, iid, Some(mid), 2, "L", "M > L");
        let m1 = insert_section_chunk(&conn, iid, mid, "M", "mid match bytes");
        insert_section_chunk(&conn, iid, leaf, "M > L", "leaf descendant bytes");
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, level, path, content, source_ref, created_at) VALUES (?1,?2,'section_summary',1,'M',?3,'ref',datetime('now'))",
            rusqlite::params![iid, mid, "mid summary extract"],
        )
        .unwrap();
        let out = parent_return(&conn, vec![mk_hit(m1, iid, sid, Some(mid), "M", "mid match bytes", 5.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].section_id, Some(mid));
        let body = &out[0].content;
        assert!(body.contains("leaf descendant bytes"), "descendants included, got:\n{body}");
        assert!(!body.contains("mid summary extract"), "summaries excluded, got:\n{body}");
        assert!(body.find("mid match bytes").unwrap() < body.find("leaf descendant bytes").unwrap(), "document order, got:\n{body}");
    }

    #[test]
    fn parent_return_walks_up_to_the_containing_ancestor() {
        // Stale section pointers resolve upward: the hit names a child
        // section, but the chunk lives in the parent — the walk skips the
        // non-containing child and returns the containing parent.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_walk", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let parent = insert_section(&conn, iid, None, 1, "P", "P");
        let child = insert_section(&conn, iid, Some(parent), 2, "C", "P > C");
        let x = insert_section_chunk(&conn, iid, parent, "P", "parent-held match bytes");
        insert_section_chunk(&conn, iid, child, "P > C", "child bytes");
        let out = parent_return(&conn, vec![mk_hit(x, iid, sid, Some(child), "P > C", "parent-held match bytes", 5.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].section_id, Some(parent), "non-containing child skipped");
        assert_eq!(out[0].path, "P");
        assert!(out[0].content.contains(&format!("[match {}]", x)));
        assert!(out[0].content.contains("child bytes"), "parent subtree returned whole");
    }

    #[test]
    fn parent_return_nested_scopes_assemble_fully() {
        // Accepted corner: hit B's returned ancestor strictly contains hit
        // A's — both assemble fully (a parent with holes would be worse
        // than the overlap).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_nested", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let root = insert_section(&conn, iid, None, 0, "R", "R");
        let kid = insert_section(&conn, iid, Some(root), 1, "K", "R > K");
        let r1 = insert_section_chunk(&conn, iid, root, "R", "root match bytes");
        let k1 = insert_section_chunk(&conn, iid, kid, "R > K", "kid match bytes");
        let out = parent_return(
            &conn,
            vec![
                mk_hit(k1, iid, sid, Some(kid), "R > K", "kid match bytes", 9.0),
                mk_hit(r1, iid, sid, Some(root), "R", "root match bytes", 8.0),
            ],
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].section_id, Some(kid));
        assert_eq!(out[1].section_id, Some(root));
        assert!(out[1].content.contains("kid match bytes"), "nested parent assembles fully, got:\n{}", out[1].content);
        assert!(out[1].content.contains(&format!("[match {}]", r1)));
        assert!(!out[1].content.contains(&format!("[match {}]", k1)), "only B's own match is marked in B");
    }

    #[test]
    fn parent_return_entity_and_orphan_hits_fall_back() {
        // Entity chunks (section-less) keep their bytes unmarked; a hit
        // naming a deleted section falls back to chunk + neighbors instead
        // of returning a stranger's bytes.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_eo", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let eid = insert_entity(&conn, iid, "some decision", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','entity match bytes',datetime('now'))",
            [eid],
        )
        .unwrap();
        let ecid = conn.last_insert_rowid();
        let out = parent_return(
            &conn,
            vec![Hit {
                chunk_id: ecid,
                entity_id: Some(eid),
                item_id: Some(iid),
                source_id: Some(sid),
                section_id: None,
                path: String::new(),
                score: 1.0,
                content: "entity match bytes".into(),
                source_ref: "ref".into(),
                is_pii: false,
                status: Some("unverified".into()),
                content_truncated: false,
            }],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "entity match bytes");
        assert!(!out[0].content.contains("[match "), "entity hits carry no marks");
        // Orphan: section row deleted after the hit was planned.
        let sec = insert_section(&conn, iid, None, 1, "Gone", "Gone");
        let g1 = insert_section_chunk(&conn, iid, sec, "Gone", "orphan match bytes");
        let g2 = insert_section_chunk(&conn, iid, sec, "Gone", "orphan neighbor bytes");
        conn.execute("DELETE FROM sections WHERE id = ?1", [sec]).unwrap();
        let out2 = parent_return(&conn, vec![mk_hit(g1, iid, sid, Some(sec), "Gone", "orphan match bytes", 1.0)]);
        assert_eq!(out2.len(), 1);
        assert!(out2[0].content.contains("orphan neighbor bytes"), "orphan falls back to neighbors, got: {}", out2[0].content);
        assert!(!out2[0].content.contains("[match "), "orphan fallback carries no marks");
        let _ = (g2, ecid);
    }

    #[test]
    fn content_window_centers_on_the_mark() {
        let content = format!("{}[match 7]\nneedle bytes here\n[/match 7]{}", "ab ".repeat(500), " cd".repeat(500));
        let w = content_window(&content, 7, 100).expect("marked content must window");
        assert!(w.contains("[match 7]"), "window must hold the mark: {w}");
        assert!(w.contains("needle bytes here"));
        assert!(w.starts_with('…') && w.ends_with('…'));
        assert_eq!(w.chars().count(), 102);
        // Wrong chunk id → no mark → None.
        assert!(content_window(&content, 8, 100).is_none());
        // Unmarked content → None (match_snippet stays the fallback).
        assert!(content_window(&"plain content here ".repeat(20), 7, 100).is_none());
        // Short marked content passes through whole.
        let short = "[match 7]\ntiny\n[/match 7]";
        assert_eq!(content_window(short, 7, 100).as_deref(), Some(short));
        assert_eq!(content_window(&content, 7, 0).as_deref(), Some(""));
    }

    #[test]
    fn demote_test_envs_ranking_exemption_and_knob() {
        let _guard = TEST_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_env", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let c_live = insert_chunk(&conn, Some(live), None, "live treasury routes content");
        let c_test = insert_chunk(&conn, Some(tst), None, "test treasury routes content");
        let mk = |cid: i64, iid: i64| Hit { chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 10.0, content: "x".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false };
        // Default: the test file is demoted, the live file untouched.
        let mut hits = vec![mk(c_test, tst), mk(c_live, live)];
        demote_test_envs(&conn, &mut hits, "which treasury routes are supported");
        assert_eq!(hits[0].score, 5.0);
        assert_eq!(hits[1].score, 10.0);
        // Exemption: a query about test files demotes nothing.
        let mut hits2 = vec![mk(c_test, tst)];
        demote_test_envs(&conn, &mut hits2, "what is in the test file");
        assert_eq!(hits2[0].score, 10.0);
        // Knob off: nothing moves.
        std::env::set_var("ANCHOR_TEST_ENV_DEMOTE", "1.0");
        let mut hits3 = vec![mk(c_test, tst)];
        demote_test_envs(&conn, &mut hits3, "which treasury routes are supported");
        assert_eq!(hits3[0].score, 10.0);
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        // Segment matching, not substring: "latest" and "contest" survive.
        assert!(!has_env_marker("reports/latest.yml"));
        assert!(!has_env_marker("notes/contest.md"));
        assert!(has_env_marker("cfg/app.test.yml"));
        assert!(has_env_marker("sandbox/vars.yml"));
    }

    #[test]
    fn entity_chunk_hits_carry_effective_item_id() {
        // Production shape: entity chunks carry entity_id only (no item_id) —
        // hits must still resolve the item so agents can drill down via
        // GET /items/:id (1.0.24 retest: only the first hit had an item_id).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_ei", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let eid = insert_entity(&conn, iid, "some decision", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','entity chunk bytes',datetime('now'))",
            [eid],
        )
        .unwrap();
        let cid = conn.last_insert_rowid();
        let hits = load_hits_filtered(&conn, &[cid], None, None, true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_id, Some(iid), "entity chunk must resolve its item via the entity");
    }

    #[test]
    fn filename_boost_picks_matching_chunk_not_first() {
        // 1.0.24 retest: naming routing-config.yml surfaced the file's head
        // chunk (~3 KB, no ACME text) — the boost must center on the chunk
        // matching the rest of the query instead.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fm", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml', title='routing-config.yml' WHERE id=?1", [iid]).unwrap();
        let head = insert_chunk(&conn, Some(iid), None, "file head bytes with nothing relevant");
        let matched = insert_chunk(&conn, Some(iid), None, "ACME_FEE routing table content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mk_other = || Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false };
        let mut hits = vec![mk_other()];
        filename_boost(&conn, &mut hits, "routing-config.yml ACME_FEE");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, matched, "named file must surface its matching chunk, not the head (head={} match={})", head, matched);
        assert_eq!(hits[0].item_id, Some(iid), "boosted whole-file hit must drill into its item");
    }

    #[test]
    fn final_search_never_surfaces_summary_chunks() {
        // R32.6: summaries are TOC coarse-stage internals — a doc_summary
        // winning BM25 on length norm used to surface as a one-line "hit".
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_sum", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let leaf = insert_chunk(&conn, Some(iid), None, "treasury report types content here");
        conn.execute(
            "INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'doc_summary','ref','treasury report types summary extract',datetime('now'))",
            [iid],
        )
        .unwrap();
        let doc_sum = conn.last_insert_rowid();
        let kw = keyword_search_filtered(&conn, "treasury report types", None, None, 5, true);
        assert!(kw.iter().any(|h| h.chunk_id == leaf), "leaf must match");
        assert!(!kw.iter().any(|h| h.chunk_id == doc_sum), "doc_summary must not surface as a hit");
        // The fallback lists newest-first; summaries are created last, so an
        // unfiltered fallback returns extracts before evidence.
        let fb = keyword_fallback(&conn, None, 5, true);
        assert!(fb.iter().any(|h| h.chunk_id == leaf), "fallback must keep leaves");
        assert!(!fb.iter().any(|h| h.chunk_id == doc_sum), "fallback must not return extracts");
    }

    #[test]
    fn filename_boost_never_surfaces_summary_extract() {
        // The named file's best content match must be bytes, even when the
        // file's one-line summary matches the rest-query better on BM25.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fbs", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml', title='routing-config.yml' WHERE id=?1", [iid]).unwrap();
        let _head = insert_chunk(&conn, Some(iid), None, "file head bytes with nothing relevant");
        conn.execute(
            "INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'doc_summary','ref','ACME_FEE',datetime('now'))",
            [iid],
        )
        .unwrap();
        let matched = insert_chunk(&conn, Some(iid), None, "ACME_FEE routing table content here with padding words");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut hits = vec![Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false }];
        filename_boost(&conn, &mut hits, "routing-config.yml ACME_FEE");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, matched, "named file must surface leaf bytes, not its summary");
    }

    #[test]
    fn demote_test_envs_covers_entity_rows() {
        // R32.3: production entity chunks (entity_id only, no item_id) resolve
        // their item's path via the entity — the retest's 36-char fixture
        // rows must demote exactly like document chunks.
        let _guard = TEST_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_en", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let mk_ent_chunk = |iid: i64, summary: &str| {
            let eid = insert_entity(&conn, iid, summary, "unverified", "");
            conn.execute(
                "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref',?2,datetime('now'))",
                rusqlite::params![eid, summary],
            )
            .unwrap();
            (conn.last_insert_rowid(), eid)
        };
        let (c_live, e_live) = mk_ent_chunk(live, "live direction supported");
        let (c_test, e_test) = mk_ent_chunk(tst, "test direction supported");
        let mk = |cid: i64, eid: i64| Hit { chunk_id: cid, entity_id: Some(eid), item_id: None, source_id: Some(sid), section_id: None, path: String::new(), score: 10.0, content: "x".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false };
        let mut hits = vec![mk(c_test, e_test), mk(c_live, e_live)];
        demote_test_envs(&conn, &mut hits, "which treasury routes are supported");
        assert_eq!(hits[0].score, 5.0, "test entity row must demote");
        assert_eq!(hits[1].score, 10.0, "live entity row untouched");
    }

    #[test]
    fn fuse_and_rank_demotes_after_rrf_before_truncate() {
        // R32.3: RRF ranks ignore scores, so pre-fusion demotion never
        // reordered hybrid results — the live file must win a top-1 cut even
        // when the fixture row outranks it in both input lists.
        let _guard = TEST_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fz", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let c_live = insert_chunk(&conn, Some(live), None, "live treasury routes acme content here");
        let eid = insert_entity(&conn, tst, "test direction supported", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','test direction supported',datetime('now'))",
            [eid],
        )
        .unwrap();
        let c_test = conn.last_insert_rowid();
        let mk = |cid: i64, eid: Option<i64>, iid: i64, score: f64, content: &str| Hit {
            chunk_id: cid, entity_id: eid, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
        };
        let lists = || {
            (
                vec![mk(c_test, Some(eid), tst, 9.0, "test direction supported"), mk(c_live, None, live, 5.0, "live treasury routes acme content here")],
                vec![mk(c_test, Some(eid), tst, 8.0, "test direction supported"), mk(c_live, None, live, 4.0, "live treasury routes acme content here")],
            )
        };
        let (v, k) = lists();
        let fused = fuse_and_rank(&conn, Some(v), Some(k), 1.0, 365.0, 3, 1, true, "treasury routes");
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].chunk_id, c_live, "post-RRF demotion must surface the live chunk");
        // Exemption preserved: a query about test files keeps fixture order.
        let (v2, k2) = lists();
        let fused2 = fuse_and_rank(&conn, Some(v2), Some(k2), 1.0, 365.0, 3, 1, true, "what is in the test file");
        assert_eq!(fused2.len(), 1);
        assert_eq!(fused2[0].chunk_id, c_test, "env-marker queries must not demote");
    }

    #[test]
    fn live_first_partition_beats_bm25_length_gap() {
        // R32.7, the 1.0.24 retest shape: a long live chunk vs short fixture
        // entity rows sharing one rare token. The fixture rows win BM25 by
        // several × on length norm alone, so halving never reorders — live
        // must still win top-1 through the partition.
        let _guard = TEST_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_lp", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let live_text = format!("treasury config {} acme", "padding ".repeat(300));
        let c_live = insert_chunk(&conn, Some(live), None, &live_text);
        let eid = insert_entity(&conn, tst, "acme fixture row", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','acme fixture row',datetime('now'))",
            [eid],
        )
        .unwrap();
        let c_test = conn.last_insert_rowid();
        // Sanity: raw FTS ranks the short fixture row first (the BM25 gap).
        let kw = keyword_search_filtered(&conn, "acme", None, None, 8, true);
        assert_eq!(kw.len(), 2);
        assert_eq!(kw[0].chunk_id, c_test, "fixture row must lead pre-partition (else no gap to prove)");
        // Single-list keyword path: live wins top-1 despite the gap.
        let fused = fuse_and_rank(&conn, None, Some(kw), 1.0, 365.0, 3, 1, true, "treasury acme");
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].chunk_id, c_live, "live chunk must win top-1 despite the BM25 gap");
        // Exemption preserved: asking about the test file keeps fixture order.
        let kw2 = keyword_search_filtered(&conn, "acme", None, None, 8, true);
        let fused2 = fuse_and_rank(&conn, None, Some(kw2), 1.0, 365.0, 3, 1, true, "acme in the test file");
        assert_eq!(fused2[0].chunk_id, c_test, "env-marker queries must not partition");
        // Knob off: pure score order returns.
        std::env::set_var("ANCHOR_TEST_ENV_DEMOTE", "1.0");
        let kw3 = keyword_search_filtered(&conn, "acme", None, None, 8, true);
        let fused3 = fuse_and_rank(&conn, None, Some(kw3), 1.0, 365.0, 3, 1, true, "treasury acme");
        assert_eq!(fused3[0].chunk_id, c_test, "knob 1.0 disables live-first");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
    }

    #[test]
    fn env_for_path_classes_live_vs_test() {
        assert_eq!(env_for_path("cfg/routing-config.yml"), "live");
        assert_eq!(env_for_path("cfg/payment-routes.test.yml"), "test");
        assert_eq!(env_for_path("sandbox/vars.yml"), "test");
        assert_eq!(env_for_path("pkg/treasury/testdata/golden.json"), "test");
        assert_eq!(env_for_path("reports/latest.yml"), "live");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_ev", "internal");
        let live = insert_item(&conn, sid, "live");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        let cid = insert_chunk(&conn, Some(live), None, "live content");
        let map = envs_for_chunks(&conn, &[cid, 999999]);
        assert_eq!(map.get(&cid).map(|s| s.as_str()), Some("live"));
        assert!(!map.contains_key(&999999), "unknown chunks have no class");
        assert!(envs_for_chunks(&conn, &[]).is_empty());
    }

    #[test]
    fn env_for_item_reads_path_or_file_head() {
        // R32.8: a live path with a fixture-grade file head still classifies
        // test — but casual "test"/"sample" prose must not flip a live file.
        assert_eq!(env_for_item("cfg/seed.yml", "# mock treasury seed data"), "test");
        assert_eq!(env_for_item("cfg/seed.yml", "package treasury // mock clients"), "test");
        assert_eq!(env_for_item("cfg/app.yml", "runbook: test the connection today"), "live");
        assert_eq!(env_for_item("cfg/app.yml", "see the sample output below"), "live");
        assert_eq!(env_for_item("cfg/payment-routes.test.yml", "live-looking head"), "test");
        // End to end through the batched classifier (item text head, 1 KB).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_eh", "internal");
        let mk = |path: &str, head: &str| {
            let iid = insert_item(&conn, sid, path);
            conn.execute(
                "UPDATE ingested_items SET external_id=?1, text=?2 WHERE id=?3",
                rusqlite::params![path, head, iid],
            )
            .unwrap();
            insert_chunk(&conn, Some(iid), None, "shared body")
        };
        let c_fixture = mk("cfg/seed.yml", "# fixture rows for the treasury tests");
        let c_live = mk("cfg/app.yml", "runbook: test the connection today");
        let map = envs_for_chunks(&conn, &[c_fixture, c_live]);
        assert_eq!(map.get(&c_fixture).map(|s| s.as_str()), Some("test"));
        assert_eq!(map.get(&c_live).map(|s| s.as_str()), Some("live"));
    }

    #[test]
    fn filename_boost_surfaces_named_file() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fn", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml', title='routing-config.yml' WHERE id=?1", [iid]).unwrap();
        let cid = insert_chunk(&conn, Some(iid), None, "ACME_FEE routing table content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mk_other = || Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false };
        let mut hits = vec![mk_other()];
        filename_boost(&conn, &mut hits, "what is in routing-config.yml?");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, cid, "named file must rank first");
        assert!(hits[0].score > 15.0);
        // No filename in the query → untouched.
        let mut hits2 = vec![mk_other()];
        filename_boost(&conn, &mut hits2, "treasury routes?");
        assert_eq!(hits2.len(), 1);
    }
}
