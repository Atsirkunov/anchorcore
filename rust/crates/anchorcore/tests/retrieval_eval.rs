//! R39.10 — golden retrieval eval set.
//!
//! A dozen-document fixture corpus plus query→expected-doc pairs that pin
//! retrieval behavior while fusion is tuned (Phase 39). The runner reports
//! per-leg recall@k — vector leg, keyword leg, TOC leaf — **and** fused
//! recall, so each leg can be measured independently before tuning fusion.
//!
//! Harness idiom: this mirrors the repo's Rust-only retrieval harness
//! (`retrieval.rs` P0 3.1 `harness_db`: tempdir + real `db::init_db`, seed
//! rows with rusqlite, call the leg functions directly). The crate is
//! binary-only (no `lib` target, and Wave-1 forbids touching `src/`), so
//! the real `db` / `common` / `retrieval` modules are compiled into this
//! test target via `#[path]` — every leg call below is the production code,
//! not a reimplementation. (The R36.1 `bench_parent_child.py` idiom drives
//! the live `POST /qa/search` API instead; that surface only exposes fused
//! hits, so it cannot measure legs independently.)
//!
//! Run (from `rust/`):
//!   cargo test -p anchorcore --test retrieval_eval -- --nocapture  # prints the table
//!   EVAL_VERBOSE=1 cargo test -p anchorcore --test retrieval_eval -- --nocapture  # + ranked lists
//! Plain `cargo test` also runs the set (assertions gate CI; output captured).
//!
//! Design notes:
//! - Fixture DB: one source, one section per doc, one `document` leaf chunk
//!   + one `section_summary` chunk per doc, all embedded. Scoring is by
//!   owning item (`external_id`), at k=5 (`bench_parent_child.py` TOP_K).
//! - Embeddings are deterministic token-hash vectors (FNV-1a, dim 256, tf,
//!   L2-normalized) computed with the same function for chunks and queries.
//!   They are a *lexical proxy*, not semantics: they understate true vector
//!   recall on paraphrase queries. The eval measures leg plumbing + fusion,
//!   not model quality. Dim 256 ≠ `ANCHOR_EMBED_DIM` on purpose: the vec0
//!   path returns `None` on dim mismatch and the real scan fallback
//!   (`cosine > 0.2`) runs — exactly what production does on bundled
//!   rusqlite (no vec0 module), including the TOC summary→keyword fallback.
//! - Fused recall calls the production fusion path itself
//!   (`retrieval::orchestrate_fusion`, shared with `answer.rs::orchestrate`
//!   since R39.12 — the old `orchestrate_mirror` restatement is deleted, so
//!   the eval can never drift from production): TOC-or-flat legs → RRF fuse
//!   → `fuse_and_rank` → keyword slots → filename boost → `parent_return` →
//!   status re-filter → relevance gate, at production defaults
//!   (keyword_weight=1.0, halflife=365d, max_per_source=3, exclude
//!   disputed). The shared fn re-runs the same leg functions on the same
//!   inputs, so it fuses the same outputs the table reports.
//! - The set asserts the golden bar (fused recall 1.0 on every pair) plus
//!   the R39.2 signature (bare opaque codes are keyword-floor-only); the
//!   per-leg columns are reported, not asserted, so fusion tuning has room.
//! - Corpus + pairs live in `tests/eval_data/` (`eval.json` manifest,
//!   `corpus/` docs). See `tests/eval_data/README.md` for how to add pairs.

// The eval uses a subset of each production module (answer.rs owns the rest
// in the real binary); silence dead-code for the includes only — the eval's
// own code below stays strict so `-D warnings` still guards it.
#[allow(dead_code)]
#[path = "../src/db.rs"]
mod db;

#[allow(dead_code)]
#[path = "../src/common.rs"]
mod common;

#[allow(dead_code)]
#[path = "../src/retrieval.rs"]
mod retrieval;

