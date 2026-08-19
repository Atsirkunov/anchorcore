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
    let re = Regex::new(r"\b(who|whom|owns?|owner|responsible|expert|knows?)\b").unwrap();
    re.is_match(query)
}

/// Mirrors `answer_engine.py:71` _fts_match_query
pub fn fts_match_query(question: &str) -> Option<String> {
    let re = Regex::new(r"[a-z0-9_§\-]+").unwrap();
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
    let mut fused: HashMap<i64, Hit> = HashMap::new();
    for (hits, weight) in lists.into_iter().zip(weights) {
        for (rank, mut hit) in hits.into_iter().enumerate() {
            let rank = (rank + 1) as f64;
            let key = hit.key();
            let entry = fused.entry(key).or_insert_with(|| Hit {
                chunk_id: hit.chunk_id,
                entity_id: hit.entity_id,
                score: 0.0,
                ..hit.clone()
            });
            // keep max score contribution per doc? RRF sums
            let add = weight / (RRF_K + rank);
            // need to update score on fused entry, not hit
            entry.score += add;
            // also update hit's score for later debug (not needed)
            hit.score += add;
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
    pub is_pii: bool,
    pub status: Option<String>,
}

impl Hit {
    fn key(&self) -> i64 {
        if self.entity_id.is_none() {
            self.chunk_id
        } else {
            // graph hits may have no chunk; use negative entity id
            -self.entity_id.unwrap()
        }
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
    let re_ws = Regex::new(r"\s+").unwrap();
    let re_page = Regex::new(r"^\d{1,4}\s+").unwrap();
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
         WHERE v.embedding MATCH :q",
    );
    if let Some(ids) = source_ids {
        if !ids.is_empty() {
            let list = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND i.source_id IN ({})", list));
        }
    }
    sql.push_str(" AND k = :limit");
    let mut stmt = conn.prepare(&sql).ok()?;
    let limit = (top_k * 4) as i64;
    let rows: Result<Vec<(i64, f64)>, _> = stmt
        .query_map(
            rusqlite::named_params! { ":q": q_json, ":limit": limit },
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
        )
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

fn load_hits(
    conn: &Connection,
    ids: &[i64],
    source_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if ids.is_empty() {
        return vec![];
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT c.id, c.entity_id, c.item_id, c.content, c.is_pii, c.created_at, e.status, \
                COALESCE(c.item_id, e.item_id) as eff_item, i.source_id \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE c.id IN ({})",
        list
    );
    let mut stmt = match conn.prepare(&sql) {
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
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (cid, eid, iid, content, is_pii, _created, status, _eff, sid) = row;
        if !status_ok(status.as_deref(), qa_exclude_disputed) {
            continue;
        }
        if let Some(filter) = source_ids {
            if let Some(s) = sid {
                if !filter.contains(&s) {
                    continue;
                }
            }
        }
        out.push(Hit {
            chunk_id: cid,
            entity_id: eid,
            item_id: iid,
            source_id: sid,
            score: 0.0,
            content,
            is_pii: is_pii != 0,
            status,
        });
    }
    out
}

fn load_chunks_with_embedding(
    conn: &Connection,
    source_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    let sql = "SELECT c.id, c.entity_id, c.item_id, c.content, c.is_pii, e.status, i.source_id \
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
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<i64>>(6)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (cid, eid, iid, content, is_pii, status, sid) = row;
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
        out.push(Hit { chunk_id: cid, entity_id: eid, item_id: iid, source_id: sid, score: 0.0, content, is_pii: is_pii != 0, status });
    }
    out
}

fn get_embedding(conn: &Connection, chunk_id: i64) -> Option<Vec<u8>> {
    conn.query_row("SELECT embedding FROM chunks WHERE id = ?1", [chunk_id], |r| r.get::<_, Option<Vec<u8>>>(0))
        .ok()?
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
        let h1 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, score: 0.0, content: "".into(), is_pii: false, status: None }];
        let h2 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, score: 0.0, content: "".into(), is_pii: false, status: None }];
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
}
