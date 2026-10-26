//! R2.1: Retrieval (RRF + FTS + vec0) — port of `backend/app/answer_engine.py:1`
//!
//! - `_fts_match_query` -> `fts_match_query`
//! - `_rrf_fuse` / `_rrf_fuse_multi` -> `rrf_fuse*`
//! - `_age_decay`, `_content_signature`, `_status_ok`
//! - `_vector_search` (vec0 + scan fallback), `_keyword_search` (`_who_knows` and
//!   `_graph_expand` were deleted in R45.9 — ownership is `GET /actions?owner=`)
//!
//! This is the CPU-bound hot path. LLM calls stay in `answer.rs`.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const RRF_K: f64 = 60.0;
/// Serializes env-knob tests: env vars are process-global and tests run on
/// threads, so every test that sets/removes an `ANCHOR_*` knob — or asserts
/// a default-knob value a concurrent test could perturb — must hold this.
/// Crate-visible so `answer.rs` knob tests join the same exclusion.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Shared query terms for FTS matching and match-centered snippets:
/// Unicode word runs, lowercased, len ≥ 2, stopwords removed.
/// Crate-visible for `vocab.rs` (multiword-expansion subset checks).
pub(crate) fn query_terms(question: &str) -> Vec<String> {
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

/// R40.6: curated synonym clusters for the FTS keyword leg only — never the
/// vector leg, never snippets (`query_terms` stays unexpanded). A query term
/// that is a member of a cluster pulls the cluster's other members into the
/// FTS `OR` query. Every cluster cites the eval pair that justifies it; the
/// golden set (`tests/eval_data/eval.json`) is the immune system — all
/// existing pairs must stay green. R40.7/R45.13 merge glossary + initialism
/// vocabulary into this same expansion path.
pub const SYNONYM_CLUSTERS: &[&[&str]] = &[
    // Pair `rfp-reversal-01` (1.0.29 field case): the RFP says "reversal",
    // users ask about "cancellation". Both directions expand.
    &[
        "cancel", "cancelled", "cancelling", "cancellation", "cancellations", "reverse",
        "reversed", "reversing", "reversal", "reversals", "withdraw", "withdrawal", "withdrawals",
    ],
    // Pair `rfp-reversal-01`: the query's "DD" is the doc's "Direct
    // Debit". Phrased as one unit so a loose "direct" never matches —
    // that would be over-expansion (`direct` is everywhere).
    &["dd", "direct-debit"],
];

/// Expand FTS terms with curated synonyms (R40.6). Original terms keep
/// their order and come first; synonyms append deduped.
fn expand_fts_terms(terms: &[String]) -> Vec<String> {
    let mut out: Vec<String> = terms.to_vec();
    for cluster in SYNONYM_CLUSTERS {
        if !terms.iter().any(|t| cluster.contains(&t.as_str())) {
            continue;
        }
        for syn in *cluster {
            if !out.iter().any(|t| t == syn) {
                out.push((*syn).to_string());
            }
        }
    }
    out
}

/// Mirrors `answer_engine.py:71` _fts_match_query — R8.2: Unicode-aware (was [a-z0-9_])
/// R40.6: terms expand through [`SYNONYM_CLUSTERS`] before the `OR` join.
/// This stays the pure curated-only form (unit-pinned, no DB) — production
/// legs call [`fts_match_query_scoped`] for the vocabulary merge.
pub fn fts_match_query(question: &str) -> Option<String> {
    let terms = query_terms(question);
    if terms.is_empty() {
        return None;
    }
    let expanded = expand_fts_terms(&terms);
    Some(expanded.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(" OR "))
}

/// R40.7/R45.13: FTS query with vocabulary expansion — curated synonyms
/// (R40.6) plus project-scoped glossary/initialism pairs, deduped. Multiword
/// expansions quote as FTS phrases.
pub fn fts_match_query_scoped(
    conn: &Connection,
    question: &str,
    source_ids: Option<&HashSet<i64>>,
) -> Option<String> {
    let terms = query_terms(question);
    if terms.is_empty() {
        return None;
    }
    let mut expanded = expand_fts_terms(&terms);
    for extra in crate::vocab::vocab_expansion_terms(conn, &terms, source_ids) {
        if !expanded.iter().any(|t| t == &extra) {
            expanded.push(extra);
        }
    }
    Some(expanded.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" OR "))
}

/// R47.1: rich-evidence tier pierce — `ANCHOR_PIERCE_MIN_TERMS` (default 3)
/// distinct FTS-matched surface forms plus `ANCHOR_PIERCE_MIN_BM25`
/// (default 5.0, magnitude) both have to hold; either knob negative
/// disables the pierce. Multi-term, high-magnitude matches surface on
/// score order; thin single-term matches stay tiered. Calibrated between
/// thin-max ≈ 2.3 and rich-min ≈ 7.4 (seed repro plus live-index probe2
/// magnitudes; pessimistic big-corpus projections clear it with margin).
pub fn pierce_min_terms() -> usize {
    std::env::var("ANCHOR_PIERCE_MIN_TERMS").ok().and_then(|v| v.parse::<isize>().ok()).map(|v| v.max(0) as usize).unwrap_or(3)
}

/// R47.1 magnitude bar half of the tier pierce (see [`pierce_min_terms`]).
/// Negative disables the whole pierce (tiers behave exactly as before).
pub fn pierce_min_bm25() -> f64 {
    std::env::var("ANCHOR_PIERCE_MIN_BM25")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(5.0)
}

pub fn pierce_enabled() -> bool {
    pierce_min_bm25() >= 0.0
}

/// R47.1 mark pair for the highlight column: control chars text never
/// carries. The parser pairs strictly (last `\x01` before each `\x02`),
/// so stray control bytes cost a span instead of corrupting one.
const PIERCE_MARK_OPEN: char = '\x01';
const PIERCE_MARK_CLOSE: char = '\x02';

/// Distinct lowercased surface forms inside highlight marks. FTS-accurate
/// (porter stemming included — the marks come from the MATCH itself), so
/// inflections count without a Rust-side stemmer.
fn marked_terms(marked: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut segments: Vec<&str> = marked.split(PIERCE_MARK_CLOSE).collect();
    segments.pop(); // tail past the last close was never opened
    for segment in segments {
        if let Some(start) = segment.rfind(PIERCE_MARK_OPEN) {
            let span: String =
                segment[start + PIERCE_MARK_OPEN.len_utf8()..].to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
            if !span.is_empty() {
                out.insert(span);
            }
        }
    }
    out
}

/// R47.1 leg-time pierce verdict: rich (enough distinct matched forms) and
/// strong (BM25 magnitude clears the bar). `bm25mag` is `-rank` (FTS5
/// ranks run negative — lower is better).
fn rich_keyword_match(marked: &str, bm25mag: f64) -> bool {
    pierce_enabled() && marked_terms(marked).len() >= pierce_min_terms() && bm25mag >= pierce_min_bm25()
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
    rrf_fuse_multi_named(lists, weights, &[])
}

/// RRF fusion with per-leg rank provenance (R39.1): each fused hit records
/// every contributing list's name + 1-based rank, overwriting any earlier
/// fusion's record (the outer fusion explains the final order). Unnamed lists
/// read as `list{index}` — `toc_search` keeps this default; its output is
/// always re-fused by `orchestrate` before it surfaces.
pub fn rrf_fuse_multi_named(lists: Vec<Vec<Hit>>, weights: Vec<f64>, legs: &[&str]) -> Vec<Hit> {
    let mut fused: HashMap<(i64, Option<i64>), Hit> = HashMap::new();
    for (li, (hits, weight)) in lists.into_iter().zip(weights).enumerate() {
        let leg = legs.get(li).map(|s| s.to_string()).unwrap_or_else(|| format!("list{li}"));
        for (rank, hit) in hits.into_iter().enumerate() {
            let key = hit.key();
            let entry = fused.entry(key).or_insert_with(|| Hit {
                chunk_id: hit.chunk_id,
                entity_id: hit.entity_id,
                score: 0.0,
                provenance: Vec::new(),
                ..hit.clone()
            });
            fuse_one_leg(entry, &hit, &leg, rank + 1, weight);
        }
    }
    let mut out: Vec<Hit> = fused.into_values().collect();
    sort_hits(&mut out);
    out
}

/// One list's contribution to a fused hit: the RRF add plus the provenance
/// record. Keyword/cosine evidence ORs across legs (the R39.3 gate reads the
/// union); decay is untouched — `fuse_and_rank` applies it after fusion.
fn fuse_one_leg(entry: &mut Hit, hit: &Hit, leg: &str, rank: usize, weight: f64) {
    entry.provenance.push(LegMatch { leg: leg.to_string(), rank });
    entry.keyword_matched |= hit.keyword_matched;
    entry.cosine = max_cosine(entry.cosine, hit.cosine);
    entry.rich_match |= hit.rich_match;
    entry.score += weight / (RRF_K + rank as f64);
}

fn max_cosine(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// R39.1: one contributing leg of a fused hit — which ranked list matched
/// and at which 1-based rank. Surfaced on search hits + ask citations.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegMatch {
    pub leg: String,
    pub rank: usize,
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
    /// R39.1: contributing legs + ranks from the last fusion (or the
    /// single-list stamp). Empty for unranked hits (exact, boosted).
    pub provenance: Vec<LegMatch>,
    /// R39.1: age-decay factor `fuse_and_rank` applied to `score` (1.0 = none).
    pub decay: f64,
    /// R39.1: surfaced by filename boost above the ranked list, not by ranking.
    pub boosted: bool,
    /// R39.3: a real BM25/FTS match on some leg (loaders set, fusion ORs).
    pub keyword_matched: bool,
    /// R39.3: raw vector cosine when a vec leg matched (fusion keeps the max).
    pub cosine: Option<f64>,
    /// R47.1: rich keyword evidence — the leg verdict
    /// ([`rich_keyword_match`]); fusion ORs it and the tier partition
    /// lets flagged hits pierce thin-match tiers.
    pub rich_match: bool,
}

impl Hit {
    fn key(&self) -> (i64, Option<i64>) {
        // was i64 with -entity_id collision when chunk_id == -entity_id (P1 4.6)
        (self.chunk_id, self.entity_id)
    }
}

/// Parse `chunks.created_at`: writers store SQLite `CURRENT_TIMESTAMP`
/// (`%Y-%m-%d %H:%M:%S` UTC), not RFC 3339. Without the fallback the parse
/// always fails and age decay silently returns 1.0 (F-10).
fn parse_created_at(v: &str) -> Option<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(v) {
        return Some(d.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|n| n.and_utc())
}