/// Minimal `embedder` shim: `retrieval.rs`'s inner unit-test module is
/// compiled into this target (`cfg(test)` is set for test builds) and calls
/// `crate::embedder::pack_single`. This is byte-identical to
/// `src/embedder.rs::pack_single` (little-endian f32 packing); including the
/// real file would drag the embedder→settings→secrets cascade into the eval
/// target for no benefit. Fixture embeddings are packed here too.
mod embedder {
    pub fn pack_single(vector: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(vector.len() * 4);
        for v in vector {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

/// Recall depth: legs and fusion are all called and scored at k=5.
const K: usize = 5;
/// Hash-embedding dim. Must only be self-consistent (chunks + queries);
/// 256 deliberately differs from `ANCHOR_EMBED_DIM` so the vec0 path
/// declines and the production scan fallback runs (see module docs).
const DIM: usize = 256;
const EVAL_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/eval_data");

/// Stopwords for the hash tokenizer — mirrors `retrieval.rs::query_terms`
/// (private) so the lexical proxy drops the same glue words FTS drops.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for", "with",
    "about", "is", "are", "was", "were", "be", "been", "being", "am", "do", "does", "did",
    "have", "has", "had", "will", "would", "can", "could", "should", "shall", "may",
    "might", "must", "what", "which", "who", "whom", "whose", "when", "where", "why", "how",
    "this", "that", "these", "those", "it", "its", "not", "no", "so", "if", "then", "than",
    "too", "very", "s", "t", "you", "your", "we", "our", "they", "their", "i", "me", "my",
];

fn is_stopword(token: &str) -> bool {
    STOPWORDS.contains(&token)
}

fn fnv1a64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Lowercased alphanumeric runs, len ≥ 2, stopwords removed — same shape as
/// `query_terms` (which additionally keeps `_§-` inside tokens; the proxy
/// splits on them, which only costs sub-token matches, never adds any).
fn hash_tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !is_stopword(t))
        .map(|t| t.to_string())
        .collect()
}

/// Deterministic stand-in for model embeddings: hashed token counts,
/// L2-normalized. Same function embeds chunks and queries.
fn hash_embed(text: &str) -> Vec<f32> {
    let mut vec = vec![0f32; DIM];
    for token in hash_tokens(text) {
        let idx = (fnv1a64(&token) % DIM as u64) as usize;
        vec[idx] += 1.0;
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut vec {
            *x /= norm;
        }
    }
    vec
}

// --- Manifest (`tests/eval_data/eval.json`) ---

struct EvalDoc {
    id: String,
    file: String,
    title: String,
    summary: String,
    /// R40.5: optional section heading. When present, the leaf chunk's
    /// *embedding input* is `heading + "\n" + content` — modeling production
    /// `markdown::embed_input(chunks.path, content)` (R38.2) — while FTS and
    /// snippets still see the raw content. Pairs that put query terms in the
    /// heading but not the body measure the vector-leg gain of the prefix.
    heading: Option<String>,
}

struct EvalPair {
    id: String,
    query: String,
    expected: Vec<String>,
    floor_only_keyword: bool,
}

fn eval_path(rel: &str) -> String {
    format!("{EVAL_DIR}/{rel}")
}

fn str_field(value: &serde_json::Value, field: &str, ctx: &str) -> String {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{ctx}: missing string field '{field}'"))
        .to_string()
}

fn str_list(value: &serde_json::Value, field: &str, ctx: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("{ctx}: missing array field '{field}'"))
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("{ctx}: non-string entry in '{field}'"))
                .to_string()
        })
        .collect()
}