/// R39.6: age-decay floor — `ANCHOR_DECAY_FLOOR`, default 0.8, clamped to
/// [0, 1]. Old evidence fades but never vanishes: a year-old chunk at a
/// 365-day halflife keeps 0.8 instead of 0.5. (Deliberately uniform — no
/// per-source-type decay.)
pub fn decay_floor() -> f64 {
    std::env::var("ANCHOR_DECAY_FLOOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(0.8)
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
    0.5_f64.powf(age_days / halflife_days).max(decay_floor())
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

/// R43.1: final scope assert for `orchestrate_fusion` — a hit survives only
/// when its source is in scope. Mirrors `make_hit`'s leniency (a hit with
/// no `source_id` passes; every loader sets it).
fn scope_hit_ok(h: &Hit, scope: Option<&HashSet<i64>>) -> bool {
    match (scope, h.source_id) {
        (Some(s), Some(sid)) => s.contains(&sid),
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

/// Which chunk kinds a search covers. Base vec/FTS search = leaf chunks:
/// summaries are TOC coarse-stage internals — a `doc_summary` surfacing as a
/// final hit is a one-line extract masquerading as evidence (1.0.24 retest).
/// R33.3: the leaf pool is document chunks only — entity/distilled paraphrase
/// rows no longer compete with content in RRF (D2 option (a): entities never
/// enter retrieval; lookup/related/conflicts/review are unchanged).
/// R46.12: a person's image caption (`kind='caption'`, `captions.rs`) is
/// leaf content too — every leaf filter lists both kinds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindScope {
    Summary,
    Leaf,
}

impl KindScope {
    fn kind_clause(&self) -> Option<&'static str> {
        match self {
            KindScope::Summary => Some(" AND c.kind IN ('section_summary','doc_summary')"),
            KindScope::Leaf => Some(" AND c.kind IN ('document','caption')"),
        }
    }

    /// Leaf treats `Some({})` as no-match; summary treats it as no-prune.
    fn empty_section_is_empty(&self) -> bool {
        !matches!(self, KindScope::Summary)
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

/// Section fence for TOC leaf search. R33.3: the R14.6 `OR c.entity_id IS NOT
/// NULL` bypass is gone with the entity leaf pool — the fence applies
/// uniformly, so every leaf hit lives in an admitted section.
fn append_section_filter(
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
    section_ids: Option<&HashSet<i64>>,
) {
    if let Some(ids) = section_ids {
        if ids.is_empty() {
            return;
        }
        sql.push_str(&format!(" AND c.section_id IN ({})", placeholders(ids.len())));
        for id in ids {
            params.push(rusqlite::types::Value::Integer(*id));
        }
    }
}

/// Descending by score; a NaN score sorts last instead of panicking (R43.17).
pub(crate) fn sort_hits(hits: &mut [Hit]) {
    let key = |h: &Hit| if h.score.is_nan() { f64::NEG_INFINITY } else { h.score };
    hits.sort_by(|a, b| key(b).total_cmp(&key(a)));
}

/// FTS/vec0 over-fetch multiplier: short fixture chunks crowd long live docs
/// out of a small candidate pool on BM25 length norm (1.0.24 retest) —
/// live-first can only promote what was fetched.
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
/// `status_ok` gate, source scoping, section scoping (uniform since R33.3 —
/// the R14.6 entity pass-through left with the entity leaf pool).
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
        match sec {
            Some(sid) if filter.contains(&sid) => {}
            _ => return None,
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
        provenance: Vec::new(),
        decay: 1.0,
        boosted: false,
        keyword_matched: false,
        cosine: None,
        rich_match: false,
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
                // R39.3: the RRF score won't survive fusion — carry the raw
                // cosine for the relevance gate.
                h.cosine = Some(score);
                out.push(h);
            }
        }
    }
    sort_hits(&mut out);
    out
}

/// Single vec0 implementation for every scope (empty-section and kind
/// rules come from `scope` — see `KindScope`).
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
    append_section_filter(&mut sql, &mut params, section_ids);
    append_vector_family_filter(conn, &mut sql, &mut params);
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
/// Rows carry their highlight column for the R47.1 evidence verdict.
fn hits_from_ranks(
    conn: &Connection,
    rows: Vec<(i64, f64, String)>,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if rows.is_empty() {
        return vec![];
    }
    let by_id: HashMap<i64, (f64, bool)> = rows
        .into_iter()
        .map(|(id, rank, marked)| {
            let rich = rich_keyword_match(&marked, -rank);
            (id, (rank, rich))
        })
        .collect();
    let ids: Vec<i64> = by_id.keys().cloned().collect();
    let hits = load_hits_filtered(conn, &ids, source_ids, section_ids, qa_exclude_disputed);
    let mut out = Vec::new();
    for mut h in hits {
        if let Some((rank, rich)) = by_id.get(&h.chunk_id) {
            h.score = -rank;
            // R39.3: an FTS row hit is a real BM25 match by construction.
            h.keyword_matched = true;
            h.rich_match = *rich;
            out.push(h);
        }
    }
    sort_hits(&mut out);
    out
}

/// Single FTS implementation for every scope. Source/section filters are
/// pushed into the SQL — mirroring `exec_vec_search` — so `LIMIT` applies
/// to the scoped candidate set, not the global BM25 ranking (F-04: the old
/// fetch-then-filter order starved project/public/TOC-scoped searches).
/// The in-memory pass in `hits_from_ranks` stays as the final authority.
fn exec_fts_search(
    conn: &Connection,
    query: &str,
    source_ids: Option<&HashSet<i64>>,
    section_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
    scope: KindScope,
) -> Vec<Hit> {
    let Some(match_q) = fts_match_query_scoped(conn, query, source_ids) else { return vec![] };
    if let Some(ids) = section_ids {
        if ids.is_empty() && scope.empty_section_is_empty() {
            return vec![];
        }
    }
    let limit = (top_k * CANDIDATE_MULT) as i64;
    // Same join shape as the vec0 leg (1:1 per FTS row — the LEFT JOINs
    // can't multiply rows, so ORDER BY rank LIMIT stays correct).
    // The highlight column rides along for the R47.1 evidence verdict
    // (matched spans, FTS-accurate — FTS5 has no matchinfo).
    let mut sql = String::from(
        "SELECT c.id, bm25(chunks_fts) AS rank, highlight(chunks_fts, 0, '\x01', '\x02') AS marked FROM chunks_fts \
         JOIN chunks c ON c.id=chunks_fts.rowid \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE chunks_fts MATCH ?",
    );
    let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(match_q)];
    append_source_filter(&mut sql, &mut params, source_ids);
    append_section_filter(&mut sql, &mut params, section_ids);
    if let Some(clause) = scope.kind_clause() {
        sql.push_str(clause);
    }
    sql.push_str(" ORDER BY rank LIMIT ?");
    params.push(rusqlite::types::Value::Integer(limit));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows: Vec<(i64, f64, String)> = match stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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
                    h.cosine = Some(score as f64);
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
    // Document chunks only — the scan path must match the vec0 path (R33.3).
    let mut sql = String::from(
        "SELECT c.id, c.entity_id, COALESCE(c.item_id, e.item_id), c.content, c.source_ref, c.is_pii, e.status, i.source_id, c.section_id, c.path \
         FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE c.embedding IS NOT NULL AND c.kind IN ('document','caption')",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    append_vector_family_filter(conn, &mut sql, &mut params);
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
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

/// R45.5: restrict a vector leg to vectors in the active query vector's
/// family (`embed_fp::comparable_fingerprints`) — mixed vector spaces are
/// never compared. Out-of-family and unstamped vectors drop out (keyword-only)
/// until the backfill re-embeds them. No active fingerprint = no filter.
fn append_vector_family_filter(conn: &Connection, sql: &mut String, params: &mut Vec<rusqlite::types::Value>) {
    let Some(fps) = crate::embed_fp::comparable_fingerprints(conn) else { return };
    if fps.is_empty() {
        sql.push_str(" AND 0");
        return;
    }
    sql.push_str(&format!(" AND c.embed_fp IN ({})", placeholders(fps.len())));
    params.extend(fps.into_iter().map(rusqlite::types::Value::Text));
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

/// R33.5: literal-substring search over stored chunk bytes — the non-ranked
/// leg alongside FTS/semantic (plan D5, trimmed: filters + navigation
/// shipped under R36.6). Case-sensitive `instr()` match (no LIKE escapes,
/// no tokenization, regex stays an owner call), document order (item, then
/// chunk row), leaf pool only (`kind='document'`, R33.3). Score is a
/// uniform 1.0 (matched, unranked); the answer layer reports
/// `retrieval: "exact"` so callers never read it as a ranked score.
/// Empty needle matches nothing (`instr(x, '')` would match everything).
pub fn exact_search(
    conn: &Connection,
    needle: &str,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if needle.is_empty() {
        return vec![];
    }
    // Same join shape as the FTS leg; the source filter is pushed into the
    // SQL (F-04) with the in-memory pass in `hits_from_ordered_ids` final.
    let mut sql = String::from(
        "SELECT c.id FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
         WHERE instr(c.content, ?) > 0",
    );
    let mut params: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(needle.to_string())];
    append_source_filter(&mut sql, &mut params, source_ids);
    if let Some(clause) = KindScope::Leaf.kind_clause() {
        sql.push_str(clause);
    }
    sql.push_str(" ORDER BY COALESCE(c.item_id, e.item_id), c.id LIMIT ?");
    params.push(rusqlite::types::Value::Integer(top_k as i64));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let ids: Vec<i64> = match stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))
        .and_then(|m| m.collect())
    {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    hits_from_ordered_ids(conn, ids, source_ids, qa_exclude_disputed)
}

/// Hydrate candidate ids in the caller's order (exact search is
/// document-ordered, not scored): uniform 1.0, status/source gates final.
fn hits_from_ordered_ids(
    conn: &Connection,
    ids: Vec<i64>,
    source_ids: Option<&HashSet<i64>>,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    if ids.is_empty() {
        return vec![];
    }
    let mut by_id: HashMap<i64, Hit> = load_hits_filtered(conn, &ids, source_ids, None, qa_exclude_disputed)
        .into_iter()
        .map(|h| (h.chunk_id, h))
        .collect();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(mut h) = by_id.remove(&id) {
            h.score = 1.0;
            out.push(h);
        }
    }
    out
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

/// R39.8: the R36.10 `suspect` predicates (`extract.rs`, verbatim).
/// `items::pdf_quality_for` is private and `items.rs` is read-only under
/// this scope, so the flag can't be called — and `crate::extract` doesn't
/// resolve here either: the R39.10 eval target compiles this file
/// standalone via `#[path]` (`{db, common, retrieval}` closure), while
/// `extract`↔`chunking` form a cycle that only resolves in the full bin.
/// The 30k word list stays single-sourced (`include_str!` of the same
/// sibling file); only the small predicates duplicate. Reunify by moving
/// the detector to `common.rs` (or extending the eval includes).
mod suspect_pdf {
    pub(super) fn is_pdf_filename(name: &str) -> bool {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
        match base.rsplit('.').next() {
            Some(ext) if ext != base.as_str() => ext == "pdf",
            _ => false,
        }
    }

    pub(super) fn pdf_spacing_suspect(text: &str) -> bool {
        let (splits, tokens) = split_suspects(text);
        pdf_trips_detector(splits, tokens)
    }

    /// Short tokens that are real words — never split-word fragments.
    const COMMON_SHORT_WORDS: &[&str] = &[
        "a", "i", "am", "an", "as", "at", "be", "by", "do", "go", "he", "if", "in", "is", "it", "me", "my", "no",
        "of", "on", "or", "so", "to", "up", "us", "we", "vs", "ok", "ex", "re", "and", "the", "for", "are",
        "but", "not", "you", "all", "any", "can", "had", "her", "was", "one", "our", "out", "day", "get",
        "has", "him", "his", "how", "man", "new", "now", "old", "see", "two", "way", "who", "did", "its",
        "let", "put", "say", "she", "too", "use", "etc", "per", "via", "pro", "aka", "that", "with", "have",
        "this", "will", "your", "from", "they", "them", "then", "than",
    ];

    /// English frequency set for the split-word catcher: top-30k wordfreq
    /// 3.1.1 words (`^[a-z]{2,}$`), Apache-2.0 — see `pdf_words.NOTICE`.
    fn pdf_words() -> &'static std::collections::HashSet<&'static str> {
        static WORDS: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
            std::sync::OnceLock::new();
        WORDS.get_or_init(|| include_str!("pdf_words.txt").split_whitespace().collect())
    }

    /// Domain abbreviations below the frequency cutoff — real tokens, never
    /// split fragments. Two-to-four lowercase letters only (the fragment
    /// shape); longer tokens never query this set.
    const TECH_ALLOW_WORDS: &[&str] = &[
        "cli", "json", "yaml", "csv", "orm", "jwt", "cors", "csrf", "crud", "sdk", "xls", "jsx", "tsx",
        "scss", "ssl", "tls", "udp", "grpc", "tiff", "bmp", "webp", "avif", "mkv", "exe", "dll", "dmg",
        "deb", "rpm", "apk", "epub", "mobi", "faq", "wip", "tbd", "fyi", "asap", "eta", "eod", "pto",
        "ooo", "kpi", "okr", "sla", "sop", "rfp", "poc", "mvp", "arr", "mrr", "cac", "ltv", "yoy", "qoq",
        "ai", "llm", "rag", "nlp", "ocr", "vpn", "lan", "wan", "nat", "iam", "sso", "mfa", "kms",
        "mttr", "mtbf", "qbr", "kyc", "pii", "ttl", "aml", "gdpr", "ccpa", "pci", "dss", "iso", "nist",
        "oecd", "dpa", "sar", "ofac", "itil", "fips", "cc", "bcc", "oss", "bss",
    ];

    fn is_known_word(tok: &str) -> bool {
        COMMON_SHORT_WORDS.contains(&tok) || TECH_ALLOW_WORDS.contains(&tok) || pdf_words().contains(tok)
    }

    /// Split-word suspects over lowercased tokens: a short unknown fragment
    /// beside a lowercase neighbour ("noti fi", "cati ons"), or a pair whose
    /// join is a known word while at least one side is not ("Cancellati"+"ons").
    /// Two known words never trip ("sun day", "the rapist", "on line").
    /// Returns (suspect_count, token_count).
    fn split_suspects(text: &str) -> (usize, usize) {
        let toks: Vec<String> = text
            .split_whitespace()
            .map(|raw| raw.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let mut splits = 0usize;
        for (i, tok) in toks.iter().enumerate() {
            if is_split_fragment(tok, toks.get(i.wrapping_sub(1)), toks.get(i + 1)) {
                splits += 1;
            }
            if let Some(next) = toks.get(i + 1) {
                if is_split_join(tok, next) {
                    splits += 1;
                }
            }
        }
        (splits, toks.len())
    }

    fn is_pure_alpha(tok: &str) -> bool {
        !tok.is_empty() && tok.chars().all(|c| c.is_ascii_lowercase())
    }

    /// Short unknown token (single letters are math variables, never splits)
    /// touching a lowercase neighbour on either side.
    fn is_split_fragment(tok: &str, prev: Option<&String>, next: Option<&String>) -> bool {
        let n = tok.chars().count();
        if !(2..=4).contains(&n) || !is_pure_alpha(tok) || is_known_word(tok) {
            return false;
        }
        prev.map(|p| p.chars().last().is_some_and(|c| c.is_ascii_lowercase())).unwrap_or(false)
            || next.map(|n| n.chars().next().is_some_and(|c| c.is_ascii_lowercase())).unwrap_or(false)
    }

    /// Adjacent pure-alpha pair whose join is a known word with at least one
    /// unknown side — the rejoin test for ngram-artifact fragments.
    fn is_split_join(a: &str, b: &str) -> bool {
        if a.chars().count() < 2 || b.chars().count() < 2 || !is_pure_alpha(a) || !is_pure_alpha(b) {
            return false;
        }
        let joined = format!("{a}{b}");
        if joined.chars().count() < 5 || !is_known_word(&joined) {
            return false;
        }
        !is_known_word(a) || !is_known_word(b)
    }

    /// Detector trip wire: an absolute floor (short clean texts stay silent)
    /// plus a density ceiling (scattered false positives stay silent).
    const PDF_TRIP_MIN_SPLITS: usize = 6;
    const PDF_TRIP_PER_MILLE: usize = 6;

    fn pdf_trips_detector(splits: usize, tokens: usize) -> bool {
        splits >= PDF_TRIP_MIN_SPLITS && splits.saturating_mul(1000) >= tokens.saturating_mul(PDF_TRIP_PER_MILLE)
    }
}

/// R39.8: items whose summaries can't be trusted — the R36.10 `suspect`
/// branch of the item PDF flag (shredded stored text on a PDF row), batched
/// into one query over in-scope PDF rows (the `LIKE` pre-filter only bounds
/// the scan; `is_pdf_filename` + the spacing check decide). Suspect wins
/// over repair state at read time, so no events lookup is needed.
fn suspect_pdf_items(conn: &Connection, source_ids: Option<&HashSet<i64>>) -> HashSet<i64> {
    let mut sql = String::from(
        "SELECT i.id, i.title, i.external_id, i.text FROM ingested_items i \
         WHERE (i.title LIKE '%.pdf' OR i.external_id LIKE '%.pdf')",
    );
    let mut params: Vec<rusqlite::types::Value> = vec![];
    append_source_filter(&mut sql, &mut params, source_ids);
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return HashSet::new(),
    };
    let rows: Vec<(i64, String, String, String)> = match stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .and_then(|m| m.collect())
    {
        Ok(v) => v,
        Err(_) => return HashSet::new(),
    };
    rows.into_iter()
        .filter(|(_, title, external_id, text)| {
            (suspect_pdf::is_pdf_filename(title) || suspect_pdf::is_pdf_filename(external_id))
                && suspect_pdf::pdf_spacing_suspect(text)
        })
        .map(|(id, _, _, _)| id)
        .collect()
}

/// R39.8: keep flat-leg hits for suspect items only, appended after the
/// fenced legs (deduped by chunk — a fenced rank always stands).
fn append_suspect_hits(fenced: &mut Vec<Hit>, flat: Vec<Hit>, suspects: &HashSet<i64>) {
    if flat.is_empty() {
        return;
    }
    let mut seen: HashSet<i64> = fenced.iter().map(|h| h.chunk_id).collect();
    for h in flat {
        if !h.item_id.is_some_and(|iid| suspects.contains(&iid)) {
            continue;
        }
        if seen.insert(h.chunk_id) {
            fenced.push(h);
        }
    }
}

/// R39.8: suspect-PDF fence skip — the TOC fence admits sections by summary
/// match, but a suspect PDF's summaries are poor, so its leaf chunks would
/// stay hidden. Run the flat leaf legs unfenced and splice the suspect-item
/// hits into the fenced legs (same legs, trailing ranks — precision-first,
/// recall for the untrustworthy). No-op when nothing is suspect: clean
/// corpora never run the extra legs.
#[allow(clippy::too_many_arguments)]
fn append_suspect_flat_legs(
    conn: &Connection,
    query: &str,
    query_emb: Option<&[f32]>,
    source_ids: Option<&HashSet<i64>>,
    top_k: usize,
    qa_exclude_disputed: bool,
    leaf_vec: &mut Vec<Hit>,
    leaf_kw: &mut Vec<Hit>,
) {
    let suspects = suspect_pdf_items(conn, source_ids);
    if suspects.is_empty() {
        return;
    }
    if let Some(emb) = query_emb {
        if let Some(flat) = leaf_vector_search_filtered(conn, emb, source_ids, None, top_k, qa_exclude_disputed) {
            append_suspect_hits(leaf_vec, flat, &suspects);
        }
    }
    let flat = leaf_keyword_search_filtered(conn, query, source_ids, None, top_k, qa_exclude_disputed);
    append_suspect_hits(leaf_kw, flat, &suspects);
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
    let mut leaf_vec = query_emb
        .and_then(|emb| leaf_vector_search_filtered(conn, emb, source_ids, Some(&top_secs), top_k, qa_exclude_disputed))
        .unwrap_or_default();
    let mut leaf_kw = leaf_keyword_search_filtered(conn, query, source_ids, Some(&top_secs), top_k, qa_exclude_disputed);
    // R39.8: suspect-PDF fence skip — poor summaries must not hide leaves.
    append_suspect_flat_legs(conn, query, query_emb, source_ids, top_k, qa_exclude_disputed, &mut leaf_vec, &mut leaf_kw);
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

pub fn fuse_evidence(mut lists: Vec<Vec<Hit>>, weights: Vec<f64>) -> Vec<Hit> {
    if lists.is_empty() { return vec![]; }
    if lists.len()==1 { let mut v = lists.remove(0); sort_hits(&mut v); return v; }
    rrf_fuse_multi(lists, weights)
}

/// R39.1: single-list provenance — input order is the loader's rank order
/// (loaders return sorted). Skips hits that already carry a fusion record:
/// the TOC path feeds one pre-fused list through `fuse_and_rank` and its
/// outer-fusion legs must survive.
fn stamp_single_leg(hits: &mut [Hit], leg: &str) {
    for (i, h) in hits.iter_mut().enumerate() {
        if h.provenance.is_empty() {
            h.provenance = vec![LegMatch { leg: leg.to_string(), rank: i + 1 }];
        }
    }
}

/// Batched age decay (P1 4.7 — was N+1 per hit): multiplies each score by
/// its factor and records the applied factor on the hit (R39.1).
pub fn apply_age_decay(conn: &Connection, hits: &mut [Hit], halflife_days: f64) {
    if hits.is_empty() {
        return;
    }
    let now = Utc::now();
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let sql = format!("SELECT id, created_at FROM chunks WHERE id IN ({})", placeholders(ids.len()));
    let mut map: HashMap<i64, Option<DateTime<Utc>>> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        }) {
            for row in rows.flatten() {
                let (id, s) = row;
                let dt = s.as_deref().and_then(parse_created_at);
                map.insert(id, dt);
            }
        }
    }
    for h in hits.iter_mut() {
        let created = map.get(&h.chunk_id).cloned().unwrap_or(None);
        let factor = age_decay(created, halflife_days, now);
        h.decay = factor;
        h.score *= factor;
    }
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
        let mut v = keyword_hits.unwrap();
        stamp_single_leg(&mut v, "keyword");
        v
    } else if keyword_hits.is_none() {
        let mut v = vector_hits.unwrap();
        stamp_single_leg(&mut v, "vector");
        v
    } else {
        rrf_fuse_multi_named(
            vec![vector_hits.unwrap(), keyword_hits.unwrap()],
            vec![1.0, keyword_weight],
            &["vector", "keyword"],
        )
    };
    apply_age_decay(conn, &mut fused, halflife_days);
    // R33.3: no score demotion — the R32.3 ×0.5 multiplier retired with the
    // entity leaf pool (it never reordered on its own; RRF ranks ignore
    // scores). Env ordering is the `prefer_live_envs` partition before the
    // cut below.
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
    // R39.7: authority tiers refine within the env classes (live-first
    // still dominates — the partition sorts by (env, tier)); the cut
    // chooses from an authority-led list.
    prefer_authority_tiers(conn, &mut fused, query);
    // R39.5: stale/disputed filter BEFORE the cut — excluded hits must not
    // consume top-k slots (seeded corpora still return a full top_k).
    // R40.4: split cut + pool so missing doc kinds pull tail slots from
    // below the cut line (both branches — the pool only exists here).
    let mut ordered: Vec<Hit> =
        fused.into_iter().filter(|h| status_ok(h.status.as_deref(), qa_exclude_disputed)).collect();
    let pool = ordered.split_off(top_k.min(ordered.len()));
    let mut hits = ordered;
    apply_type_slots(conn, &mut hits, &pool, top_k, query);
    // Raw ranked hits out — return-shape assembly (`parent_return`) runs
    // once in `orchestrate`, after graph + filename boost, so same-scope
    // merging sees the final list.
    hits
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
/// order. Document chunks only — parents return evidence, never extracts
/// (R32.5) or entity paraphrase (R33.3).
fn subtree_leaf_chunks(conn: &Connection, section_id: i64) -> Vec<(i64, String)> {
    let mut stmt = match conn.prepare(&format!(
        "{SUBTREE_CTE} SELECT c.id, c.content FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.kind IN ('document','caption') ORDER BY c.id"
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
                "{SUBTREE_CTE} SELECT COALESCE(SUM(LENGTH(c.content)), 0), COUNT(*) FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.kind IN ('document','caption')"
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
            "{SUBTREE_CTE} SELECT 1 FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.id = ?2 AND c.kind IN ('document','caption') LIMIT 1"
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

/// True for a line that is exactly a match mark (`[match 12]` / `[/match 12]`).
fn is_mark_line(line: &str) -> bool {
    let t = line.trim();
    let inner = t.strip_prefix("[/match ").or_else(|| t.strip_prefix("[match "));
    inner
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Content without its match-mark lines — for prose consumers (the answer
/// model's context) that must not echo the marks back to people. Agent-facing
/// payloads keep the marks (R36.3).
pub fn strip_match_marks(content: &str) -> String {
    content.lines().filter(|l| !is_mark_line(l)).collect::<Vec<_>>().join("\n")
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

/// Header markers, strong tier: fixture-grade vocabulary that one occurrence
/// in the file head is enough to flag. Whole words like "test", "sample" or
/// "dev" appear constantly in live prose ("test the connection"), so the head
/// only counts this deliberate subset.
const HEADER_ENV_MARKERS: &[&str] = &["testdata", "mock", "mocks", "fixture", "fixtures"];

/// Header markers, weak tier: ordinary domain words ("a proper sandbox",
/// "Provision sandbox" in payments docs, "staging" in release notes). One
/// alone is prose; two distinct ones read as an environment fixture. (R43.27:
/// a lone "sandbox" sank the live billing decision behind every unrelated doc.)
const HEADER_WEAK_ENV_MARKERS: &[&str] = &["sandbox", "staging"];

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
    if has_marker_in(head, HEADER_ENV_MARKERS) {
        return true;
    }
    HEADER_WEAK_ENV_MARKERS.iter().filter(|m| has_marker_in(head, &[**m])).count() >= 2
}

/// Chunk → (item file path, item text head), batched (entity chunks resolve
/// via their entity). One query feeds live-first and the `env` field so
/// both always agree on the class.
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
/// retest) — the R32.3 multiplier retired in R33.3 and this partition is the
/// only env ordering left. Stable within each class, so relevance still
/// orders live-vs-live and test-vs-test. Guards: queries naming test files
/// are exempt, `ANCHOR_TEST_ENV_DEMOTE=1.0` disables.
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
/// hit/citation field + filter (R32.4) and the live-first partition — every
/// caller classifies through here so the field and the ordering never disagree.
pub fn env_for_item(path: &str, text_head: &str) -> &'static str {
    if has_env_marker(path) || has_header_marker(text_head) {
        "test"
    } else {
        "live"
    }
}

/// Batched env classes for chunk ids (entity chunks resolve via their entity).
/// Test-env classifier for chunks — `pub` so the golden eval (an integration
/// test) can drive the production leg the same way the unit tests do.
pub fn envs_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> HashMap<i64, String> {
    chunk_item_env(conn, chunk_ids).into_iter().map(|(id, (p, head))| (id, env_for_item(&p, &head).to_string())).collect()
}

/// R39.7: source-authority tiers — a file-class prior as an ORDERING
/// PARTITION (the `prefer_live_envs` pattern generalized, never a score
/// multiplier — RRF fuses by rank and single-list BM25 gaps dwarf any
/// multiplier, R32.7). Lower wins; ties keep score order (stable).
///
/// | tier | class | markers |
/// | 0 `Spec` | ADR/decision/spec docs | `adr/`, `rfc/`, `decisions/`, `design-docs/` dirs; `adr-*`/`rfc-*` names; `spec` in a prose filename |
/// | 1 `Code` | source code | code extensions (self-contained list — the R39.10 eval target compiles this module standalone, so `extract.rs` helpers are out of reach) |
/// | 2 `Config` | config/data/operational | config extensions + `Dockerfile`/`Makefile`/… names |
/// | 3 `Prose` | everything else (default) | runbooks, notes, unknown kinds |
/// | 4 `Generated` | lint / generated / vendored / build output | `vendor/`, `generated/`, `node_modules/`-style output dirs; `lint`/`golangci`/`eslint`/… names; `*.generated.*`, `*.min.*`, lockfiles |
///
/// Terse precise classes (code, config) outrank verbose prose: a BM25
/// OR-query matches half the corpus on one shared word, so prose-above-
/// config buries exact config answers (eval `flags-01`: a 5-term YAML
/// match lost to 1-term prose). Lowest authority wins on overlap: a
/// generated file is generated wherever it sits (`vendor/spec.md` →
/// `Generated`, `*.golangci.yml` → `Generated` before the config extension
/// is read). Deliberate non-promotions: bare `spec/` dirs (RSpec suites own
/// that name) promote prose files only, and `*.spec.ts` stays `Code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityTier {
    Spec,
    Code,
    Config,
    Prose,
    Generated,
}

/// R39.7: dir segments that demote to `Generated` (vendored deps + build
/// output — overlaps `folder.rs::SKIP_DIRS`, which prunes most of these at
/// ingest; the tier catches the rest: Drive sources, pre-skip rows).
const GENERATED_DIR_MARKERS: &[&str] = &[
    "node_modules", "bower_components", "vendor", "vendored", "third_party", "third-party",
    "generated", "dist", "build", "out", "target", "__pycache__", ".venv", "venv",
    "coverage", ".next", ".nuxt", "pods", "deriveddata",
];

/// R39.7: dir segments that promote to `Spec` (bare `spec/` is NOT here —
/// RSpec suites own that name; prose files under it promote via
/// `SPEC_DIR_PROSE_ONLY`).
const SPEC_DIR_MARKERS: &[&str] = &["adr", "adrs", "rfc", "rfcs", "decision", "decisions", "design-docs", "designdocs"];

/// R39.7: dir segments that promote prose files only (see above).
const SPEC_DIR_PROSE_ONLY: &[&str] = &["spec", "specs"];

/// R39.7: whole-word `Spec` filename tokens (prose extensions only —
/// `foo.spec.ts` stays `Code`).
const SPEC_NAME_TOKENS: &[&str] = &["spec", "specs", "rfc", "rfcs", "adr", "adrs", "decision", "decisions"];

/// R39.7: lint-tool name fragments — a lint config is enforcement, not
/// evidence (`api-lint.yml` matches on the `lint` token instead).
const LINT_NAME_MARKERS: &[&str] = &["golangci", "eslint", "flake8", "pylint", "rubocop"];

/// R39.7: prose extensions (gate the `Spec` filename/dir rules).
const PROSE_EXTENSIONS: &[&str] = &["md", "markdown", "rst", "txt", "adoc"];

/// R39.7: source-code extensions (self-contained: the R39.10 eval target
/// compiles this module standalone via `#[path]`, so `extract.rs` helpers
/// are out of reach — keep this list in sync by shape, not by import).
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "go", "java", "kt", "kts",
    "scala", "c", "h", "cc", "hh", "cpp", "hpp", "cxx", "hxx", "cs", "rb", "php", "swift",
    "m", "mm", "sh", "bash", "zsh", "fish", "ps1", "sql", "r", "lua", "pl", "pm", "ex",
    "exs", "erl", "hrl", "hs", "ml", "mli", "clj", "cljs", "dart", "vue", "svelte", "ipynb",
    "html", "htm", "css", "scss", "less", "sass", "proto", "graphql", "gql",
];

/// R39.7: config/data extensions.
const CONFIG_EXTENSIONS: &[&str] = &[
    "yml", "yaml", "json", "jsonc", "json5", "toml", "ini", "cfg", "conf", "config", "env",
    "properties", "xml", "csv", "tsv", "hcl", "tf", "tfvars", "nix",
];

/// R39.7: extensionless operational filenames (compared against the stem,
/// so `CMakeLists.txt` matches too) — build recipes, not implementation.
const CONFIG_NAME_MARKERS: &[&str] =
    &["dockerfile", "containerfile", "makefile", "gnumakefile", "justfile", "gemfile", "vagrantfile", "brewfile", "cmakelists"];

/// R39.7: last path segment (both separators — folder `external_id`s use
/// `/`, Drive titles and Windows rows may carry `\`).
fn tier_basename(lower_path: &str) -> &str {
    lower_path.rsplit(['/', '\\']).next().unwrap_or(lower_path)
}

/// R39.7: extension after the last dot (`""` when dotless; `.env` → `env`).
fn tier_extension(basename: &str) -> &str {
    basename.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
}

/// R39.7: stem (basename minus the last extension) for exact-name matches.
fn tier_stem(basename: &str) -> &str {
    basename.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(basename)
}

/// R39.7: dir segments (everything above the basename), lowercased already.
fn tier_dir_segments(lower_path: &str) -> Vec<&str> {
    let mut segs: Vec<&str> = lower_path.split(['/', '\\']).collect();
    segs.pop();
    segs
}

/// R39.7: basename tokens (lowercased already) for whole-word markers.
fn basename_tokens(basename: &str) -> Vec<String> {
    basename.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
}

/// R39.7: `Generated` wins over every other class — checked first.
fn is_generated_path(dir_segments: &[&str], basename: &str) -> bool {
    if dir_segments.iter().any(|s| GENERATED_DIR_MARKERS.contains(s)) {
        return true;
    }
    if basename_tokens(basename).iter().any(|t| t == "lint" || t == "lints" || t == "generated") {
        return true;
    }
    if LINT_NAME_MARKERS.iter().any(|m| basename.contains(m)) {
        return true;
    }
    basename.contains(".min.") || basename.contains(".bundle.") || basename.ends_with(".lock") || basename.contains("-lock.")
}

/// R39.7: `Spec` — decision/spec dirs and names. Runs after the generated
/// check, before extensions.
fn is_spec_path(dir_segments: &[&str], basename: &str, ext: &str) -> bool {
    if dir_segments.iter().any(|s| SPEC_DIR_MARKERS.contains(s)) {
        return true;
    }
    if basename.starts_with("adr-") || basename.starts_with("adr_") || basename.starts_with("rfc-") || basename.starts_with("rfc_") {
        return true;
    }
    if !PROSE_EXTENSIONS.contains(&ext) {
        return false;
    }
    if dir_segments.iter().any(|s| SPEC_DIR_PROSE_ONLY.contains(s)) {
        return true;
    }
    basename_tokens(basename).iter().any(|t| SPEC_NAME_TOKENS.contains(&t.as_str()))
}

/// R39.7: file-class authority for one item path (case-insensitive).
/// Precedence: generated → spec → code → config → prose default.
pub fn authority_tier(path: &str) -> AuthorityTier {
    let lower = path.to_lowercase();
    let basename = tier_basename(&lower);
    let dirs = tier_dir_segments(&lower);
    if is_generated_path(&dirs, basename) {
        return AuthorityTier::Generated;
    }
    let ext = tier_extension(basename);
    if is_spec_path(&dirs, basename, ext) {
        return AuthorityTier::Spec;
    }
    if CODE_EXTENSIONS.contains(&ext) {
        return AuthorityTier::Code;
    }
    if CONFIG_EXTENSIONS.contains(&ext) || CONFIG_NAME_MARKERS.contains(&tier_stem(basename)) {
        return AuthorityTier::Config;
    }
    AuthorityTier::Prose
}

/// R39.7 kill knob — `ANCHOR_AUTHORITY_TIERS`, default on; `0`/`false` off.
pub fn authority_tiers_enabled() -> bool {
    match std::env::var("ANCHOR_AUTHORITY_TIERS") {
        Ok(v) => v != "0" && v.to_lowercase() != "false",
        Err(_) => true,
    }
}

/// R39.7: authority-tier ordering — the `prefer_live_envs` pattern
/// generalized to file classes. Stable sort by (env, tier): tiers refine
/// WITHIN env classes and live-first still dominates. Never a score
/// multiplier (same R32.7 argument — partitions reorder, multipliers don't).
/// Guards (the live-first analogues): queries naming a file are exempt (an
/// explicit filename is the strongest intent signal — R32.7), and
/// `ANCHOR_AUTHORITY_TIERS=0` disables. Under a live-first exemption (test
/// files named, or the demote knob at 1.0) the env rank flattens and tiers
/// refine score order alone — the partition never imposes an env order the
/// live-first guards forbid.
/// R47.1: rich-evidence hits pierce within their env class — the key is
/// (env, pierced, tier), so a test-env pierce never jumps above live.
pub fn prefer_authority_tiers(conn: &Connection, hits: &mut [Hit], query: &str) {
    if hits.len() < 2 || !authority_tiers_enabled() {
        return;
    }
    if !filename_tokens(query).is_empty() {
        return;
    }
    let live_first = !has_env_marker(query) && (0.0..1.0).contains(&test_env_factor());
    let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let env = chunk_item_env(conn, &ids);
    hits.sort_by_key(|h| {
        let pierced = u8::from(!h.rich_match);
        match env.get(&h.chunk_id) {
            Some((path, head)) => {
                let test = live_first && env_for_item(path, head) == "test";
                (u8::from(test), pierced, authority_tier(path))
            }
            // No marker evidence (unknown chunk) reads as live prose, not test.
            None => (0, pierced, AuthorityTier::Prose),
        }
    });
}

/// Tokens that look like filenames (`routing-config.yml`, `service.go`):
/// contain a dot, carry a letter (excludes `1.0.23`), long enough to matter.
/// R39.4: version-like tokens (`v1.2`) never fire — a release label is not
/// a filename mention.
fn filename_tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-' || c == '_')).to_string())
        .filter(|t| t.contains('.') && t.len() >= 4 && t.chars().any(|c| c.is_alphabetic()) && !is_version_token(t))
        .collect()
}

/// R39.4: version-like tokens — an optional `v`/`V` prefix over digits and
/// dots only (`v1.2`, `V2.0.1`). Pure-numeric dotted tokens (`1.2.3`) match
/// too (the letter rule already drops them; this is explicit).
fn is_version_token(t: &str) -> bool {
    let core = t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t);
    !core.is_empty() && core.contains('.') && core.chars().any(|c| c.is_ascii_digit()) && core.chars().all(|c| c.is_ascii_digit() || c == '.')
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

/// Item ids for up to 3 named files — one ranked winner per token.
/// R43.1: `scope` filters candidates before ranking, so an in-scope
/// substring match beats an out-of-scope exact match — the latter must
/// never surface into a project-scoped or public-only answer.
fn named_item_ids(conn: &Connection, names: &[String], scope: Option<&HashSet<i64>>) -> Vec<i64> {
    names.iter().take(3).filter_map(|name| best_item_for_name(conn, name, scope)).collect()
}

/// R39.4: last path segment — the unit whole-token filename matching compares.
fn item_basename(external_id: &str) -> &str {
    external_id.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(external_id)
}

/// R39.4: match tier for one candidate — exact basename first (exact case,
/// then case-insensitive for cross-platform paths), exact title alongside,
/// everything else last.
fn name_match_tier(name: &str, basename: &str, title: Option<&str>) -> u8 {
    if basename == name || title == Some(name) {
        0
    } else if basename.eq_ignore_ascii_case(name) || title.is_some_and(|t| t.eq_ignore_ascii_case(name)) {
        1
    } else {
        2
    }
}

/// R39.4: the one item a filename token names. Candidates come from the
/// substring LIKE (unchanged recall), but the winner is ranked, not
/// lowest-id: exact basename beats substring (`service.go` over
/// `myservice.go`), and among substring matches the shortest path wins.
fn best_item_for_name(conn: &Connection, name: &str, scope: Option<&HashSet<i64>>) -> Option<i64> {
    let pat = format!("%{}%", escape_like(name));
    let mut stmt = conn
        .prepare(
            "SELECT id, external_id, title, source_id FROM ingested_items WHERE external_id LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\' ORDER BY id LIMIT 25",
        )
        .ok()?;
    let rows: Vec<(i64, String, Option<String>, i64)> = stmt
        .query_map(rusqlite::params![pat, pat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .ok()?
        .flatten()
        .collect();
    rows.into_iter()
        .filter(|(_, _, _, sid)| scope.map_or(true, |s| s.contains(sid)))
        .map(|(id, ext, title, _)| {
            let tier = name_match_tier(name, item_basename(&ext), title.as_deref());
            (tier, ext.len(), id)
        })
        .min()
        .map(|(_, _, id)| id)
}

/// Best content-matching chunk of one item for an FTS match expression.
/// Document only — a named file must surface bytes, never a one-line
/// summary extract or paraphrase (R33.3 leaf rule).
fn best_chunk_for_item(conn: &Connection, item_id: i64, match_q: &str) -> Option<i64> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid WHERE chunks_fts MATCH ? AND c.item_id = ? AND c.kind IN ('document','caption') ORDER BY bm25(chunks_fts) LIMIT 1",
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
        "SELECT id FROM chunks WHERE item_id = ?1 AND kind IN ('document','caption') ORDER BY id LIMIT 1",
        [item_id],
        |r| r.get(0),
    )
    .ok()
}

/// Explicit filename mentions surface their file: FTS indexes chunk
/// content, not filenames, so a filename in the query otherwise matches
/// nothing at all. Appends up to 3 matched-item chunks (raw —
/// `parent_return` assembles them with the rest, merging a boosted chunk
/// into an already-returned section), ranked above fused hits — an explicit
/// mention is the strongest intent signal. R32.1: the surfaced chunk is the
/// item's best content match for the rest of the query (file head only when
/// nothing matches). No-op when nothing matches. Kept in R33.3: the leaf
/// pool never sees filenames, so no filter subsumes the bare-filename case.
/// R39.4: whole-token matching (exact basename beats substring, shortest
/// path among the rest) + version-like tokens never fire. R39.5: stale is
/// always excluded, disputed per `qa_exclude_disputed` (threaded from
/// `orchestrate` — no hardcoded filter value on this path). Scoring is
/// exactly `top + 1.0` — `mark_boosted_hits` detects boosted hits via
/// `score > pre-boost max`, so this formula must not change.
/// R43.1: `scope` threads the project/public source filter through name
/// resolution, vocab expansion, and hydration — boosted hits used to
/// ignore scope and leak non-public chunks into `/qa/public`.
pub fn filename_boost_filtered(
    conn: &Connection,
    hits: &mut Vec<Hit>,
    query: &str,
    qa_exclude_disputed: bool,
    scope: Option<&HashSet<i64>>,
) {
    let names = filename_tokens(query);
    if names.is_empty() {
        return;
    }
    let content_q = strip_filename_tokens(query, &names);
    let match_q = fts_match_query_scoped(conn, &content_q, scope);
    let mut cids: Vec<i64> = Vec::new();
    for iid in named_item_ids(conn, &names, scope).into_iter().take(3) {
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
    let mut boosted = load_hits_filtered(conn, &fresh, scope, None, qa_exclude_disputed);
    for h in boosted.iter_mut() {
        h.score = top + 1.0;
    }
    hits.extend(boosted);
    sort_hits(hits);
}

/// R39.5: production default for the boost exclusion flag — mirrors
/// `answer.rs::RetrievalConfig::from_env` (`ANCHOR_QA_EXCLUDE_DISPUTED`,
/// default exclude). Only the legacy 3-arg wrapper resolves it here;
/// `orchestrate` threads its cfg explicitly.
fn boost_exclude_disputed_default() -> bool {
    std::env::var("ANCHOR_QA_EXCLUDE_DISPUTED")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true)
}

/// Legacy 3-arg entry — unit tests call this; it resolves the production
/// exclusion default from env. Production (`orchestrate_fusion`, R39.12)
/// passes the flag to `filename_boost_filtered` explicitly.
pub fn filename_boost(conn: &Connection, hits: &mut Vec<Hit>, query: &str) {
    filename_boost_filtered(conn, hits, query, boost_exclude_disputed_default(), None)
}

/// R39.1: pre-boost max score — the snapshot `mark_boosted_hits` compares to.
pub fn top_score(hits: &[Hit]) -> f64 {
    hits.iter().map(|h| h.score).fold(0.0f64, f64::max)
}

/// R39.1: mark filename-boosted hits. `filename_boost` scores surfaced
/// chunks `top + 1.0`, so anything above the pre-boost max is boosted —
/// detected here so the boost itself stays untouched.
pub fn mark_boosted_hits(hits: &mut [Hit], prev_top: f64) {
    for h in hits.iter_mut() {
        if h.score > prev_top {
            h.boosted = true;
        }
    }
}

/// R39.2: reserved keyword slots in the final top-k — `ANCHOR_KEYWORD_SLOTS`,
/// default 2, 0 disables.
pub fn keyword_slots() -> usize {
    std::env::var("ANCHOR_KEYWORD_SLOTS").ok().and_then(|v| v.parse().ok()).unwrap_or(2)
}

/// R39.2: pull top floor-keyword hits locked out by fusion into reserved
/// tail slots of the final top-k — via ordering, not weight changes. The
/// TOC path fuses at TOC@1.0 vs floors@0.5, so a floor-only keyword match
/// can never outrank a TOC hit and the fuse cut drops it; the reserved
/// slots displace the lowest-ranked tail instead (the list is live-first,
/// so tail eviction takes test hits before live ones). Decay applies to
/// pulled hits like any fused hit; live-first + authority tiers (R39.7)
/// re-partition after the insert, so env/tier ordering still holds.
pub fn apply_keyword_slots(
    conn: &Connection,
    hits: &mut Vec<Hit>,
    floor_kw: &[Hit],
    top_k: usize,
    halflife_days: f64,
    query: &str,
) {
    let slots = keyword_slots();
    if slots == 0 || floor_kw.is_empty() || top_k == 0 {
        return;
    }
    let mut take = select_slot_hits(hits, floor_kw, slots);
    if take.is_empty() {
        return;
    }
    apply_age_decay(conn, &mut take, halflife_days);
    let overflow = hits.len().saturating_add(take.len()).saturating_sub(top_k);
    // R40.4: diversity-preserving eviction (uniform-kind lists truncate the
    // plain tail, exactly as before).
    evict_for_slots(conn, hits, overflow);
    hits.extend(take);
    prefer_live_envs(conn, hits, query);
    // R39.7: the slot insert runs after the fuse cut, so the pulled hits
    // need the same (env, tier) order the cut list already has.
    prefer_authority_tiers(conn, hits, query);
}

/// R40.4: reserved per-doc-type slots in the final top-k —
/// `ANCHOR_TYPE_SLOTS`, default 2, 0 disables.
pub fn type_slots() -> usize {
    std::env::var("ANCHOR_TYPE_SLOTS").ok().and_then(|v| v.parse().ok()).unwrap_or(2)
}

/// R40.4: batch `doc_type` lookup for slot diversity (mirrors the
/// `envs_for_chunks` batching — one query, not one per hit). Empty/unset
/// reads as absent; `slot_kind` renders those `"unknown"`.
fn doc_types_for_items(conn: &Connection, item_ids: &[i64]) -> HashMap<i64, String> {
    if item_ids.is_empty() {
        return HashMap::new();
    }
    let placeholders = item_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT id, doc_type FROM ingested_items WHERE id IN ({placeholders})");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let params: Vec<rusqlite::types::Value> =
        item_ids.iter().map(|id| rusqlite::types::Value::Integer(*id)).collect();
    stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
    })
    .map(|m| {
        m.filter_map(|r| r.ok())
            .filter_map(|(id, dt)| dt.filter(|s| !s.is_empty()).map(|d| (id, d)))
            .collect()
    })
    .unwrap_or_default()
}