fn load_manifest() -> (Vec<EvalDoc>, Vec<EvalPair>) {
    let raw = std::fs::read_to_string(eval_path("eval.json")).expect("eval.json must be readable");
    let root: serde_json::Value = serde_json::from_str(&raw).expect("eval.json must parse");
    let docs = root
        .get("docs")
        .and_then(|v| v.as_array())
        .expect("eval.json must have a docs array")
        .iter()
        .map(|d| {
            let ctx = format!("doc {}", d.get("id").and_then(|v| v.as_str()).unwrap_or("?"));
            EvalDoc {
                id: str_field(d, "id", &ctx),
                file: str_field(d, "file", &ctx),
                title: str_field(d, "title", &ctx),
                summary: str_field(d, "summary", &ctx),
                heading: d.get("heading").and_then(|v| v.as_str()).map(|s| s.to_string()),
            }
        })
        .collect();
    let pairs = root
        .get("pairs")
        .and_then(|v| v.as_array())
        .expect("eval.json must have a pairs array")
        .iter()
        .map(|p| {
            let ctx = format!("pair {}", p.get("id").and_then(|v| v.as_str()).unwrap_or("?"));
            EvalPair {
                id: str_field(p, "id", &ctx),
                query: str_field(p, "query", &ctx),
                expected: str_list(p, "expected", &ctx),
                floor_only_keyword: p.get("floor_only_keyword").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        })
        .collect();
    (docs, pairs)
}

// --- Fixture DB (real migrations, hand-inserted corpus) ---

fn corpus_text(doc: &EvalDoc) -> String {
    std::fs::read_to_string(eval_path(&format!("corpus/{}", doc.file)))
        .unwrap_or_else(|_| panic!("doc {}: missing corpus file {}", doc.id, doc.file))
}

fn insert_chunk(
    conn: &Connection,
    item_id: i64,
    section_id: i64,
    kind: &str,
    source_ref: &str,
    content: &str,
    embed_text: &str,
) -> i64 {
    let embedding = embedder::pack_single(&hash_embed(embed_text));
    conn.execute(
        "INSERT INTO chunks (item_id, entity_id, source_ref, content, embedding, kind, section_id, level, path, is_pii, created_at) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 0, '', 0, datetime('now'))",
        params![item_id, source_ref, content, embedding, kind, section_id],
    )
    .expect("insert chunk");
    conn.last_insert_rowid()
}

fn insert_doc(conn: &Connection, source_id: i64, doc: &EvalDoc) {
    let content = corpus_text(doc);
    conn.execute(
        "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at) VALUES (?1, ?2, ?3, ?4, ?5, '', 0, datetime('now'))",
        params![source_id, doc.file, doc.title, content, format!("eval-{:016x}", fnv1a64(&content))],
    )
    .expect("insert item");
    let item_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sections (item_id, parent_id, level, title, path, chunk_range, summary, summary_hash, created_at) VALUES (?1, NULL, 0, ?2, ?2, '', ?3, '', datetime('now'))",
        params![item_id, doc.title, doc.summary],
    )
    .expect("insert section");
    let section_id = conn.last_insert_rowid();
    // R40.5: a declared heading prefixes the leaf *embedding input only*
    // (production `embed_input` split); FTS/snippet content stays raw.
    let leaf_embed = doc.heading.as_ref().map(|h| format!("{h}\n{content}")).unwrap_or_else(|| content.clone());
    insert_chunk(conn, item_id, section_id, "document", &doc.title, &content, &leaf_embed);
    insert_chunk(conn, item_id, section_id, "section_summary", &doc.title, &doc.summary, &doc.summary);
}

/// Fresh temp DB per test: real `init_db` migration chain (FTS triggers,
/// sections, vec fallback table), one `internal` folder source, the whole
/// manifest corpus inserted as items + sections + leaf/summary chunks.
fn build_eval_db(docs: &[EvalDoc]) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = db::init_db(&dir.path().join("eval.db")).expect("init_db");
    conn.execute(
        "INSERT INTO sources (name, connector, config, label, enabled, last_sync_cursor, error_count, created_at) VALUES ('eval-fixture', 'folder', '{}', 'internal', 1, '', 0, datetime('now'))",
        [],
    )
    .expect("insert source");
    let source_id = conn.last_insert_rowid();
    for doc in docs {
        insert_doc(&conn, source_id, doc);
    }
    (dir, conn)
}

fn item_externals(conn: &Connection) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, external_id FROM ingested_items").expect("items query");
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .expect("items rows");
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    map
}

// --- Legs + fusion ---

struct LegResults {
    vector: Vec<retrieval::Hit>,
    keyword: Vec<retrieval::Hit>,
    toc: Vec<retrieval::Hit>,
    fused: Vec<retrieval::Hit>,
}

fn run_query(conn: &Connection, query: &str) -> LegResults {
    let query_emb = hash_embed(query);
    let vector = retrieval::vector_search(conn, &query_emb, None, K, true);
    let keyword = retrieval::keyword_search(conn, query, None, K, true);
    let toc = retrieval::toc_search(conn, query, Some(&query_emb), None, K, true);
    // R39.12: the fused column calls the production fusion path itself
    // (`retrieval::orchestrate_fusion`, shared with `answer.rs::orchestrate`)
    // at production defaults (keyword_weight 1.0, halflife 365d,
    // max_per_source 3, exclude disputed) — the old `orchestrate_mirror`
    // restatement is deleted, so the eval can never drift from production.
    // The shared fn re-runs the same leg functions on the same inputs, so
    // it fuses the same outputs the per-leg columns report.
    let fused = retrieval::orchestrate_fusion(conn, query, Some(&query_emb), None, K, 1.0, 365.0, 3, true);
    LegResults { vector, keyword, toc, fused }
}

// --- Scoring + reporting ---

/// Best (1-based) rank per owning item external_id.
fn best_ranks(hits: &[retrieval::Hit], item_ext: &HashMap<i64, String>) -> HashMap<String, usize> {
    let mut ranks = HashMap::new();
    for (idx, hit) in hits.iter().enumerate() {
        if let Some(item_id) = hit.item_id {
            if let Some(external) = item_ext.get(&item_id) {
                ranks.entry(external.clone()).or_insert(idx + 1);
            }
        }
    }
    ranks
}

/// Fraction of `expected_files` present in the ranked list + their ranks.
/// Multi-doc pairs (ADR-vs-lint-config) get partial credit.
fn pair_recall(ranked: &HashMap<String, usize>, expected_files: &[String]) -> (f64, Vec<usize>) {
    let mut found: Vec<usize> = expected_files.iter().filter_map(|f| ranked.get(f).copied()).collect();
    found.sort_unstable();
    (found.len() as f64 / expected_files.len() as f64, found)
}

fn rank_cell(found: &[usize]) -> String {
    if found.is_empty() {
        return "·".to_string();
    }
    let ranks: Vec<String> = found.iter().map(|r| r.to_string()).collect();
    format!("✓{}", ranks.join(","))
}

struct TableRow {
    pair: String,
    expected: String,
    cells: [String; 4],
}

fn print_table(rows: &[TableRow], means: [f64; 4], doc_count: usize, pair_count: usize) {
    println!(
        "R39.10 golden retrieval eval — recall@{K} per leg ({doc_count} docs, {pair_count} pairs, hash-emb dim {DIM})"
    );
    println!(
        "{:<20} {:<26} {:>8} {:>8} {:>8} {:>8}",
        "pair", "expected", "vector", "keyword", "toc", "fused"
    );
    for row in rows {
        println!(
            "{:<20} {:<26} {:>8} {:>8} {:>8} {:>8}",
            row.pair, row.expected, row.cells[0], row.cells[1], row.cells[2], row.cells[3]
        );
    }
    println!(
        "{:<20} {:<26} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
        "MEAN (recall)", "", means[0], means[1], means[2], means[3]
    );
}