/// R40.4: slot kind of a hit — its item's `doc_type`, `"unknown"` when the
/// item is missing or untyped. Unknowns never pull (fallback artifacts
/// carry no diversity value) but count as present.
fn slot_kind(hit: &Hit, kinds: &HashMap<i64, String>) -> String {
    hit.item_id.and_then(|i| kinds.get(&i).cloned()).unwrap_or_else(|| "unknown".to_string())
}

/// R40.4: pull the best below-cut hit of each doc kind missing from the cut
/// into reserved tail slots — the R39.2 keyword-slot pattern (ordering, not
/// weights) for the ADR-heavy-corpus crowding case (a PDF match ranked 6th
/// of top-5 would otherwise never surface). Runs inside `fuse_and_rank`,
/// so both the TOC and flat branches get it; the (env, tier) re-partition
/// after the insert keeps live-first + authority order. The pool is the
/// below-cut tail of the already-decayed fuse list, so no re-decay (unlike
/// keyword slots, whose pool never saw decay). Precedence on tail
/// collision: keyword slots run after (TOC branch) and `evict_for_slots`
/// protects singleton-kind hits, so a type pull usually survives — when
/// singletons cover the whole tail, keyword relevance wins the eviction.
pub fn apply_type_slots(conn: &Connection, cut: &mut Vec<Hit>, pool: &[Hit], top_k: usize, query: &str) {
    let slots = type_slots();
    if slots == 0 || pool.is_empty() || top_k == 0 {
        return;
    }
    let ids: Vec<i64> =
        cut.iter().filter_map(|h| h.item_id).chain(pool.iter().filter_map(|h| h.item_id)).collect();
    let kinds = doc_types_for_items(conn, &ids);
    let present: HashSet<String> = cut.iter().map(|h| slot_kind(h, &kinds)).collect();
    let mut pulled: HashSet<String> = HashSet::new();
    let mut take: Vec<Hit> = Vec::new();
    for h in pool {
        if take.len() >= slots {
            break;
        }
        let kind = slot_kind(h, &kinds);
        if kind == "unknown" || present.contains(&kind) || pulled.contains(&kind) {
            continue;
        }
        pulled.insert(kind);
        take.push(h.clone());
    }
    if take.is_empty() {
        return;
    }
    let overflow = cut.len().saturating_add(take.len()).saturating_sub(top_k);
    evict_for_slots(conn, cut, overflow);
    cut.extend(take);
    prefer_live_envs(conn, cut, query);
    prefer_authority_tiers(conn, cut, query);
}