fn verbose_dump(
    pair_id: &str,
    query: &str,
    legs: &LegResults,
    item_ext: &HashMap<i64, String>,
) {
    println!("--- {pair_id}: {query}");
    let lists = [
        ("vector", &legs.vector),
        ("keyword", &legs.keyword),
        ("toc", &legs.toc),
        ("fused", &legs.fused),
    ];
    for (name, hits) in lists {
        println!("  {name}:");
        if hits.is_empty() {
            println!("    (no hits)");
        }
        for (idx, hit) in hits.iter().enumerate() {
            let external = hit
                .item_id
                .and_then(|id| item_ext.get(&id).cloned())
                .unwrap_or_else(|| "?".to_string());
            println!("    #{} {external} score={:.4}", idx + 1, hit.score);
        }
    }
}

fn verbose_enabled() -> bool {
    std::env::var("EVAL_VERBOSE").as_deref() == Ok("1")
}

struct PairOutcome {
    legs: LegResults,
    recalls: [(f64, Vec<usize>); 4],
}

fn score_pair(
    conn: &Connection,
    item_ext: &HashMap<i64, String>,
    files: &HashMap<&str, &str>,
    pair: &EvalPair,
) -> PairOutcome {
    let legs = run_query(conn, &pair.query);
    let expected_files: Vec<String> =
        pair.expected.iter().map(|id| files[id.as_str()].to_string()).collect();
    let recalls = [
        pair_recall(&best_ranks(&legs.vector, item_ext), &expected_files),
        pair_recall(&best_ranks(&legs.keyword, item_ext), &expected_files),
        pair_recall(&best_ranks(&legs.toc, item_ext), &expected_files),
        pair_recall(&best_ranks(&legs.fused, item_ext), &expected_files),
    ];
    PairOutcome { legs, recalls }
}

// --- Tests ---

/// Manifest self-check (no DB): every expected id resolves, every doc is
/// covered by at least one pair, nothing is empty, corpus files exist.
#[test]
fn eval_manifest_is_consistent() {
    let (docs, pairs) = load_manifest();
    assert!(!docs.is_empty(), "eval.json must list docs");
    assert!(!pairs.is_empty(), "eval.json must list pairs");
    let ids: HashSet<&str> = docs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), docs.len(), "duplicate doc ids in eval.json");
    let pair_ids: HashSet<&str> = pairs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(pair_ids.len(), pairs.len(), "duplicate pair ids in eval.json");
    let mut covered: HashSet<&str> = HashSet::new();
    for pair in &pairs {
        assert!(!pair.query.trim().is_empty(), "pair {} has an empty query", pair.id);
        assert!(!pair.expected.is_empty(), "pair {} has no expected docs", pair.id);
        for expected in &pair.expected {
            assert!(ids.contains(expected.as_str()), "pair {} expects unknown doc '{expected}'", pair.id);
            covered.insert(expected.as_str());
        }
        if pair.floor_only_keyword {
            assert_eq!(
                pair.expected.len(),
                1,
                "pair {}: the floor-only signature is per-doc",
                pair.id
            );
        }
    }
    for doc in &docs {
        assert!(!doc.summary.trim().is_empty(), "doc {} has an empty summary", doc.id);
        let content = corpus_text(doc);
        assert!(!content.trim().is_empty(), "doc {} has an empty corpus file", doc.id);
        assert!(covered.contains(doc.id.as_str()), "doc {} is covered by no pair", doc.id);
    }
    assert!(pairs.iter().any(|p| p.floor_only_keyword), "no floor-only R39.2 pair in eval.json");
}