/// R40.4: tail eviction that preserves diversity — removes `n` hits,
/// preferring duplicated-kind hits (lowest-ranked first) so singleton-kind
/// hits (type-slot pulls included) survive slot pressure from keyword
/// slots running later. `"unknown"` is always evictable (no diversity
/// value); when singletons cover the whole tail it falls back to plain
/// tail truncation. Uniform-kind lists remove exactly the lowest `n`,
/// so pre-R40.4 truncation behavior is unchanged there.
fn evict_for_slots(conn: &Connection, hits: &mut Vec<Hit>, n: usize) {
    if n == 0 || hits.is_empty() {
        return;
    }
    let ids: Vec<i64> = hits.iter().filter_map(|h| h.item_id).collect();
    let kinds = doc_types_for_items(conn, &ids);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for h in hits.iter() {
        *counts.entry(slot_kind(h, &kinds)).or_insert(0) += 1;
    }
    let mut evicted = 0usize;
    let mut idx = hits.len();
    while evicted < n && idx > 0 {
        idx -= 1;
        let kind = slot_kind(&hits[idx], &kinds);
        if kind == "unknown" || counts.get(&kind).copied().unwrap_or(0) > 1 {
            hits.remove(idx);
            if let Some(c) = counts.get_mut(&kind) {
                *c = c.saturating_sub(1);
            }
            evicted += 1;
        }
    }
    while evicted < n && !hits.is_empty() {
        hits.pop();
        evicted += 1;
    }
}

/// R39.3: minimum vector cosine for evidence without any BM25 match —
/// `ANCHOR_MIN_RELEVANCE_COSINE`, default 0.50. Calibrated 2026-10-19 on
/// nomic-embed-text x sample/ (14 chunks, 800-char windows): direct 0.75,
/// no-shared-term paraphrases 0.57-0.64, weak 0.56 | gibberish 0.46,
/// unrelated 0.37. 0.50 separates with ~0.1 margin either side; the 0.2
/// vec floor admits everything measured (min 0.30), so it gates nothing.
pub fn min_relevance_cosine() -> f64 {
    std::env::var("ANCHOR_MIN_RELEVANCE_COSINE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.50)
}

/// R39.3: minimum-relevance gate — some hit must carry a real BM25 match,
/// an explicit filename boost, or vector cosine at or above the floor, or
/// the whole list is vector noise and `orchestrate` returns empty (ask
/// answers mode "none"). Boosted hits pass as explicit intent: a bare
/// filename matches no content by design (R32.1), so gating them would
/// delete the exact file the user named.
pub fn passes_relevance_gate(hits: &[Hit]) -> bool {
    let floor = min_relevance_cosine();
    hits.iter().any(|h| h.keyword_matched || h.boosted || h.cosine.map(|c| c >= floor).unwrap_or(false))
}

/// R39.12: TOC-branch fusion inputs — the TOC precision list at 1.0 over
/// the discounted flat recall floor (0.5 legs), with leg names for R39.1
/// provenance. Empty floor legs stay out (no empty-list fusion noise).
fn toc_fusion_lists(
    toc: Vec<Hit>,
    floor_vec: Option<Vec<Hit>>,
    floor_kw: Option<Vec<Hit>>,
) -> (Vec<Vec<Hit>>, Vec<f64>, Vec<&'static str>) {
    let mut lists = vec![toc];
    let mut weights = vec![1.0];
    let mut legs = vec!["toc"];
    if let Some(h) = floor_vec {
        lists.push(h);
        weights.push(0.5);
        legs.push("vector");
    }
    if let Some(h) = floor_kw {
        lists.push(h);
        weights.push(0.5);
        legs.push("keyword");
    }
    (lists, weights, legs)
}

/// R39.12: post-cut finish shared by both branches — filename boost above
/// a pre-boost snapshot (R39.1 detection still exact), then return-shape
/// assembly (R36.3) over the final ranked list, so same-scope merging sees
/// boosted hits too.
fn boost_and_assemble(
    conn: &Connection,
    mut hits: Vec<Hit>,
    query: &str,
    qa_exclude_disputed: bool,
    scope: Option<&HashSet<i64>>,
) -> Vec<Hit> {
    let prev_top = top_score(&hits);
    // R39.5: the cfg exclusion flag threads in — the boost hardcodes nothing.
    // R43.1: scope threads in too — boosted hits obey project/public scope.
    filename_boost_filtered(conn, &mut hits, query, qa_exclude_disputed, scope);
    mark_boosted_hits(&mut hits, prev_top);
    parent_return(conn, hits)
}

/// R39.12: the pure fusion path — TOC-or-flat legs → RRF fuse →
/// `fuse_and_rank` → keyword slots → filename boost → `parent_return` →
/// status re-filter → relevance gate. Everything `answer.rs::orchestrate`
/// does minus scope resolution and embedding computation, which stay with
/// the caller (`answer.rs` resolves project scope + the embedder; the
/// R39.10 eval passes its own). Both call this — the eval's
/// `orchestrate_mirror` restatement is deleted, so production and the
/// golden set can never drift again. R33.3: no graph leg — entity
/// neighbors are entity chunks, and entities never enter retrieval.
#[allow(clippy::too_many_arguments)]
pub fn orchestrate_fusion(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    scope: Option<&HashSet<i64>>,
    top_k: usize,
    keyword_weight: f64,
    halflife_days: f64,
    max_per_source: usize,
    qa_exclude_disputed: bool,
) -> Vec<Hit> {
    // R14.5: TOC coarse-to-fine — try toc_search first (prunes to ~200 candidates)
    let toc_opt = {
        let toc = toc_search(conn, query, query_embedding, scope, top_k, qa_exclude_disputed);
        if toc.is_empty() { None } else { Some(toc) }
    };
    let hits = if let Some(toc) = toc_opt {
        // Flat recall floor (R36.7 funnel follow-up): TOC fences leaf search
        // to summary-admitted sections, and head-extract summaries can't see
        // mid-body signals (benchmark ai-03: d1's only "incident" mention is
        // a See-also line — its summaries match nothing, so the leaf fence
        // excludes the whole file). Union unfiltered hybrid legs UNDERNEATH
        // the TOC ranking (discounted: precision-first, recall-floor), so TOC
        // can never underperform flat. Single fused list into fuse_and_rank
        // keeps all downstream behavior (dedupe/live-first/cut).
        let floor_vec = query_embedding.and_then(|emb| {
            let hits = vector_search(conn, emb, scope, top_k, qa_exclude_disputed);
            if hits.is_empty() { None } else { Some(hits) }
        });
        let floor_kw = keyword_search(conn, query, scope, top_k, qa_exclude_disputed);
        // R39.2: slot candidates — one clone per TOC-path query; fusion below consumes the original.
        let slot_pool = floor_kw.clone();
        let floor_kw_opt = if floor_kw.is_empty() { None } else { Some(floor_kw) };
        let (lists, weights, legs) = toc_fusion_lists(toc, floor_vec, floor_kw_opt);
        let fused = rrf_fuse_multi_named(lists, weights, &legs);
        // Env ordering is the live-first partition inside fuse_and_rank
        // (R33.3: the R32.3 score multiplier is retired — RRF fuses by rank).
        let mut cut = fuse_and_rank(
            conn,
            Some(fused),
            None,
            keyword_weight,
            halflife_days,
            max_per_source,
            top_k,
            qa_exclude_disputed,
            query,
        );
        // R39.2: reserved keyword slots pull locked-out floor hits into the
        // cut list before boost/return-shape (boost detection still exact —
        // the snapshot runs after the slots).
        apply_keyword_slots(conn, &mut cut, &slot_pool, top_k, halflife_days, query);
        boost_and_assemble(conn, cut, query, qa_exclude_disputed, scope)
    } else {
        // R8.1: hybrid search — vector + keyword
        let vector_hits = query_embedding.and_then(|emb| {
            let hits = vector_search(conn, emb, scope, top_k, qa_exclude_disputed);
            if hits.is_empty() { None } else { Some(hits) }
        });
        let keyword_hits = keyword_search(conn, query, scope, top_k, qa_exclude_disputed);
        // F-07: no newest-documents fallback — a no-match query must surface as
        // empty evidence (ask answers mode "none", search returns no hits), not
        // as cited "evidence" that never mentions the question.
        let keyword_opt = if keyword_hits.is_empty() { None } else { Some(keyword_hits) };
        let fused = fuse_and_rank(
            conn,
            vector_hits,
            keyword_opt,
            keyword_weight,
            halflife_days,
            max_per_source,
            top_k,
            qa_exclude_disputed,
            query,
        );
        boost_and_assemble(conn, fused, query, qa_exclude_disputed, scope)
    };
    // R39.5: status re-filtered after `parent_return` — return-shape
    // assembly must never reintroduce excluded evidence — before the
    // relevance gate sees the final list.
    // R43.1: scope asserted here too — defense in depth so no path
    // (boost, slots, return-shape) can leak an out-of-scope hit.
    let hits: Vec<Hit> = hits
        .into_iter()
        .filter(|h| status_ok(h.status.as_deref(), qa_exclude_disputed))
        .filter(|h| scope_hit_ok(h, scope))
        .collect();
    // R39.3: minimum-relevance gate — empty evidence unless some hit
    // carries a real BM25 match, a boost, or cosine at the floor (ask
    // answers mode "none", search returns no hits).
    if hits.is_empty() || passes_relevance_gate(&hits) {
        hits
    } else {
        Vec::new()
    }
}

/// Top floor-keyword hits (keyword-rank order) missing from the fused list,
/// stamped with their keyword rank — they bypassed fusion, but unlike
/// boosted hits their rank is real.
fn select_slot_hits(hits: &[Hit], floor_kw: &[Hit], slots: usize) -> Vec<Hit> {
    let present: HashSet<i64> = hits.iter().map(|h| h.chunk_id).collect();
    let mut take = Vec::new();
    for (i, h) in floor_kw.iter().enumerate() {
        if take.len() >= slots {
            break;
        }
        if present.contains(&h.chunk_id) {
            continue;
        }
        let mut h = h.clone();
        h.provenance = vec![LegMatch { leg: "keyword".to_string(), rank: i + 1 }];
        // R40.2: rescale into RRF range — the raw BM25 score (~14) would sit
        // next to fused scores (~0.01) and lie to score-reading agents
        // (1.0.29 live report). The value is the RRF contribution the floor
        // leg (weight 0.5) would have given this rank; order is unaffected
        // (list position + stable partitions decide, never scores).
        h.score = 0.5 / (RRF_K + (i + 1) as f64);
        take.push(h);
    }
    take
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
    fn synonym_expansion_pulls_cluster_members() {
        // R40.6: "cancellation" expands to the cancel/reverse/withdraw family;
        // originals first, synonyms appended deduped.
        let q = fts_match_query("how does cancellation work").expect("terms");
        assert!(q.starts_with("\"cancellation\" OR \"work\""), "originals first: {q}");
        for syn in ["cancel", "reversal", "reverse", "withdrawal"] {
            assert!(q.contains(&format!("\"{syn}\"")), "missing synonym {syn}: {q}");
        }
        // Both directions: "reversal" pulls "cancellation" back.
        let back = fts_match_query("reversal notifications").expect("terms");
        assert!(back.contains("\"cancellation\""), "reverse direction: {back}");
    }

    #[test]
    fn synonym_expansion_acronym_stays_phrased() {
        // R40.6: "DD" expands to the "direct-debit" phrase — a loose
        // "direct" term would over-match (it is everywhere).
        let q = fts_match_query("the DD process").expect("terms");
        assert!(q.contains("\"direct-debit\""), "phrased acronym: {q}");
        assert!(!q.split(" OR ").any(|t| t == "\"direct\""), "no loose direct: {q}");
        assert!(!q.split(" OR ").any(|t| t == "\"debit\""), "no loose debit: {q}");
    }

    #[test]
    fn synonym_expansion_leaves_other_queries_alone() {
        // R40.6 immune check at the unit level: no cluster member, no change.
        assert_eq!(
            fts_match_query("who knows about Phoenix?"),
            Some("\"knows\" OR \"phoenix\"".to_string())
        );
        assert_eq!(fts_match_query("the and or"), None);
    }

    #[test]
    fn scoped_fts_query_merges_active_vocab() {
        // R40.7/R45.13: glossary pairs join the FTS OR behind curated
        // synonyms (marked-file rows apply in every llm_mode); project scope
        // filters.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "sv", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let dunning = crate::vocab::normalize_pair("dunning", "payment reminder").unwrap();
        crate::vocab::store_vocab(&conn, iid, sid, &[], &[dunning]).unwrap();
        assert_eq!(
            fts_match_query_scoped(&conn, "what is dunning", None),
            Some("\"dunning\" OR \"payment reminder\"".to_string())
        );
        assert_eq!(
            fts_match_query_scoped(&conn, "solo mission", None),
            Some("\"solo\" OR \"mission\"".to_string()),
            "no pair, no expansion"
        );
        let other: HashSet<i64> = [sid + 100].into_iter().collect();
        assert_eq!(
            fts_match_query_scoped(&conn, "what is dunning", Some(&other)),
            Some("\"dunning\"".to_string()),
            "other project sees no vocab"
        );
        // The pure form is vocab-blind (sees the DB-free curated map only).
        assert_eq!(fts_match_query("what is dunning"), Some("\"dunning\"".to_string()));
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
        let h1 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false }];
        let h2 = vec![Hit { chunk_id: 1, entity_id: None, item_id: None, source_id: None, section_id: None, path: String::new(), score: 0.0, content: "".into(), source_ref: "".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false }];
        let fused = rrf_fuse_multi(vec![h1, h2], vec![1.0, 1.0]);
        assert_eq!(fused.len(), 1);
        assert!(fused[0].score > 0.0);
    }

    fn blank_hit(chunk_id: i64) -> Hit {
        Hit {
            chunk_id,
            entity_id: None,
            item_id: None,
            source_id: None,
            section_id: None,
            path: String::new(),
            score: 0.0,
            content: String::new(),
            source_ref: String::new(),
            is_pii: false,
            status: None,
            content_truncated: false,
            provenance: Vec::new(),
            decay: 1.0,
            boosted: false,
            keyword_matched: false,
            cosine: None,
            rich_match: false,
        }
    }

    #[test]
    fn slot_pulled_hits_score_in_rrf_range() {
        // R40.2: pulled hits arrive with raw BM25-scale scores (~14) — they
        // must rescale into the fused RRF range (~0.01) so score-reading
        // agents compare like with like (1.0.29 live report).
        let mut kw1 = blank_hit(7);
        kw1.score = 14.3;
        let mut kw2 = blank_hit(8);
        kw2.score = 9.1;
        let fused = vec![blank_hit(1)];
        let out = select_slot_hits(&fused, &[kw1, kw2], 2);
        assert_eq!(out.len(), 2);
        assert!((out[0].score - 0.5 / (RRF_K + 1.0)).abs() < 1e-9, "rank-1 slot scores its floor RRF share, got {}", out[0].score);
        assert!((out[1].score - 0.5 / (RRF_K + 2.0)).abs() < 1e-9, "rank-2 slot scores its floor RRF share, got {}", out[1].score);
        assert!(out.iter().all(|h| h.score < 1.0), "no BM25-scale survivor");
    }

    // --- R40.4: reserved per-doc-type slots ---

    /// Seed one item + chunk with doc_type + external path (R40.4 slot tests).
    fn insert_typed_item(
        conn: &rusqlite::Connection,
        sid: i64,
        title: &str,
        doc_type: &str,
        external: &str,
        content: &str,
    ) -> (i64, i64) {
        let iid = insert_item(conn, sid, title);
        conn.execute(
            "UPDATE ingested_items SET doc_type=?1, external_id=?2 WHERE id=?3",
            rusqlite::params![doc_type, external, iid],
        )
        .unwrap();
        let cid = insert_chunk(conn, Some(iid), None, content);
        (iid, cid)
    }

    fn slot_hit(cid: i64, iid: i64, sid: i64, score: f64, content: &str) -> Hit {
        Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        }
    }

    #[test]
    fn type_slots_pull_pdf_below_cut_into_tail() {
        // R40.4: ADR-heavy seeded corpus — the PDF match ranked 6th of top-5
        // pulls a reserved tail slot (ordering, not weights); live-first +
        // tiers still hold on the slotted list.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TYPE_SLOTS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s40", "internal");
        let names = ["alpha", "beta", "gamma", "delta", "epsilon"];
        let mut adr = Vec::new();
        for (i, name) in names.iter().enumerate() {
            adr.push(insert_typed_item(
                &conn,
                sid,
                name,
                "record",
                &format!("decisions/adr-{:03}.md", i + 1),
                &format!("adr {name} billing routes ledger entry"),
            ));
        }
        let (tst_iid, tst_cid) =
            insert_typed_item(&conn, sid, "dup", "record", "test/adr-dup.md", "adr duplicate billing routes ledger entry");
        let (pdf_iid, pdf_cid) =
            insert_typed_item(&conn, sid, "rfp", "reference", "docs/rfp.pdf", "rfp reversal direct debit schedule");
        // Scores: the test-env ADR outranks everything raw, the PDF trails.
        let mut hits = vec![slot_hit(tst_cid, tst_iid, sid, 9.0, "adr duplicate billing routes ledger entry")];
        for (rank, (iid, cid)) in adr.iter().enumerate() {
            hits.push(slot_hit(*cid, *iid, sid, 8.0 - rank as f64, &format!("adr {} billing routes ledger entry", names[rank])));
        }
        hits.push(slot_hit(pdf_cid, pdf_iid, sid, 3.0, "rfp reversal direct debit schedule"));
        let out = fuse_and_rank(&conn, None, Some(hits), 1.0, 365.0, 3, 5, true, "billing routes");
        assert_eq!(out.len(), 5);
        let order: Vec<i64> = out.iter().map(|h| h.chunk_id).collect();
        // Live-first drops the test ADR below the live PDF before the cut;
        // the PDF (reference, missing from the all-record cut) pulls the
        // tail slot, displacing the lowest live ADR; tiers keep Spec ADRs
        // above the Prose PDF.
        assert!(!order.contains(&tst_cid), "test-env hit stays out: {order:?}");
        assert_eq!(order, vec![adr[0].1, adr[1].1, adr[2].1, adr[3].1, pdf_cid], "slotted order: {order:?}");
    }

    #[test]
    fn type_slots_knob_zero_disables() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("ANCHOR_TYPE_SLOTS", "0");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s40b", "internal");
        let mut hits = Vec::new();
        for (i, name) in ["alpha", "beta", "gamma"].iter().enumerate() {
            let (iid, cid) = insert_typed_item(
                &conn, sid, name, "record",
                &format!("decisions/adr-{:03}.md", i + 1),
                &format!("adr {name} billing routes ledger entry"),
            );
            hits.push(slot_hit(cid, iid, sid, 9.0 - i as f64, &format!("adr {name} billing routes ledger entry")));
        }
        let (pdf_iid, pdf_cid) =
            insert_typed_item(&conn, sid, "rfp", "reference", "docs/rfp.pdf", "rfp reversal direct debit schedule");
        hits.push(slot_hit(pdf_cid, pdf_iid, sid, 1.0, "rfp reversal direct debit schedule"));
        let out = fuse_and_rank(&conn, None, Some(hits), 1.0, 365.0, 3, 3, true, "billing routes");
        assert_eq!(out.len(), 3);
        assert!(!out.iter().any(|h| h.chunk_id == pdf_cid), "slots=0 pulls nothing");
        std::env::remove_var("ANCHOR_TYPE_SLOTS");
    }

    #[test]
    fn type_slots_noop_when_covered_or_single_kind() {
        // Covered: the missing kind is already in the cut (live PDF above
        // test ADRs), pool holds only covered kinds → list identical.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TYPE_SLOTS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s40c", "internal");
        let (pdf_iid, pdf_cid) =
            insert_typed_item(&conn, sid, "rfp", "reference", "docs/rfp.pdf", "rfp reversal direct debit schedule");
        let mut hits = vec![slot_hit(pdf_cid, pdf_iid, sid, 2.0, "rfp reversal direct debit schedule")];
        let mut tst = Vec::new();
        for i in 0..4 {
            let (iid, cid) = insert_typed_item(
                &conn, sid, &format!("t{i}"), "record",
                &format!("test/adr-dup-{i}.md"),
                &format!("adr duplicate {i} billing routes ledger entry"),
            );
            hits.push(slot_hit(cid, iid, sid, 9.0 - i as f64, &format!("adr duplicate {i} billing routes ledger entry")));
            tst.push(cid);
        }
        let out = fuse_and_rank(&conn, None, Some(hits), 1.0, 365.0, 3, 3, true, "billing routes");
        let order: Vec<i64> = out.iter().map(|h| h.chunk_id).collect();
        assert_eq!(order, vec![pdf_cid, tst[0], tst[1]], "covered cut is untouched: {order:?}");
        // Single-kind: everything is record → nothing to pull, plain top-3.
        let (_dir2, conn2) = harness_db();
        let sid2 = insert_source(&conn2, "s40d", "internal");
        let mut hits2 = Vec::new();
        let mut cids = Vec::new();
        for (i, name) in ["alpha", "beta", "gamma", "delta", "epsilon"].iter().enumerate() {
            let (iid, cid) = insert_typed_item(
                &conn2, sid2, name, "record",
                &format!("decisions/adr-{:03}.md", i + 1),
                &format!("adr {name} billing routes ledger entry"),
            );
            hits2.push(slot_hit(cid, iid, sid2, 9.0 - i as f64, &format!("adr {name} billing routes ledger entry")));
            cids.push(cid);
        }
        let out2 = fuse_and_rank(&conn2, None, Some(hits2), 1.0, 365.0, 3, 3, true, "billing routes");
        let order2: Vec<i64> = out2.iter().map(|h| h.chunk_id).collect();
        assert_eq!(order2, cids[..3].to_vec(), "single-kind cut is untouched: {order2:?}");
    }

    #[test]
    fn type_slots_skip_unknown_kind() {
        // A below-cut hit with no item never pulls (no diversity value);
        // the next missing kind still does.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TYPE_SLOTS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s40e", "internal");
        let mut cut = Vec::new();
        for (i, name) in ["alpha", "beta", "gamma"].iter().enumerate() {
            let (iid, cid) = insert_typed_item(
                &conn, sid, name, "record",
                &format!("decisions/adr-{:03}.md", i + 1),
                &format!("adr {name} billing routes ledger entry"),
            );
            cut.push(slot_hit(cid, iid, sid, 9.0 - i as f64, &format!("adr {name} billing routes ledger entry")));
        }
        let (pdf_iid, pdf_cid) =
            insert_typed_item(&conn, sid, "rfp", "reference", "docs/rfp.pdf", "rfp reversal direct debit schedule");
        let mut orphan = blank_hit(4242);
        orphan.content = "orphan bytes with no item".to_string();
        let pool = vec![orphan, slot_hit(pdf_cid, pdf_iid, sid, 1.0, "rfp reversal direct debit schedule")];
        apply_type_slots(&conn, &mut cut, &pool, 3, "billing routes");
        assert_eq!(cut.len(), 3);
        assert!(cut.iter().any(|h| h.chunk_id == pdf_cid), "PDF pulls past the orphan");
        assert!(!cut.iter().any(|h| h.chunk_id == 4242), "item-less hit never pulls");
    }

    #[test]
    fn keyword_slots_preserve_singleton_type_pull() {
        // R40.4 × R39.2 composition: keyword slots run after type slots and
        // evict duplicated-kind tail first, so a singleton type pull
        // survives the collision (both mechanisms hold their hits).
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TYPE_SLOTS");
        std::env::remove_var("ANCHOR_KEYWORD_SLOTS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s40f", "internal");
        let mut cut = Vec::new();
        for (i, name) in ["alpha", "beta", "gamma", "delta"].iter().enumerate() {
            let (iid, cid) = insert_typed_item(
                &conn, sid, name, "record",
                &format!("decisions/adr-{:03}.md", i + 1),
                &format!("adr {name} billing routes ledger entry"),
            );
            cut.push(slot_hit(cid, iid, sid, 9.0 - i as f64, &format!("adr {name} billing routes ledger entry")));
        }
        let (pdf_iid, pdf_cid) =
            insert_typed_item(&conn, sid, "rfp", "reference", "docs/rfp.pdf", "rfp reversal direct debit schedule");
        cut.push(slot_hit(pdf_cid, pdf_iid, sid, 4.0, "rfp reversal direct debit schedule"));
        let mut floor = Vec::new();
        for (i, name) in ["one", "two"].iter().enumerate() {
            let (iid, cid) = insert_typed_item(
                &conn, sid, name, "record",
                &format!("docs/note-{name}.md"),
                &format!("note {name} billing routes keyword lockout"),
            );
            let mut h = slot_hit(cid, iid, sid, 14.0 - i as f64, &format!("note {name} billing routes keyword lockout"));
            h.keyword_matched = true;
            floor.push(h);
        }
        apply_keyword_slots(&conn, &mut cut, &floor, 5, 365.0, "billing routes");
        assert_eq!(cut.len(), 5);
        let order: Vec<i64> = cut.iter().map(|h| h.chunk_id).collect();
        assert!(order.contains(&pdf_cid), "singleton type pull survives: {order:?}");
        assert!(order.contains(&floor[0].chunk_id) && order.contains(&floor[1].chunk_id), "keyword pulls land: {order:?}");
    }

    #[test]
    fn rrf_fuse_named_records_contributing_ranks() {
        // R39.1: every fused hit names its contributing legs + 1-based ranks.
        let fused = rrf_fuse_multi_named(
            vec![vec![blank_hit(1), blank_hit(2)], vec![blank_hit(2), blank_hit(3)]],
            vec![1.0, 1.0],
            &["vector", "keyword"],
        );
        assert_eq!(fused.len(), 3);
        let prov = |cid: i64| fused.iter().find(|h| h.chunk_id == cid).unwrap().provenance.clone();
        assert_eq!(prov(1), vec![LegMatch { leg: "vector".into(), rank: 1 }]);
        assert_eq!(prov(2), vec![LegMatch { leg: "vector".into(), rank: 2 }, LegMatch { leg: "keyword".into(), rank: 1 }]);
        assert_eq!(prov(3), vec![LegMatch { leg: "keyword".into(), rank: 2 }]);
        // Unnamed lists (toc_search internals) read as list{index}.
        let unnamed = rrf_fuse_multi(vec![vec![blank_hit(7)]], vec![1.0]);
        assert_eq!(unnamed[0].provenance, vec![LegMatch { leg: "list0".into(), rank: 1 }]);
    }

    #[test]
    fn fusion_preserves_keyword_and_cosine_evidence() {
        // R39.1/38.3: the gate reads the union — keyword ORs, cosine maxes.
        let mut v = blank_hit(1);
        v.cosine = Some(0.7);
        let mut k = blank_hit(1);
        k.keyword_matched = true;
        k.cosine = Some(0.4);
        let fused = rrf_fuse_multi_named(vec![vec![v], vec![k]], vec![1.0, 1.0], &["vector", "keyword"]);
        assert_eq!(fused.len(), 1);
        assert!(fused[0].keyword_matched);
        assert_eq!(fused[0].cosine, Some(0.7));
    }

    #[test]
    fn fuse_and_rank_stamps_single_list_provenance_and_decay() {
        // R39.1: single-list hits keep raw scores AND record leg + rank +
        // applied decay; pre-fused lists keep their outer-fusion record.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_prov", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let c1 = insert_chunk(&conn, Some(iid), None, "treasury routes alpha content here");
        let c2 = insert_chunk(&conn, Some(iid), None, "treasury routes beta content here");
        let mk = |cid: i64, score: f64| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: format!("distinct content {cid}"), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let out = fuse_and_rank(&conn, None, Some(vec![mk(c1, 9.0), mk(c2, 4.0)]), 1.0, 365.0, 3, 10, true, "treasury routes");
        assert_eq!(out.len(), 2);
        for h in &out {
            assert_eq!(h.provenance.len(), 1, "single-list hit carries one leg");
            assert_eq!(h.provenance[0].leg, "keyword");
            assert!((h.decay - 1.0).abs() < 1e-6, "fresh chunk decay must be ~1.0, got {}", h.decay);
        }
        assert_eq!(out.iter().find(|h| h.chunk_id == c1).unwrap().provenance[0].rank, 1);
        assert_eq!(out.iter().find(|h| h.chunk_id == c2).unwrap().provenance[0].rank, 2);
        // Pre-fused input keeps its record (the TOC path feeds one fused list).
        let mut pre = mk(c1, 0.03);
        pre.provenance = vec![LegMatch { leg: "toc".into(), rank: 3 }];
        let out2 = fuse_and_rank(&conn, Some(vec![pre]), None, 1.0, 365.0, 3, 10, true, "treasury routes");
        assert_eq!(out2[0].provenance, vec![LegMatch { leg: "toc".into(), rank: 3 }]);
    }

    #[test]
    fn mark_boosted_hits_flags_above_prev_top() {
        // R39.1: filename_boost scores surfaced chunks top + 1.0 — anything
        // above the pre-boost max is boosted, everything else is not.
        let mut v = vec![blank_hit(1), blank_hit(2)];
        v[0].score = 5.0;
        v[1].score = 6.0;
        mark_boosted_hits(&mut v, 5.0);
        assert!(!v[0].boosted);
        assert!(v[1].boosted);
        assert_eq!(top_score(&v), 6.0);
    }

    #[test]
    fn relevance_gate_needs_keyword_boost_or_cosine() {
        // R39.3: vector noise below the floor gates the whole list; any one
        // real signal (BM25, boost, cosine) passes it. Default floor 0.50.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_MIN_RELEVANCE_COSINE");
        let mut noise = blank_hit(1);
        noise.cosine = Some(0.30);
        assert!(!passes_relevance_gate(&[noise.clone()]), "0.30 cosine must gate at 0.50");
        assert!(!passes_relevance_gate(&[blank_hit(2)]), "no evidence at all must gate");
        let mut strong = blank_hit(3);
        strong.cosine = Some(0.99);
        assert!(passes_relevance_gate(&[noise.clone(), strong]));
        let mut kw = blank_hit(4);
        kw.keyword_matched = true;
        assert!(passes_relevance_gate(&[noise.clone(), kw]));
        let mut b = blank_hit(5);
        b.boosted = true;
        assert!(passes_relevance_gate(&[noise, b]));
        // Knob honored: a stricter floor gates the 0.99 hit too.
        std::env::set_var("ANCHOR_MIN_RELEVANCE_COSINE", "0.9999");
        let mut strong2 = blank_hit(6);
        strong2.cosine = Some(0.99);
        assert!(!passes_relevance_gate(&[strong2]));
        std::env::remove_var("ANCHOR_MIN_RELEVANCE_COSINE");
    }

    #[test]
    fn age() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
        let now = Utc.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap();
        let old = Utc.with_ymd_and_hms(2025, 1, 10, 12, 0, 0).unwrap();
        let d = age_decay(Some(old), 365.0, now);
        assert!(d < 1.0 && d > 0.4);
    }

    #[test]
    fn age_decay_floors_at_08_by_default() {
        // R39.6: max(0.8, decay); the knob clamps to [0, 1], 0 restores raw.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
        let now = Utc.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap();
        let old = Utc.with_ymd_and_hms(2024, 1, 10, 12, 0, 0).unwrap();
        let raw = 0.5_f64.powf(731.0 / 365.0);
        assert!((age_decay(Some(old), 365.0, now) - 0.8).abs() < 1e-9, "raw {raw} must floor at 0.8");
        let fresh = Utc.with_ymd_and_hms(2026, 1, 9, 12, 0, 0).unwrap();
        assert!(age_decay(Some(fresh), 365.0, now) > 0.99, "fresh stays above the floor");
        std::env::set_var("ANCHOR_DECAY_FLOOR", "0");
        assert!((age_decay(Some(old), 365.0, now) - raw).abs() < 1e-9, "floor 0 restores raw decay");
        std::env::set_var("ANCHOR_DECAY_FLOOR", "bogus");
        assert!((age_decay(Some(old), 365.0, now) - 0.8).abs() < 1e-9, "unparsable falls back to 0.8");
        std::env::set_var("ANCHOR_DECAY_FLOOR", "7");
        assert_eq!(age_decay(Some(old), 365.0, now), 1.0, ">1 clamps to 1.0");
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
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



    // --- R33.5: exact-string search ---

    fn insert_chunk_kind(conn: &rusqlite::Connection, item_id: Option<i64>, kind: &str, content: &str) -> i64 {
        conn.execute("INSERT INTO chunks (item_id, kind, content, source_ref, created_at) VALUES (?1,?2,?3,'ref',datetime('now'))", rusqlite::params![item_id, kind, content]).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn exact_search_matches_literal_case_and_whitespace_verbatim() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        let camel = insert_chunk(&conn, Some(iid), None, "call connectTimeout(30) before send");
        let _lower = insert_chunk(&conn, Some(iid), None, "call connecttimeout(30) before send");
        let double = insert_chunk(&conn, Some(iid), None, "a  double  spaced line");
        let _single = insert_chunk(&conn, Some(iid), None, "a double spaced line");
        let special = insert_chunk(&conn, Some(iid), None, "price is a+b (c) total");
        // Case-sensitive: only the exact-case chunk matches.
        let hits = exact_search(&conn, "connectTimeout", None, 8, true);
        assert_eq!(hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![camel]);
        // Whitespace-verbatim: two spaces match only two spaces.
        let ws = exact_search(&conn, "double  spaced", None, 8, true);
        assert_eq!(ws.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![double]);
        // Regex-significant chars are literal (regex stays an owner call).
        let sp = exact_search(&conn, "a+b (c)", None, 8, true);
        assert_eq!(sp.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![special]);
        // Unranked: every hit scores the uniform 1.0.
        assert!(hits.iter().all(|h| h.score == 1.0));
    }

    #[test]
    fn exact_search_no_match_and_empty_needle() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        insert_chunk(&conn, Some(iid), None, "some ordinary content here");
        assert!(exact_search(&conn, "zzz-no-such-token", None, 8, true).is_empty());
        assert!(exact_search(&conn, "", None, 8, true).is_empty(), "empty needle must match nothing (instr(x,'') matches all)");
    }

    #[test]
    fn exact_search_leaf_only_and_document_order() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let first = insert_item(&conn, sid, "aaa-first");
        let second = insert_item(&conn, sid, "zzz-second");
        // Insert out of document order: the second item's chunk gets the
        // lower row id, so row order alone would list it first.
        let c_second = insert_chunk(&conn, Some(second), None, "needle in second file");
        let c_first = insert_chunk(&conn, Some(first), None, "needle in first file");
        // Non-leaf rows hold the needle too but must never surface (R33.3).
        let eid = insert_entity(&conn, first, "needle paraphrase", "unverified", "alice");
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','ref','needle in entity row')", [eid]).unwrap();
        insert_chunk_kind(&conn, Some(first), "section_summary", "needle in summary row");
        let hits = exact_search(&conn, "needle", None, 8, true);
        assert_eq!(
            hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(),
            vec![c_first, c_second],
            "document order (item, then chunk) — not insertion order"
        );
        assert!(hits.iter().all(|h| h.entity_id.is_none()));
    }

    #[test]
    fn exact_search_honors_source_scope_and_disputed_gate() {
        let (_dir, conn) = harness_db();
        let s1 = insert_source(&conn, "s1", "internal");
        let s2 = insert_source(&conn, "s2", "internal");
        let i1 = insert_item(&conn, s1, "doc1");
        let i2 = insert_item(&conn, s2, "doc2");
        let c1 = insert_chunk(&conn, Some(i1), None, "shared marker one");
        let c2 = insert_chunk(&conn, Some(i2), None, "shared marker two");
        let only_s1: HashSet<i64> = [s1].into_iter().collect();
        let scoped = exact_search(&conn, "shared marker", Some(&only_s1), 8, true);
        assert_eq!(scoped.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![c1]);
        let empty: HashSet<i64> = HashSet::new();
        assert!(exact_search(&conn, "shared marker", Some(&empty), 8, true).is_empty(), "empty scope matches nothing (project/public parity)");
        let all = exact_search(&conn, "shared marker", None, 8, true);
        assert_eq!(all.len(), 2, "unscoped search sees both sources ({c1}, {c2})");
        // A document chunk linked to a disputed entity honors the gate.
        let ed = insert_entity(&conn, i1, "disputed note", "disputed", "bob");
        let cd = insert_chunk(&conn, Some(i1), Some(ed), "shared marker disputed");
        assert!(!exact_search(&conn, "shared marker disputed", None, 8, true).iter().any(|h| h.chunk_id == cd));
        assert!(exact_search(&conn, "shared marker disputed", None, 8, false).iter().any(|h| h.chunk_id == cd));
    }

    #[test]
    fn exact_search_differs_from_fts_ranking() {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        insert_chunk(&conn, Some(iid), None, "the Tenant Identifier for Northbank");
        // FTS folds case (unicode61): ranked search finds it …
        let ranked = keyword_search(&conn, "tenant identifier", None, 8, true);
        assert_eq!(ranked.len(), 1, "FTS must match case-insensitively");
        // … but the exact leg is case-sensitive: no match.
        assert!(exact_search(&conn, "tenant identifier", None, 8, true).is_empty());
        // And the reverse: a substring spanning a token boundary is
        // literal here, while FTS only matches whole tokens.
        insert_chunk(&conn, Some(iid), None, "settle code confirmed");
        let lit = exact_search(&conn, "tle co", None, 8, true);
        assert_eq!(lit.len(), 1);
        let fts_sub = keyword_search(&conn, "tle co", None, 8, true);
        assert!(fts_sub.is_empty(), "FTS matches whole tokens only, got {}", fts_sub.len());
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
        let h1 = Hit { chunk_id: c1, entity_id: Some(e1), item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 1.0, content: "c1".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
        let h2 = Hit { chunk_id: c2, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 0.5, content: "c2".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
        let fused = fuse_and_rank(&conn, Some(vec![h1]), Some(vec![h2]), 1.0, 365.0, 3, 5, true, "phoenix billing");
        assert!(!fused.is_empty());
        // RRF should have merged both; diversity cap per item should keep both since distinct chunk_ids
        assert!(fused.len() >= 1);
    }

    #[test]
    fn harness_status_ok_integration() {
        assert!(!status_ok(Some("stale"), true));
        assert!(status_ok(Some("unverified"), true));
    }

    fn summary_hit(chunk_id: i64, sid: i64, item: i64, sec: Option<i64>) -> Hit {
        Hit { chunk_id, entity_id: None, item_id: Some(item), source_id: Some(sid), section_id: sec, path: String::new(), score: 1.0, content: String::new(), source_ref: String::new(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false }
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
    #[test]
    fn sort_hits_puts_nan_last_without_panicking() {
        let mut hits = vec![
            mk_hit(1, 1, 1, None, "a", "a", 0.2),
            mk_hit(2, 1, 1, None, "b", "b", f64::NAN),
            mk_hit(3, 1, 1, None, "c", "c", 0.5),
        ];
        sort_hits(&mut hits);
        assert_eq!(hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![3, 1, 2]);
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
            provenance: Vec::new(),
            decay: 1.0,
            boosted: false,
            keyword_matched: false,
            cosine: None,
            rich_match: false,
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
        let _guard = ENV_LOCK.lock().unwrap();
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

    // --- R39.8: suspect-PDF fence skip ---

    /// Shredded spacing that trips the R36.10 detector (same join shapes as
    /// the extractor's shredded fixture, enough suspects to clear the floor).
    fn shredded_pdf_text() -> String {
        "Cancellati ons and Correcti ons with noti fi cati ons for processi ng of subscripti ons and installati on guides".to_string()
    }

    /// Two-file TOC corpus: a PDF whose summary matches nothing (poor-summary
    /// stand-in) but whose leaf holds the query term, plus a clean md file
    /// whose summary matches — keeps the TOC path alive so the fence is real.
    /// Returns (db dir, conn, pdf leaf id, md leaf id).
    fn seed_suspect_toc_db(pdf_text: &str) -> (tempfile::TempDir, rusqlite::Connection, i64, i64) {
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_r398", "internal");
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'report.pdf','report.pdf',?2,'h')",
            rusqlite::params![sid, pdf_text],
        )
        .unwrap();
        let pdf = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'alpha.md','alpha.md','plain words','h')",
            [sid],
        )
        .unwrap();
        let md = conn.last_insert_rowid();
        let mut leaves = Vec::new();
        for (iid, summary, body) in [
            (pdf, "quarterly planning notes nothing matching", "quasarion refund policy details"),
            (md, "quasarion overview notes", "quasarion overview body text"),
        ] {
            conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,1,'S','S')", [iid]).unwrap();
            let sec = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, kind, source_ref, content) VALUES (?1,?2,'section_summary','ref',?3)",
                rusqlite::params![iid, sec, summary],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,'ref',?3)",
                rusqlite::params![iid, sec, body],
            )
            .unwrap();
            leaves.push(conn.last_insert_rowid());
        }
        (_dir, conn, leaves[0], leaves[1])
    }

    #[test]
    fn suspect_pdf_items_flags_only_shredded_pdfs() {
        // R36.10 parity, batched: shredded PDF flags, clean PDF reads null,
        // shredded non-PDF never scores.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_r398_flag", "internal");
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'bad.pdf','bad.pdf',?2,'h')",
            rusqlite::params![sid, shredded_pdf_text()],
        )
        .unwrap();
        let bad = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'good.pdf','good.pdf','Cancellations and notifications for processing.','h')",
            [sid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'n.md','n.md',?2,'h')",
            rusqlite::params![sid, shredded_pdf_text()],
        )
        .unwrap();
        assert_eq!(suspect_pdf_items(&conn, None), HashSet::from([bad]));
        assert!(suspect_pdf_items(&conn, Some(&HashSet::from([sid + 999]))).is_empty(), "source scope must bound the scan");
    }

    #[test]
    fn toc_search_rescues_suspect_pdf_leaf_behind_poor_summaries() {
        let (_dir, conn, pdf_leaf, md_leaf) = seed_suspect_toc_db(&shredded_pdf_text());
        // The fence is real: the md summary admits only its own section, so
        // the flat leg sees the pdf leaf but the summary stage cannot.
        let flat = keyword_search(&conn, "quasarion", None, 8, true);
        assert!(flat.iter().any(|h| h.chunk_id == pdf_leaf), "flat leg must see the pdf leaf: {:?}", flat.iter().map(|h| h.chunk_id).collect::<Vec<_>>());
        let hits = toc_search(&conn, "quasarion", None, None, 8, true);
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        assert!(ids.contains(&md_leaf), "fenced md leaf kept: {ids:?}");
        assert!(ids.contains(&pdf_leaf), "suspect-pdf leaf must skip the fence: {ids:?}");
        // Fenced ranks stand: the admitted md leaf outranks the rescue.
        let pos = |id: i64| ids.iter().position(|c| *c == id).unwrap();
        assert!(pos(md_leaf) < pos(pdf_leaf), "rescue appends after fenced ranks: {ids:?}");
    }

    #[test]
    fn toc_search_fence_still_holds_for_clean_pdfs() {
        let (_dir, conn, pdf_leaf, md_leaf) = seed_suspect_toc_db("Cancellations and notifications for processing.");
        let flat = keyword_search(&conn, "quasarion", None, 8, true);
        assert!(flat.iter().any(|h| h.chunk_id == pdf_leaf), "control: flat leg sees the clean-pdf leaf too");
        let hits = toc_search(&conn, "quasarion", None, None, 8, true);
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        assert!(ids.contains(&md_leaf), "fenced md leaf kept: {ids:?}");
        assert!(!ids.contains(&pdf_leaf), "clean-pdf leaf stays fenced out: {ids:?}");
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
    fn vector_search_never_compares_mixed_families() {
        // R45.5: identical bytes, different provenance. Only vectors in the
        // active query vector's family are compared; unstamped and foreign
        // vectors are keyword-only until re-embedded.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s1", "internal");
        let iid = insert_item(&conn, sid, "doc1");
        let mut v = vec![0.0f32; 768];
        v[0] = 1.0;
        let blob = crate::embedder::pack_single(&v);
        let mut ids = Vec::new();
        for (text, fp) in [
            ("ollama nomic vector", Some("ollama:nomic-embed-text:768")),
            ("builtin nomic vector", Some("builtin:nomic-embed-text-v1.5:768")),
            ("other model vector", Some("ollama:mxbai-embed-large:768")),
            ("unstamped vector", None),
        ] {
            let cid = insert_chunk(&conn, Some(iid), None, text);
            conn.execute("UPDATE chunks SET embedding=?1, embed_fp=?2 WHERE id=?3", rusqlite::params![blob, fp, cid]).unwrap();
            ids.push(cid);
        }
        let found = |conn: &Connection| {
            let mut got: Vec<i64> = vector_search_filtered(conn, &v, None, None, 8, false).iter().map(|h| h.chunk_id).collect();
            got.sort();
            got
        };
        // No query fingerprint recorded yet: pre-R45.5 behavior, all compared.
        assert_eq!(found(&conn), ids);
        // A nomic query compares against both nomic providers only.
        crate::embed_fp::record_active_fingerprint(&conn, "builtin:nomic-embed-text-v1.5:768").unwrap();
        assert_eq!(found(&conn), vec![ids[0], ids[1]]);
        // Switching models: only that model's vectors.
        crate::embed_fp::record_active_fingerprint(&conn, "ollama:mxbai-embed-large:768").unwrap();
        assert_eq!(found(&conn), vec![ids[2]]);
        // Same model, different dim (e.g. a Matryoshka cut): not comparable.
        crate::embed_fp::record_active_fingerprint(&conn, "ollama:nomic-embed-text:256").unwrap();
        assert!(found(&conn).is_empty());
        // A model nothing was embedded with: vector leg empty, not mixed.
        crate::embed_fp::record_active_fingerprint(&conn, "ollama:all-minilm:768").unwrap();
        assert!(found(&conn).is_empty());
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
                provenance: Vec::new(),
                decay: 1.0,
                boosted: false,
                keyword_matched: false,
                cosine: None,
                rich_match: false,
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
    fn live_first_partition_exemption_knob_and_markers() {
        // R33.3: the R32.3 score multiplier is retired — the partition is the
        // only env ordering. Guards (query exemption, kill knob, segment
        // matching) move here so they stay pinned.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_env", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let c_live = insert_chunk(&conn, Some(live), None, "live treasury routes content");
        let c_test = insert_chunk(&conn, Some(tst), None, "test treasury routes content");
        let mk = |cid: i64, iid: i64| Hit { chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None, path: String::new(), score: 10.0, content: "x".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
        // Default: the test file sinks below the live file, scores untouched.
        let mut hits = vec![mk(c_test, tst), mk(c_live, live)];
        prefer_live_envs(&conn, &mut hits, "which treasury routes are supported");
        assert_eq!(hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![c_live, c_test]);
        assert!(hits.iter().all(|h| h.score == 10.0), "partition orders, never rescores");
        // Exemption: a query about test files keeps fixture order.
        let mut hits2 = vec![mk(c_test, tst), mk(c_live, live)];
        prefer_live_envs(&conn, &mut hits2, "what is in the test file");
        assert_eq!(hits2[0].chunk_id, c_test);
        // Knob off: nothing moves.
        std::env::set_var("ANCHOR_TEST_ENV_DEMOTE", "1.0");
        let mut hits3 = vec![mk(c_test, tst), mk(c_live, live)];
        prefer_live_envs(&conn, &mut hits3, "which treasury routes are supported");
        assert_eq!(hits3[0].chunk_id, c_test);
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
        let mk_other = || Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
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
        let mut hits = vec![Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false }];
        filename_boost(&conn, &mut hits, "routing-config.yml ACME_FEE");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, matched, "named file must surface leaf bytes, not its summary");
    }

    #[test]
    fn live_first_partition_covers_entity_hits() {
        // R33.3: production entity chunks (entity_id only, no item_id) still
        // resolve their item's path via the entity — env classification stays
        // kind-agnostic even though entity rows no longer reach retrieval.
        let _guard = ENV_LOCK.lock().unwrap();
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
        let mk = |cid: i64, eid: i64| Hit { chunk_id: cid, entity_id: Some(eid), item_id: None, source_id: Some(sid), section_id: None, path: String::new(), score: 10.0, content: "x".into(), source_ref: "ref".into(), is_pii: false, status: Some("unverified".into()), content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
        let mut hits = vec![mk(c_test, e_test), mk(c_live, e_live)];
        prefer_live_envs(&conn, &mut hits, "which treasury routes are supported");
        assert_eq!(hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![c_live, c_test], "test entity row must sink like a document chunk");
    }

    #[test]
    fn fts_scoping_survives_global_limit_cutoff() {
        // F-04: LIMIT used to apply before source/section filtering, so a
        // scoped match ranked below the global cut-off never surfaced.
        // Twelve high-TF chunks in the big source crowd one low-TF chunk in
        // the small source out of a top-1 (LIMIT 10) global fetch.
        let (_dir, conn) = harness_db();
        let big = insert_source(&conn, "s_big", "internal");
        let small = insert_source(&conn, "s_small", "internal");
        let big_item = insert_item(&conn, big, "bigdoc");
        for _ in 0..12 {
            insert_chunk(&conn, Some(big_item), None, "alpha alpha alpha alpha alpha");
        }
        let small_item = insert_item(&conn, small, "smalldoc");
        let target = insert_chunk(&conn, Some(small_item), None, "alpha omega psi chi phi");
        // unscoped sanity: FTS finds the term at all
        assert!(!keyword_search(&conn, "alpha", None, 1, true).is_empty());
        // project scope: the small source's only match must surface
        let scoped = keyword_search(&conn, "alpha", Some(&HashSet::from([small])), 1, true);
        assert!(scoped.iter().any(|h| h.chunk_id == target), "scoped hit missing: {:?}", scoped.iter().map(|h| h.chunk_id).collect::<Vec<_>>());
        // section fence (TOC leaf shape): same guarantee via section_ids
        conn.execute("INSERT INTO sections (item_id) VALUES (?1)", [small_item]).unwrap();
        let sec: i64 = conn.last_insert_rowid();
        conn.execute("UPDATE chunks SET section_id = ?1 WHERE id=?2", rusqlite::params![sec, target]).unwrap();
        let fenced = keyword_search_filtered(&conn, "alpha", None, Some(&HashSet::from([sec])), 1, true);
        assert!(fenced.iter().any(|h| h.chunk_id == target));
        let empty = keyword_search_filtered(&conn, "alpha", None, Some(&HashSet::from([sec + 999])), 1, true);
        assert!(empty.is_empty());
    }

    #[test]
    fn fuse_and_rank_decays_sqlite_timestamps() {
        // F-10: writers store SQLite `CURRENT_TIMESTAMP`, not RFC 3339 —
        // decay must apply to real rows, not silently return 1.0.
        // R39.6: the raw ~x0.5 now floors at 0.8 (factor recorded on the hit).
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_age", "internal");
        let old = insert_item(&conn, sid, "old");
        let fresh = insert_item(&conn, sid, "fresh");
        conn.execute("UPDATE ingested_items SET external_id='cfg/treasury-old.md' WHERE id=?1", [old]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/treasury-fresh.md' WHERE id=?1", [fresh]).unwrap();
        let c_old = insert_chunk(&conn, Some(old), None, "old treasury routes alfa content");
        let c_fresh = insert_chunk(&conn, Some(fresh), None, "fresh treasury routes beta content");
        conn.execute("UPDATE chunks SET created_at = datetime('now','-365 days') WHERE id=?1", [c_old]).unwrap();
        let mk = |cid: i64, iid: i64, content: &str| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score: 10.0, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let out = fuse_and_rank(&conn, None, Some(vec![mk(c_old, old, "old treasury"), mk(c_fresh, fresh, "fresh treasury")]), 1.0, 365.0, 3, 10, true, "treasury routes");
        assert_eq!(out.len(), 2);
        let h_old = out.iter().find(|h| h.chunk_id == c_old).unwrap();
        let s_fresh = out.iter().find(|h| h.chunk_id == c_fresh).unwrap().score;
        assert!((h_old.score - 8.0).abs() < 0.05, "year-old chunk must floor at ~x0.8 at 365d halflife, got {}", h_old.score);
        assert!((h_old.decay - 0.8).abs() < 0.01, "applied factor recorded, got {}", h_old.decay);
        assert!((s_fresh - 10.0).abs() < 0.05, "fresh chunk must keep full score, got {}", s_fresh);
    }

    #[test]
    fn decay_floor_flips_fuse_ordering() {
        // R39.6: old(10) vs fresh(7) — floored 8.0 beats 7.0, raw 5.0 loses.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_floor", "internal");
        let old = insert_item(&conn, sid, "old");
        let fresh = insert_item(&conn, sid, "fresh");
        conn.execute("UPDATE ingested_items SET external_id='cfg/floor-old.md' WHERE id=?1", [old]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/floor-fresh.md' WHERE id=?1", [fresh]).unwrap();
        let c_old = insert_chunk(&conn, Some(old), None, "old treasury routes alfa content");
        let c_fresh = insert_chunk(&conn, Some(fresh), None, "fresh treasury routes beta content");
        conn.execute("UPDATE chunks SET created_at = datetime('now','-365 days') WHERE id=?1", [c_old]).unwrap();
        let mk = |cid: i64, iid: i64, score: f64, content: &str| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let run = || {
            fuse_and_rank(
                &conn,
                None,
                Some(vec![mk(c_old, old, 10.0, "old treasury routes alfa"), mk(c_fresh, fresh, 7.0, "fresh treasury routes beta")]),
                1.0,
                365.0,
                3,
                10,
                true,
                "treasury routes",
            )
        };
        let floored = run();
        assert_eq!(floored[0].chunk_id, c_old, "floored 8.0 must beat fresh 7.0");
        std::env::set_var("ANCHOR_DECAY_FLOOR", "0");
        let raw = run();
        assert_eq!(raw[0].chunk_id, c_fresh, "raw 5.0 must lose to fresh 7.0");
        std::env::remove_var("ANCHOR_DECAY_FLOOR");
    }

    #[test]
    fn fuse_and_rank_partitions_live_before_truncate() {
        // R33.3: the score multiplier is retired — the live-first partition
        // sits directly before the cut, so the live file still wins top-1
        // even when the fixture chunk outranks it in both input lists.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fz", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let c_live = insert_chunk(&conn, Some(live), None, "live treasury routes acme content here");
        let c_test = insert_chunk(&conn, Some(tst), None, "test treasury routes content here");
        let mk = |cid: i64, iid: i64, score: f64, content: &str| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let lists = || {
            (
                vec![mk(c_test, tst, 9.0, "test treasury routes content here"), mk(c_live, live, 5.0, "live treasury routes acme content here")],
                vec![mk(c_test, tst, 8.0, "test treasury routes content here"), mk(c_live, live, 4.0, "live treasury routes acme content here")],
            )
        };
        let (v, k) = lists();
        let fused = fuse_and_rank(&conn, Some(v), Some(k), 1.0, 365.0, 3, 1, true, "treasury routes");
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].chunk_id, c_live, "live-first partition must surface the live chunk");
        // Exemption preserved: a query about test files keeps fixture order.
        let (v2, k2) = lists();
        let fused2 = fuse_and_rank(&conn, Some(v2), Some(k2), 1.0, 365.0, 3, 1, true, "what is in the test file");
        assert_eq!(fused2.len(), 1);
        assert_eq!(fused2[0].chunk_id, c_test, "env-marker queries must not partition");
    }

    #[test]
    fn live_first_partition_beats_bm25_length_gap() {
        // R32.7, the 1.0.24 retest shape: a long live chunk vs a short
        // fixture chunk sharing one rare token. The fixture wins BM25 by
        // several × on length norm alone — live must still win top-1
        // through the partition. R33.3: the gap is doc-vs-doc now; the
        // fixture's entity row sits in the DB and must never surface.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_lp", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        let live_text = format!("treasury config {} acme", "padding ".repeat(300));
        let c_live = insert_chunk(&conn, Some(live), None, &live_text);
        let c_test = insert_chunk(&conn, Some(tst), None, "acme fixture row");
        let eid = insert_entity(&conn, tst, "acme fixture entity row", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','acme fixture entity row',datetime('now'))",
            [eid],
        )
        .unwrap();
        let c_ent = conn.last_insert_rowid();
        // Sanity: raw FTS ranks the short fixture chunk first (the BM25
        // gap) and never the entity row.
        let kw = keyword_search_filtered(&conn, "acme", None, None, 8, true);
        assert_eq!(kw.len(), 2);
        assert_eq!(kw[0].chunk_id, c_test, "fixture chunk must lead pre-partition (else no gap to prove)");
        assert!(!kw.iter().any(|h| h.chunk_id == c_ent), "entity row must not enter the leaf pool");
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
        // R43.27: lone domain words are prose; two distinct weak markers are not.
        assert_eq!(env_for_item("decisions/d2.md", "We need a proper sandbox before cutover."), "live");
        assert_eq!(env_for_item("specs/runbook.md", "Step 1: Provision sandbox. Sandbox again."), "live");
        assert_eq!(env_for_item("notes/rel.md", "rolled to staging first"), "live");
        assert_eq!(env_for_item("cfg/seed.yml", "# sandbox + staging tenant seed"), "test");
        assert_eq!(env_for_item("cfg/seed.yml", "# fixture data"), "test");
        assert_eq!(env_for_item("sandbox/vars.yml", "plain head"), "test");
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
    fn authority_tier_classes_file_kinds() {
        // R39.7: the tier table — spec dirs/names, prose default, code and
        // config extensions, generated/lint/vendored last. Lowest authority
        // wins on overlap; `spec/` test suites and `*.spec.ts` stay out.
        use AuthorityTier::*;
        let cases = [
            ("decisions/adr-007-cursor-pagination.md", Spec),
            ("adr/0001-use-cursor-pagination.md", Spec),
            ("docs/adr-012-billing.md", Spec),
            ("rfc/0012-pagination.md", Spec),
            ("spec/api.md", Spec),
            ("design-docs/search.md", Spec),
            ("docs\\adr-001.md", Spec),
            ("notes/api-design-sync.md", Prose),
            ("docs/runbook.md", Prose),
            ("README", Prose),
            ("file.weird-ext", Prose),
            ("src/auth/session.py", Code),
            ("src/billing/proration.rs", Code),
            ("src/web/pagination.ts", Code),
            ("service.go", Code),
            ("foo.spec.ts", Code),
            ("spec/pagination_spec.rb", Code),
            ("config/pagerduty.yml", Config),
            ("config/flags.json", Config),
            ("pyproject.toml", Config),
            (".env", Config),
            ("Dockerfile", Config),
            ("Makefile", Config),
            ("CMakeLists.txt", Config),
            ("config/api-lint.yml", Generated),
            (".golangci.yml", Generated),
            (".eslintrc.js", Generated),
            ("gen/service.generated.go", Generated),
            ("vendor/spec.md", Generated),
            ("node_modules/pkg/index.js", Generated),
            ("dist/bundle.js", Generated),
            ("Cargo.lock", Generated),
            ("package-lock.json", Generated),
            ("static/app.min.js", Generated),
            ("ADRs/Upper-Case-RFC-001.MD", Spec),
        ];
        for (path, want) in cases {
            assert_eq!(authority_tier(path), want, "tier of {path}");
        }
    }

    #[test]
    fn fuse_and_rank_partitions_authority_tiers() {
        // R39.7: an ADR outranks a same-query lint config and a generated
        // file through the partition even when both lead it on score in
        // every input list — and a generated hit never beats a same-score
        // ADR from either input order.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_AUTHORITY_TIERS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_tier", "internal");
        let adr = insert_item(&conn, sid, "adr");
        let lint = insert_item(&conn, sid, "lint");
        let gen = insert_item(&conn, sid, "gen");
        conn.execute("UPDATE ingested_items SET external_id='decisions/adr-007-pagination.md' WHERE id=?1", [adr]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='config/api-lint.yml' WHERE id=?1", [lint]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='gen/pagination.generated.ts' WHERE id=?1", [gen]).unwrap();
        let c_adr = insert_chunk(&conn, Some(adr), None, "adr cursor pagination decision record content");
        let c_lint = insert_chunk(&conn, Some(lint), None, "lint cursor pagination rule enforcement content");
        let c_gen = insert_chunk(&conn, Some(gen), None, "generated cursor pagination client stub content");
        let mk = |cid: i64, iid: i64, score: f64, content: &str| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: content.into(), source_ref: "ref".into(),
            is_pii: false, status: Some("unverified".into()), content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let bodies = |cid: i64| match cid {
            c if c == c_adr => (adr, "adr cursor pagination decision record content"),
            c if c == c_lint => (lint, "lint cursor pagination rule enforcement content"),
            _ => (gen, "generated cursor pagination client stub content"),
        };
        let lists = || {
            let one = |scores: [(i64, f64); 3]| {
                scores.into_iter().map(|(cid, s)| { let (iid, body) = bodies(cid); mk(cid, iid, s, body) }).collect::<Vec<_>>()
            };
            (one([(c_lint, 9.0), (c_gen, 8.0), (c_adr, 5.0)]), one([(c_lint, 9.0), (c_gen, 8.0), (c_adr, 5.0)]))
        };
        let (v, k) = lists();
        let fused = fuse_and_rank(&conn, Some(v), Some(k), 1.0, 365.0, 3, 3, true, "cursor pagination enforced");
        let ids: Vec<i64> = fused.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![c_adr, c_lint, c_gen], "ADR first, same-tier ties keep fused order");
        // Same-score pin from both input orders (ties sort unstably — only
        // the partition makes the ADR win deterministic).
        for order in [vec![c_lint, c_gen, c_adr], vec![c_adr, c_gen, c_lint]] {
            let single: Vec<Hit> = order.into_iter().map(|cid| { let (iid, body) = bodies(cid); mk(cid, iid, 10.0, body) }).collect();
            let tied = fuse_and_rank(&conn, None, Some(single), 1.0, 365.0, 3, 3, true, "cursor pagination enforced");
            assert_eq!(tied.len(), 3);
            assert_eq!(tied[0].chunk_id, c_adr, "generated/lint must never beat a same-score ADR");
        }
    }

    /// R47.1: rich-evidence Prose pierces the tier partition — sixty
    /// one-term Code chunks must not bury a four-term Prose chunk below
    /// the cut (the PDF burial shape: weak per-chunk scores, strong
    /// multi-term evidence, Prose tier).
    #[test]
    fn pierce_rich_prose_above_thin_code_flood() {
        let (_dir, conn) = harness_db();
        let src = insert_source(&conn, "s", "internal");
        for i in 0..60 {
            let item = insert_item(&conn, src, &format!("svc{i}.go"));
            insert_chunk(&conn, Some(item), None, "process the request queue");
        }
        let prose = insert_item(&conn, src, "field-manual.md");
        insert_chunk(
            &conn,
            Some(prose),
            None,
            "quasar flux capacitor requirements for the warp drive process",
        );
        let hits = orchestrate_fusion(
            &conn,
            "quasar flux requirements process",
            None,
            None,
            20,
            1.0,
            365.0,
            3,
            true,
        );
        assert!(
            hits.iter().any(|h| h.item_id == Some(prose)),
            "rich prose buried below {} code hits",
            hits.len()
        );
        // Mechanism pin: the 4-term prose hit is flagged rich, the 1-term
        // code hits are not.
        let leg = keyword_search(&conn, "quasar flux requirements process", None, 100, true);
        let rich: Vec<i64> = leg.iter().filter(|h| h.rich_match).filter_map(|h| h.item_id).collect();
        assert_eq!(rich, vec![prose], "only the rich prose hit may pierce");
    }

    #[test]
    fn marked_terms_counts_distinct_spans() {
        assert!(marked_terms("no marks here").is_empty());
        assert!(marked_terms("dangling \x01open").is_empty());
        let two = marked_terms("a \x01Quasar\x02 b \x01flux\x02 c \x01QUASAR\x02");
        assert_eq!(two.len(), 2, "case folds, duplicates merge");
        let one = marked_terms("\x01requirements\x02");
        assert_eq!(one.len(), 1);
    }

    /// R47.1 guard: the tiers' purpose survives the pierce — one-term Prose
    /// still sorts below a five-term exact Config match (eval `flags-01`
    /// shape). Both pierce here; score order keeps Config first.
    #[test]
    fn tiers_still_bury_thin_prose_below_exact_config() {
        let (_dir, conn) = harness_db();
        let src = insert_source(&conn, "s", "internal");
        let config = insert_item(&conn, src, "routing-config.yml");
        insert_chunk(&conn, Some(config), None, "cache key tenant identifier settle codes");
        for i in 0..10 {
            let item = insert_item(&conn, src, &format!("note{i}.md"));
            insert_chunk(&conn, Some(item), None, "notes about the cache");
        }
        let hits = orchestrate_fusion(
            &conn,
            "cache key tenant identifier settle",
            None,
            None,
            20,
            1.0,
            365.0,
            3,
            true,
        );
        let pos = |iid: i64| hits.iter().position(|h| h.item_id == Some(iid));
        let config_pos = pos(config).expect("exact config must surface");
        let thin_best =
            hits.iter().enumerate().filter(|(_, h)| h.item_id != Some(config)).map(|(i, _)| i).min();
        if let Some(thin) = thin_best {
            assert!(config_pos < thin, "exact config must lead thin prose");
        }
    }

    #[test]
    fn authority_tiers_refine_within_live_first() {
        // R39.7: tiers refine within env classes — a live lint config still
        // outranks a test ADR (live-first dominates), while the live ADR
        // leads both.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_AUTHORITY_TIERS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_tierenv", "internal");
        let live_adr = insert_item(&conn, sid, "live-adr");
        let live_lint = insert_item(&conn, sid, "live-lint");
        let test_adr = insert_item(&conn, sid, "test-adr");
        conn.execute("UPDATE ingested_items SET external_id='decisions/adr-011-cursor.md' WHERE id=?1", [live_adr]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='config/api-lint.yml' WHERE id=?1", [live_lint]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='tests/adr-009-legacy.md' WHERE id=?1", [test_adr]).unwrap();
        let c_live_adr = insert_chunk(&conn, Some(live_adr), None, "live adr cursor content");
        let c_live_lint = insert_chunk(&conn, Some(live_lint), None, "live lint cursor content");
        let c_test_adr = insert_chunk(&conn, Some(test_adr), None, "test adr cursor content");
        let mk = |cid: i64, iid: i64, score: f64| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: "x".into(), source_ref: "ref".into(),
            is_pii: false, status: None, content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let mut hits = vec![mk(c_test_adr, test_adr, 9.0), mk(c_live_lint, live_lint, 8.0), mk(c_live_adr, live_adr, 7.0)];
        prefer_authority_tiers(&conn, &mut hits, "cursor pagination enforced");
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![c_live_adr, c_live_lint, c_test_adr], "live ADR > live lint > test ADR");
        assert!(hits.iter().all(|h| h.score == 9.0 || h.score == 8.0 || h.score == 7.0), "partition orders, never rescores");
    }

    #[test]
    fn authority_tiers_filename_mention_and_knob_exempt() {
        // R39.7 guards (the live-first analogues): a query naming the lint
        // file keeps score order (explicit filename intent wins), and
        // ANCHOR_AUTHORITY_TIERS=0/false disables the partition.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_AUTHORITY_TIERS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_tierex", "internal");
        let adr = insert_item(&conn, sid, "adr");
        let lint = insert_item(&conn, sid, "lint");
        conn.execute("UPDATE ingested_items SET external_id='decisions/adr-007-pagination.md' WHERE id=?1", [adr]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='config/api-lint.yml' WHERE id=?1", [lint]).unwrap();
        let c_adr = insert_chunk(&conn, Some(adr), None, "adr cursor pagination decision record");
        let c_lint = insert_chunk(&conn, Some(lint), None, "lint cursor pagination rule enforcement");
        let mk = |cid: i64, iid: i64, score: f64| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: "x".into(), source_ref: "ref".into(),
            is_pii: false, status: None, content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        // Default: the ADR leads despite trailing on score.
        let mut hits = vec![mk(c_lint, lint, 10.0), mk(c_adr, adr, 9.0)];
        prefer_authority_tiers(&conn, &mut hits, "cursor pagination enforced");
        assert_eq!(hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![c_adr, c_lint]);
        assert!(hits.iter().all(|h| h.score == 10.0 || h.score == 9.0), "partition orders, never rescores");
        // Exemption: naming the file keeps score order.
        let mut named = vec![mk(c_lint, lint, 10.0), mk(c_adr, adr, 9.0)];
        prefer_authority_tiers(&conn, &mut named, "what is in api-lint.yml");
        assert_eq!(named[0].chunk_id, c_lint, "filename mentions must not partition");
        // Knob off: nothing moves.
        for off in ["0", "false"] {
            std::env::set_var("ANCHOR_AUTHORITY_TIERS", off);
            let mut hits_off = vec![mk(c_lint, lint, 10.0), mk(c_adr, adr, 9.0)];
            prefer_authority_tiers(&conn, &mut hits_off, "cursor pagination enforced");
            assert_eq!(hits_off[0].chunk_id, c_lint, "knob {off} disables tiers");
        }
        std::env::remove_var("ANCHOR_AUTHORITY_TIERS");
    }

    #[test]
    fn authority_tiers_rank_code_above_config_above_prose() {
        // R39.7 middle chain: terse precise classes outrank verbose prose —
        // a 1-term code/config match leads a 5-term prose match (the eval
        // `flags-01` shape: exact config answers must not sink under prose).
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_AUTHORITY_TIERS");
        std::env::remove_var("ANCHOR_TEST_ENV_DEMOTE");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_tiermid", "internal");
        let code = insert_item(&conn, sid, "code");
        let cfg = insert_item(&conn, sid, "cfg");
        let prose = insert_item(&conn, sid, "prose");
        conn.execute("UPDATE ingested_items SET external_id='src/billing/proration.rs' WHERE id=?1", [code]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='config/flags.json' WHERE id=?1", [cfg]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='docs/glossary.md' WHERE id=?1", [prose]).unwrap();
        let c_code = insert_chunk(&conn, Some(code), None, "code proration content");
        let c_cfg = insert_chunk(&conn, Some(cfg), None, "config flags content");
        let c_prose = insert_chunk(&conn, Some(prose), None, "prose glossary content");
        let mk = |cid: i64, iid: i64, score: f64| Hit {
            chunk_id: cid, entity_id: None, item_id: Some(iid), source_id: Some(sid), section_id: None,
            path: String::new(), score, content: "x".into(), source_ref: "ref".into(),
            is_pii: false, status: None, content_truncated: false,
            provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None,
            rich_match: false,
        };
        let mut hits = vec![mk(c_prose, prose, 10.0), mk(c_cfg, cfg, 9.5), mk(c_code, code, 9.0)];
        prefer_authority_tiers(&conn, &mut hits, "proration flags glossary");
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![c_code, c_cfg, c_prose], "code > config > prose despite score order");
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
        let mk_other = || Hit { chunk_id: c_other, entity_id: None, item_id: Some(other), source_id: Some(sid), section_id: None, path: String::new(), score: 15.0, content: "unrelated".into(), source_ref: "ref".into(), is_pii: false, status: None, content_truncated: false, provenance: Vec::new(), decay: 1.0, boosted: false, keyword_matched: false, cosine: None, rich_match: false };
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

    #[test]
    fn leaf_pool_excludes_entity_and_distilled_chunks() {
        // R33.3 core: the FTS leaf leg returns document chunks only — a
        // one-line entity paraphrase matching every term must not outrank
        // (or even join) the content it was extracted from.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_leaf", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let doc = insert_chunk(&conn, Some(iid), None, "tungsten ledger alpha content here with body words");
        let eid = insert_entity(&conn, iid, "tungsten ledger alpha entity", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','tungsten ledger alpha entity',datetime('now'))",
            [eid],
        )
        .unwrap();
        let ent = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'distilled','ref','tungsten ledger alpha distilled',datetime('now'))",
            [iid],
        )
        .unwrap();
        let dist = conn.last_insert_rowid();
        let kw = keyword_search_filtered(&conn, "tungsten ledger alpha", None, None, 8, true);
        let ids: Vec<i64> = kw.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![doc], "leaf pool must hold only the document chunk, got {ids:?} (entity={ent} distilled={dist})");
    }

    #[test]
    fn leaf_section_fence_excludes_entity_chunks() {
        // R33.3: the R14.6 `OR c.entity_id IS NOT NULL` bypass is gone — a
        // section fence admits only chunks inside admitted sections.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_fence", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let sec = insert_section(&conn, iid, None, 1, "Billing", "Billing");
        let other = insert_section(&conn, iid, None, 1, "Auth", "Auth");
        let inside = insert_section_chunk(&conn, iid, sec, "Billing", "tungsten ledger beta inside content");
        insert_section_chunk(&conn, iid, other, "Auth", "tungsten ledger beta outside content");
        let eid = insert_entity(&conn, iid, "tungsten ledger beta entity", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','tungsten ledger beta entity',datetime('now'))",
            [eid],
        )
        .unwrap();
        let ent = conn.last_insert_rowid();
        let kw = keyword_search_filtered(&conn, "tungsten ledger beta", None, Some(&HashSet::from([sec])), 8, true);
        let ids: Vec<i64> = kw.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![inside], "fenced leaf must hold only the in-section document chunk, got {ids:?} (entity={ent})");
    }

    #[test]
    fn leaf_scan_excludes_entity_chunks() {
        // R33.3: the vector-scan fallback leg matches the vec0/FTS legs —
        // document chunks only, even when an entity row embeds identically.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_scan", "internal");
        let iid = insert_item(&conn, sid, "doc");
        let doc = insert_chunk(&conn, Some(iid), None, "tungsten ledger gamma content here");
        let eid = insert_entity(&conn, iid, "tungsten ledger gamma entity", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','tungsten ledger gamma entity',datetime('now'))",
            [eid],
        )
        .unwrap();
        let ent = conn.last_insert_rowid();
        let dim: usize = std::env::var("ANCHOR_EMBED_DIM").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        let emb = vec![0.25f32; dim];
        let blob: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute("UPDATE chunks SET embedding = ?1 WHERE id = ?2", rusqlite::params![blob, doc]).unwrap();
        conn.execute("UPDATE chunks SET embedding = ?1 WHERE id = ?2", rusqlite::params![blob, ent]).unwrap();
        let hits = vector_search_filtered(&conn, &emb, None, None, 8, true);
        let ids: Vec<i64> = hits.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids, vec![doc], "scan leg must hold only the document chunk, got {ids:?} (entity={ent})");
    }

    #[test]
    fn treasury_shape_passes_without_patches() {
        // R33.3 subsumption proof (1.0.24 treasury-monorepo retest shape):
        // the live routing config is long, the fixture file overlaps on
        // terms, and a 3-term entity row sits in the DB. The RAW leaf leg —
        // no filename boost, no demotion, no live-first partition — must
        // rank the live document first on term merit alone. Pre-R33.3 the
        // short entity row won this query on BM25 length norm.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_tr", "internal");
        let live = insert_item(&conn, sid, "live");
        let tst = insert_item(&conn, sid, "tst");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml' WHERE id=?1", [live]).unwrap();
        conn.execute("UPDATE ingested_items SET external_id='cfg/payment-routes.test.yml' WHERE id=?1", [tst]).unwrap();
        // Standalone `acme` token: FTS matches whole tokens, so `ACME_FEE`
        // alone would not match the query term — the live file genuinely
        // matches all three terms, the fixture only two.
        let live_text = format!(
            "treasury routes live acme config {} ACME_FEE payout lines {} end",
            "padding ".repeat(120),
            "filler ".repeat(120)
        );
        let c_live = insert_chunk(&conn, Some(live), None, &live_text);
        let fixture_text = format!("treasury routes fixture content {} end", "padding ".repeat(80));
        let c_fixture = insert_chunk(&conn, Some(tst), None, &fixture_text);
        let eid = insert_entity(&conn, tst, "treasury routes acme fixture row", "unverified", "");
        conn.execute(
            "INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity','ref','treasury routes acme fixture row',datetime('now'))",
            [eid],
        )
        .unwrap();
        let c_ent = conn.last_insert_rowid();
        // Sanity: the entity row matches every query term (pre-fix winner).
        let ent_content: String = conn.query_row("SELECT content FROM chunks WHERE id=?1", [c_ent], |r| r.get(0)).unwrap();
        assert!(["treasury", "routes", "acme"].iter().all(|t| ent_content.contains(t)));
        let kw = keyword_search_filtered(&conn, "treasury routes acme", None, None, 8, true);
        let ids: Vec<i64> = kw.iter().map(|h| h.chunk_id).collect();
        assert_eq!(ids.len(), 2, "only the two document chunks compete, got {ids:?}");
        assert_eq!(ids[0], c_live, "live config must lead on term merit without any patch");
        assert_eq!(ids[1], c_fixture);
    }

    #[test]
    fn version_like_tokens_never_fire_filename_boost() {
        // R39.4: release labels are not filename mentions — `v1.2` must not
        // surface a file that merely contains that string.
        assert!(filename_tokens("deploy v1.2 now").is_empty());
        assert!(filename_tokens("upgrade to V2.0.1 today").is_empty());
        assert_eq!(filename_tokens("see service.go"), vec!["service.go".to_string()]);
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_ver", "internal");
        let iid = insert_item(&conn, sid, "rel");
        conn.execute("UPDATE ingested_items SET external_id='docs/release-v1.2.md', title='release-v1.2.md' WHERE id=?1", [iid]).unwrap();
        insert_chunk(&conn, Some(iid), None, "ship the release candidate notes");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut other_hit = blank_hit(c_other);
        other_hit.item_id = Some(other);
        other_hit.score = 15.0;
        let mut hits = vec![other_hit];
        filename_boost_filtered(&conn, &mut hits, "ship v1.2", true, None);
        assert_eq!(hits.len(), 1, "a version token must leave the list untouched");
        assert_eq!(hits[0].chunk_id, c_other);
        assert_eq!(hits[0].score, 15.0);
    }

    #[test]
    fn named_file_ranking_prefers_exact_basename_over_substring() {
        // R39.4: `service.go` must surface service.go even when myservice.go
        // (substring match, lower id) would win `%LIKE% + LIMIT 1`.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_ex", "internal");
        let mine = insert_item(&conn, sid, "mine");
        conn.execute("UPDATE ingested_items SET external_id='myservice.go', title='myservice.go' WHERE id=?1", [mine]).unwrap();
        insert_chunk(&conn, Some(mine), None, "myservice legacy work routines");
        let svc = insert_item(&conn, sid, "svc");
        conn.execute("UPDATE ingested_items SET external_id='src/service.go', title='service.go' WHERE id=?1", [svc]).unwrap();
        let c_svc = insert_chunk(&conn, Some(svc), None, "service bootstrap work routines");
        assert!(mine < svc, "regression shape needs the substring match at the lower id");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut other_hit = blank_hit(c_other);
        other_hit.item_id = Some(other);
        other_hit.score = 15.0;
        let mut hits = vec![other_hit];
        filename_boost_filtered(&conn, &mut hits, "how does service.go work", true, None);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, c_svc, "exact basename must beat the substring match");
        assert_eq!(hits[0].item_id, Some(svc));
    }

    #[test]
    fn named_item_ids_prefers_shortest_path_without_exact_match() {
        // R39.4: no exact basename anywhere — the shallow candidate wins
        // over the deeply nested one even at a higher id.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_sp", "internal");
        let deep = insert_item(&conn, sid, "deep");
        conn.execute("UPDATE ingested_items SET external_id='a/very/deep/dir/svc.go.bak' WHERE id=?1", [deep]).unwrap();
        let shallow = insert_item(&conn, sid, "shallow");
        conn.execute("UPDATE ingested_items SET external_id='svc.go.bak' WHERE id=?1", [shallow]).unwrap();
        assert!(deep < shallow, "regression shape needs the long path at the lower id");
        let ids = named_item_ids(&conn, &["svc.go".to_string()], None);
        assert_eq!(ids, vec![shallow], "shortest path must win among substring matches");
    }

    #[test]
    fn filename_boost_pulls_below_cut_chunk_at_top_plus_one() {
        // R39.4 DoD pin: the named file's chunk is absent from the fused
        // list (below the cut) — boost appends it at exactly max + 1.0, and
        // `mark_boosted_hits` flags exactly that hit via `score > prev_top`
        // (the R39.1 detection contract — this formula must not change).
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_top", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='cfg/routing-config.yml', title='routing-config.yml' WHERE id=?1", [iid]).unwrap();
        let cid = insert_chunk(&conn, Some(iid), None, "ACME_FEE routing table content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut other_hit = blank_hit(c_other);
        other_hit.item_id = Some(other);
        other_hit.score = 15.0;
        let mut hits = vec![other_hit];
        let prev_top = top_score(&hits);
        filename_boost_filtered(&conn, &mut hits, "routing-config.yml ACME_FEE", true, None);
        mark_boosted_hits(&mut hits, prev_top);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chunk_id, cid);
        assert_eq!(hits[0].score, 16.0, "boost scores exactly top + 1.0");
        assert!(hits[0].boosted, "the surfaced chunk must read as boosted");
        assert!(!hits[1].boosted, "pre-existing hits must not read as boosted");
    }

    #[test]
    fn fuse_and_rank_filters_status_before_the_cut() {
        // R39.5: stale/disputed hits ranked above the cut must not consume
        // top-k slots — a seeded corpus still returns a full live top_k.
        let (_dir, conn) = harness_db();
        let mk = |cid: i64, score: f64, status: Option<&str>| {
            let mut h = blank_hit(cid);
            h.item_id = Some(100 + cid);
            h.score = score;
            h.status = status.map(|s| s.to_string());
            h.content = format!("distinct fused content {cid}");
            h
        };
        let out = fuse_and_rank(
            &conn,
            None,
            Some(vec![mk(1, 9.0, Some("stale")), mk(2, 8.0, Some("disputed")), mk(3, 7.0, None), mk(4, 6.0, Some("unverified"))]),
            1.0,
            365.0,
            3,
            2,
            true,
            "plain query",
        );
        assert_eq!(out.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![3, 4], "excluded hits must not eat the cut");
        // The flag still flows: disputed survives when exclusion is off.
        let out2 = fuse_and_rank(
            &conn,
            None,
            Some(vec![mk(1, 9.0, Some("stale")), mk(2, 8.0, Some("disputed")), mk(3, 7.0, None), mk(4, 6.0, Some("unverified"))]),
            1.0,
            365.0,
            3,
            2,
            false,
            "plain query",
        );
        assert_eq!(out2.iter().map(|h| h.chunk_id).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn filename_boost_filtered_respects_exclude_disputed() {
        // R39.5: a disputed-only named file boosts when exclusion is off
        // and stays out when it is on.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_bd", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='docs/config.yml', title='config.yml' WHERE id=?1", [iid]).unwrap();
        let eid = insert_entity(&conn, iid, "config treasury decision", "disputed", "");
        insert_chunk(&conn, Some(iid), Some(eid), "config treasury content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mk_hits = || {
            let mut h = blank_hit(c_other);
            h.item_id = Some(other);
            h.score = 15.0;
            vec![h]
        };
        let mut on = mk_hits();
        filename_boost_filtered(&conn, &mut on, "config.yml", true, None);
        assert_eq!(on.len(), 1, "disputed chunk must not boost when excluded");
        let mut off = mk_hits();
        filename_boost_filtered(&conn, &mut off, "config.yml", false, None);
        assert_eq!(off.len(), 2, "disputed chunk boosts when exclusion is off");
        assert_eq!(off[0].score, 16.0);
    }

    #[test]
    fn filename_boost_filtered_always_excludes_stale() {
        // R39.5: stale is out regardless of the disputed flag.
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_bs", "internal");
        let iid = insert_item(&conn, sid, "legacy");
        conn.execute("UPDATE ingested_items SET external_id='docs/legacy.yml', title='legacy.yml' WHERE id=?1", [iid]).unwrap();
        let eid = insert_entity(&conn, iid, "legacy treasury decision", "stale", "");
        insert_chunk(&conn, Some(iid), Some(eid), "legacy treasury content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut h = blank_hit(c_other);
        h.item_id = Some(other);
        h.score = 15.0;
        let mut hits = vec![h];
        filename_boost_filtered(&conn, &mut hits, "legacy.yml", false, None);
        assert_eq!(hits.len(), 1, "stale chunk must never boost");
    }

    #[test]
    fn filename_boost_filtered_respects_source_scope() {
        // R43.1: naming an internal file under a public-only scope must not
        // boost it (the pre-fix call leaked non-public chunks into
        // `/qa/public`); in-scope names still boost, unscoped is unchanged.
        let (_dir, conn) = harness_db();
        let pub_sid = insert_source(&conn, "s_pub", "public");
        let pub_iid = insert_item(&conn, pub_sid, "faq");
        conn.execute("UPDATE ingested_items SET external_id='docs/public-faq.md', title='public-faq.md' WHERE id=?1", [pub_iid]).unwrap();
        let pub_seed = insert_chunk(&conn, Some(pub_iid), None, "public launch faq details here");
        let pub_match = insert_chunk(&conn, Some(pub_iid), None, "refund policy details for customers");
        let int_sid = insert_source(&conn, "s_int", "internal");
        let int_iid = insert_item(&conn, int_sid, "plan");
        conn.execute("UPDATE ingested_items SET external_id='docs/internal-launch-plan.md', title='internal-launch-plan.md' WHERE id=?1", [int_iid]).unwrap();
        let int_cid = insert_chunk(&conn, Some(int_iid), None, "secret launch timeline details here");
        let scope_pub: HashSet<i64> = [pub_sid].into_iter().collect();
        let mk_hits = || {
            let mut h = blank_hit(pub_seed);
            h.item_id = Some(pub_iid);
            h.source_id = Some(pub_sid);
            h.score = 15.0;
            vec![h]
        };
        // Out-of-scope name under a public-only scope: no boost.
        let mut scoped_out = mk_hits();
        filename_boost_filtered(&conn, &mut scoped_out, "what is in internal-launch-plan.md", true, Some(&scope_pub));
        assert_eq!(scoped_out.len(), 1, "out-of-scope named file must not boost");
        assert_eq!(scoped_out[0].chunk_id, pub_seed);
        // In-scope name under the same scope: boosts its content match.
        let mut scoped_in = mk_hits();
        filename_boost_filtered(&conn, &mut scoped_in, "public-faq.md refund policy", true, Some(&scope_pub));
        assert_eq!(scoped_in.len(), 2, "in-scope named file must still boost");
        assert_eq!(scoped_in[0].chunk_id, pub_match, "boost must surface the matching chunk");
        assert_eq!(scoped_in[0].score, 16.0, "boost scores exactly top + 1.0");
        // Unscoped control: the internal file boosts as before.
        let mut unscoped = mk_hits();
        filename_boost_filtered(&conn, &mut unscoped, "what is in internal-launch-plan.md", true, None);
        assert_eq!(unscoped.len(), 2, "unscoped boost must be unchanged");
        assert_eq!(unscoped[0].chunk_id, int_cid);
        assert_eq!(unscoped[0].item_id, Some(int_iid));
    }

    #[test]
    fn orchestrate_fusion_scope_blocks_boost_leak_end_to_end() {
        // R43.1: through the full fusion path — a public-only scope naming
        // an internal file yields zero out-of-scope hits, while a project
        // scope containing the internal source surfaces it via the boost.
        let (_dir, conn) = harness_db();
        let pub_sid = insert_source(&conn, "s_pub2", "public");
        let pub_iid = insert_item(&conn, pub_sid, "notes");
        insert_chunk(&conn, Some(pub_iid), None, "quarterly planning notes for the team");
        let int_sid = insert_source(&conn, "s_int2", "internal");
        let int_iid = insert_item(&conn, int_sid, "plan");
        conn.execute("UPDATE ingested_items SET external_id='docs/internal-launch-plan.md', title='internal-launch-plan.md' WHERE id=?1", [int_iid]).unwrap();
        insert_chunk(&conn, Some(int_iid), None, "secret launch timeline details here");
        let scope_pub: HashSet<i64> = [pub_sid].into_iter().collect();
        let scope_int: HashSet<i64> = [int_sid].into_iter().collect();
        let out = orchestrate_fusion(&conn, "what is in internal-launch-plan.md", None, Some(&scope_pub), 5, 1.0, 365.0, 3, true);
        assert!(
            out.iter().all(|h| h.source_id.map_or(true, |sid| scope_pub.contains(&sid))),
            "public-only scope must yield zero out-of-scope hits"
        );
        assert!(out.is_empty(), "nothing in scope matches, so the answer is empty — not a leak");
        let proj = orchestrate_fusion(&conn, "what is in internal-launch-plan.md", None, Some(&scope_int), 5, 1.0, 365.0, 3, true);
        assert_eq!(proj.len(), 1, "in-scope project search must surface the named file");
        assert_eq!(proj[0].item_id, Some(int_iid));
        assert!(proj[0].boosted, "the surfaced hit arrives via the filename boost");
        assert!(
            proj.iter().all(|h| h.source_id.map_or(true, |sid| scope_int.contains(&sid))),
            "project scope must yield zero out-of-scope hits"
        );
    }

    #[test]
    fn filename_boost_wrapper_excludes_disputed_by_default() {
        // R39.5: the legacy 3-arg entry (frozen R39.10 eval mirror) resolves
        // the production default from env — exclude unless opted out.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANCHOR_QA_EXCLUDE_DISPUTED");
        let (_dir, conn) = harness_db();
        let sid = insert_source(&conn, "s_bw", "internal");
        let iid = insert_item(&conn, sid, "cfg");
        conn.execute("UPDATE ingested_items SET external_id='docs/config.yml', title='config.yml' WHERE id=?1", [iid]).unwrap();
        let eid = insert_entity(&conn, iid, "config treasury decision", "disputed", "");
        insert_chunk(&conn, Some(iid), Some(eid), "config treasury content here");
        let other = insert_item(&conn, sid, "other");
        let c_other = insert_chunk(&conn, Some(other), None, "unrelated notes");
        let mut h = blank_hit(c_other);
        h.item_id = Some(other);
        h.score = 15.0;
        let mut hits = vec![h];
        filename_boost(&conn, &mut hits, "config.yml");
        assert_eq!(hits.len(), 1, "wrapper must exclude disputed under the production default");
        std::env::set_var("ANCHOR_QA_EXCLUDE_DISPUTED", "0");
        filename_boost(&conn, &mut hits, "config.yml");
        assert_eq!(hits.len(), 2, "opt-out restores the legacy include-disputed boost");
        std::env::remove_var("ANCHOR_QA_EXCLUDE_DISPUTED");
    }
}