/// The whole fixture must classify as live env: a stray test-env marker in
/// a fixture path or file head would let `prefer_live_envs` reorder hits and
/// make the eval order-dependent. (This also keeps `envs_for_chunks` used in
/// this target so `-D warnings` stays clean.)
#[test]
fn eval_fixture_is_live_env() {
    let (docs, _) = load_manifest();
    let (_dir, conn) = build_eval_db(&docs);
    let chunk_ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM chunks").expect("chunks query");
        stmt.query_map([], |r| r.get::<_, i64>(0))
            .expect("chunks rows")
            .flatten()
            .collect()
    };
    assert!(!chunk_ids.is_empty(), "fixture inserted no chunks");
    let envs = retrieval::envs_for_chunks(&conn, &chunk_ids);
    assert_eq!(envs.len(), chunk_ids.len(), "every chunk must classify");
    for (chunk_id, env) in &envs {
        assert_eq!(
            env, "live",
            "chunk {chunk_id} classified as test env — check eval_data paths/heads for env markers"
        );
    }
}

/// Full run: print the per-leg recall table (visible with `-- --nocapture`)
/// and assert the golden bar — fused recall 1.0 on every pair. Per-leg
/// columns are reported, not asserted: fusion tuning needs room to move
/// them while the fused bar stays green.
#[test]
fn eval_golden_recall() {
    let (docs, pairs) = load_manifest();
    let (_dir, conn) = build_eval_db(&docs);
    let item_ext = item_externals(&conn);
    let files: HashMap<&str, &str> = docs.iter().map(|d| (d.id.as_str(), d.file.as_str())).collect();
    let verbose = verbose_enabled();
    let mut rows = Vec::new();
    let mut means = [0.0; 4];
    let mut fused_recalls = Vec::new();
    for pair in &pairs {
        let outcome = score_pair(&conn, &item_ext, &files, pair);
        if verbose {
            verbose_dump(&pair.id, &pair.query, &outcome.legs, &item_ext);
        }
        let mut cells: [String; 4] = Default::default();
        for (idx, (_, found)) in outcome.recalls.iter().enumerate() {
            cells[idx] = rank_cell(found);
            means[idx] += found.len() as f64 / pair.expected.len() as f64;
        }
        fused_recalls.push(outcome.recalls[3].0);
        rows.push(TableRow { pair: pair.id.clone(), expected: pair.expected.join("+"), cells });
    }
    let pair_count = pairs.len() as f64;
    for mean in &mut means {
        *mean /= pair_count;
    }
    print_table(&rows, means, docs.len(), pairs.len());
    for (pair, fused) in pairs.iter().zip(fused_recalls) {
        assert_eq!(
            fused, 1.0,
            "pair {}: fused recall {fused} — expected {:?} missing from top-{K}",
            pair.id, pair.expected
        );
    }
}

/// R39.2 regression pin: a bare opaque code must be invisible to the vector
/// leg (no lexical signal, cosine under the 0.2 floor — the hash proxy for a
/// real embedder's no-signal direction) and to TOC (summaries carry no code
/// tokens), while the keyword leg fires and the fused result still hits via
/// the keyword floor.
#[test]
fn r39_2_exact_code_is_keyword_floor_only() {
    let (docs, pairs) = load_manifest();
    let flagged: Vec<&EvalPair> = pairs.iter().filter(|p| p.floor_only_keyword).collect();
    assert!(!flagged.is_empty(), "no floor_only_keyword pairs in eval.json");
    let (_dir, conn) = build_eval_db(&docs);
    let item_ext = item_externals(&conn);
    let files: HashMap<&str, &str> = docs.iter().map(|d| (d.id.as_str(), d.file.as_str())).collect();
    for pair in flagged {
        let outcome = score_pair(&conn, &item_ext, &files, pair);
        let [vec_r, kw_r, toc_r, fused_r] = &outcome.recalls;
        assert_eq!(vec_r.0, 0.0, "pair {}: vector leg must MISS the opaque code", pair.id);
        assert_eq!(kw_r.0, 1.0, "pair {}: keyword leg must HIT the opaque code", pair.id);
        assert_eq!(toc_r.0, 0.0, "pair {}: TOC leg must MISS (no code in summaries)", pair.id);
        assert_eq!(fused_r.0, 1.0, "pair {}: fused must HIT via the keyword floor", pair.id);
    }
}
