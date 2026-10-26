//! Dynamic tag taxonomy — R14.4 reuse-or-create.

use rusqlite::Connection;
use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

/// R30.2 perf counters (read by pipeline debug logs).
pub(crate) static ENSURE_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static ENSURE_EXACT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static ENSURE_SCANS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static ENSURE_MICROS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn stopwords() -> &'static HashSet<&'static str> {
    static SW: OnceLock<HashSet<&str>> = OnceLock::new();
    SW.get_or_init(|| {
        [
            "the", "and", "for", "with", "this", "that", "from", "have", "will", "would",
            "about", "which", "when", "where", "what", "their", "there", "been", "also",
            "into", "more", "than", "some", "such", "only", "other", "could", "should",
            "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for",
        ]
        .into()
    })
}

pub fn normalize_tag(name: &str) -> String {
    name.trim().to_lowercase()
}

/// R30.7: compiled once — `propose_tags` runs per section (2801 compiles
/// for one 2MB file) and the compile dwarfed the tokenize.
fn tag_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}]{4,}").unwrap())
}

pub fn propose_tags(section_text: &str, title: &str) -> Vec<String> {
    let combined = format!("{} {}", title, section_text);
    let re = tag_token_re();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in re.find_iter(&combined.to_lowercase()) {
        let tok = m.as_str().to_string();
        if stopwords().contains(tok.as_str()) {
            continue;
        }
        *counts.entry(tok).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted.into_iter().take(4).map(|(k, _)| normalize_tag(&k)).collect()
}

fn tag_vec(name: &str) -> Vec<f32> {
    crate::embedder::deterministic_embed(name)
}

/// R25.3: tag vectors are computed locally (`deterministic_embed`) — rebuild
/// any that came from an import (or an older build) without one, so cosine
/// reuse keeps working.
pub fn ensure_embeddings(conn: &Connection) -> Result<usize, String> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name FROM tags WHERE embedding IS NULL")
            .map_err(|e| e.to_string())?;
        let mapped: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        mapped
    };
    let mut n = 0usize;
    for (id, name) in rows {
        let blob = crate::embedder::pack_single(&tag_vec(&name));
        conn.execute("UPDATE tags SET embedding=?1 WHERE id=?2", rusqlite::params![blob, id])
            .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    crate::embedder::cosine_similarity(a, b)
}

fn threshold() -> f32 {
    if let Ok(v) = std::env::var("ANCHOR_TAG_REUSE_THRESHOLD") {
        if let Ok(f) = v.parse::<f32>() {
            return f.clamp(0.0, 1.0);
        }
    }
    // DB app_settings fallback (SettingsService persistence via PUT /settings)
    if let Ok(data_dir) = std::env::var("ANCHOR_DATA_DIR") {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::open_db(&db_path) {
            if let Ok(val) = conn.query_row(
                "SELECT value FROM app_settings WHERE key='tag_reuse_threshold'",
                [],
                |r| r.get::<_, String>(0),
            ) {
                if let Ok(f) = val.parse::<f32>() {
                    return f.clamp(0.0, 1.0);
                }
            }
        }
    } else {
        // Try default data dir locations (covers tests with temp dir via ANCHOR_DATA_DIR, else fallback to ./data)
        for cand in ["data", &format!("{}/.anchorcore", std::env::var("HOME").unwrap_or_default())] {
            let db_path = crate::db::resolve_db_path(cand);
            if !db_path.exists() {
                continue;
            }
            if let Ok(conn) = crate::db::open_db(&db_path) {
                if let Ok(val) = conn.query_row(
                    "SELECT value FROM app_settings WHERE key='tag_reuse_threshold'",
                    [],
                    |r| r.get::<_, String>(0),
                ) {
                    if let Ok(f) = val.parse::<f32>() {
                        return f.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
    0.82
}

/// R30.5: tag vectors decoded once per tag pass. A miss used to re-read and
/// unpack every tag embedding (plus a `threshold()` lookup that can open a
/// DB connection); the pass now loads them on its first miss and keeps the
/// list current as it creates tags. Scope is one `create_tags` transaction
/// (the write lock is held, so nothing else can change `tags` meanwhile) —
/// a cache across documents would go stale against a concurrent sync.
#[derive(Default)]
pub struct TagVecCache {
    loaded: Option<LoadedTags>,
}

struct LoadedTags {
    threshold: f32,
    vecs: Vec<(i64, Vec<f32>)>,
    /// Per name: best match so far and how many `vecs` it has seen. A name
    /// that keeps missing the exact lookup (reused by cosine, so never
    /// inserted under its own name) only compares against tags added since —
    /// the same answer as a full scan, without redoing it.
    memo: std::collections::HashMap<String, (Option<(i64, f32)>, usize)>,
    /// Non-zero dimension → positions in `vecs`. Tag vectors are sparse
    /// bag-of-words hashes and the threshold is ≥ 0, so a tag sharing no
    /// non-zero dimension (cosine 0) can never be reused — only candidates
    /// from here are scored, with the same `cosine`, in `vecs` order.
    postings: std::collections::HashMap<usize, Vec<usize>>,
}

impl LoadedTags {
    fn new(threshold: f32, vecs: Vec<(i64, Vec<f32>)>) -> Self {
        let mut tags = LoadedTags { threshold, vecs: Vec::new(), memo: Default::default(), postings: Default::default() };
        for (id, vec) in vecs {
            tags.push(id, vec);
        }
        tags
    }

    fn push(&mut self, id: i64, vec: Vec<f32>) {
        let pos = self.vecs.len();
        for dim in nonzero_dims(&vec) {
            self.postings.entry(dim).or_default().push(pos);
        }
        self.vecs.push((id, vec));
    }

    /// Positions (ascending, ≥ `from`) of tags sharing a dimension with `vec`.
    fn candidates(&self, vec: &[f32], from: usize) -> Vec<usize> {
        let mut out: Vec<usize> = nonzero_dims(vec)
            .filter_map(|d| self.postings.get(&d))
            .flatten()
            .copied()
            .filter(|&p| p >= from)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn nonzero_dims(vec: &[f32]) -> impl Iterator<Item = usize> + '_ {
    vec.iter().enumerate().filter(|(_, v)| **v != 0.0).map(|(i, _)| i)
}

impl TagVecCache {
    fn loaded(&mut self, conn: &Connection, dim: usize) -> Result<&mut LoadedTags, String> {
        if self.loaded.is_none() {
            ENSURE_SCANS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.loaded = Some(LoadedTags::new(threshold(), load_tag_vecs(conn, dim)?));
        }
        self.loaded.as_mut().ok_or_else(|| "tag cache".to_string())
    }
}

fn load_tag_vecs(conn: &Connection, dim: usize) -> Result<Vec<(i64, Vec<f32>)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, embedding FROM tags WHERE embedding IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows
        .flatten()
        .filter_map(|(id, blob)| crate::embedder::unpack_f32(&blob, dim).map(|v| (id, v)))
        .collect())
}

/// Most similar cached tag above the reuse threshold (first wins on ties,
/// like the id-ordered full scan).
fn best_match(tags: &mut LoadedTags, norm: &str, new_vec: &[f32]) -> Option<i64> {
    let (mut best, seen) = tags.memo.get(norm).copied().unwrap_or((None, 0));
    for pos in tags.candidates(new_vec, seen) {
        let (id, vec) = &tags.vecs[pos];
        let sim = cosine(new_vec, vec);
        if sim > tags.threshold && best.map(|(_, s)| sim > s).unwrap_or(true) {
            best = Some((*id, sim));
        }
    }
    tags.memo.insert(norm.to_string(), (best, tags.vecs.len()));
    best.map(|(id, _)| id)
}

fn bump_count(conn: &Connection, id: i64) -> Result<i64, String> {
    conn.execute("UPDATE tags SET count = count + 1 WHERE id=?1", [id]).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Ensure a tag exists — reuse if cosine > threshold else create.
/// Returns tag id. One-off callers; a tag pass uses `ensure_tag_cached`.
pub fn ensure_tag(conn: &Connection, name: &str) -> Result<i64, String> {
    ensure_tag_cached(conn, name, &mut TagVecCache::default())
}

/// `ensure_tag` against a per-pass vector cache (R30.5).
pub fn ensure_tag_cached(conn: &Connection, name: &str, cache: &mut TagVecCache) -> Result<i64, String> {
    use std::sync::atomic::Ordering;
    let t0 = std::time::Instant::now();
    ENSURE_CALLS.fetch_add(1, Ordering::Relaxed);
    let norm = normalize_tag(name);
    if norm.is_empty() {
        return Err("empty tag".to_string());
    }
    // check exact match first (fast)
    if let Ok(id) = conn.query_row("SELECT id FROM tags WHERE name=?1", [&norm], |r| r.get::<_, i64>(0)) {
        ENSURE_EXACT.fetch_add(1, Ordering::Relaxed);
        ENSURE_MICROS.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        return bump_count(conn, id);
    }
    let new_vec = tag_vec(&norm);
    let tags = cache.loaded(conn, new_vec.len())?;
    if let Some(best_id) = best_match(tags, &norm, &new_vec) {
        ENSURE_MICROS.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        return bump_count(conn, best_id);
    }
    ENSURE_MICROS.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
    // create new
    let emb = crate::embedder::pack_single(&new_vec);
    conn.execute(
        "INSERT INTO tags (name, description, embedding, count, uid) VALUES (?1,'',?2,1,?3)",
        rusqlite::params![norm, emb, crate::db::new_uid()],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    tags.push(id, new_vec);
    Ok(id)
}

// --- API handlers ---

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::health::AppState;

pub async fn list_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        let mut stmt = match conn.prepare("SELECT id, name, description, count, created_at FROM tags ORDER BY count DESC, name ASC") {
            Ok(s) => s,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "description": r.get::<_, String>(2)?,
                    "count": r.get::<_, i64>(3)?,
                    "created_at": r.get::<_, Option<String>>(4)?,
                }))
            })
            .unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Json(Value::Array(out))
    })
    .await
    .unwrap();
    result
}

pub async fn chunks_handler(State(state): State<AppState>, axum::extract::Path(tag_id): axum::extract::Path<i64>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        let mut stmt = match conn.prepare(
            "SELECT c.id, c.item_id, c.section_id, c.kind, c.source_ref, c.content, c.path, c.created_at FROM chunks c JOIN chunk_tags ct ON ct.chunk_id=c.id WHERE ct.tag_id=?1 ORDER BY c.id",
        ) {
            Ok(s) => s,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let rows = stmt
            .query_map([tag_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "item_id": r.get::<_, Option<i64>>(1)?,
                    "section_id": r.get::<_, Option<i64>>(2)?,
                    "kind": r.get::<_, String>(3)?,
                    "source_ref": r.get::<_, String>(4)?,
                    "content": r.get::<_, String>(5)?,
                    "path": r.get::<_, String>(6)?,
                    "created_at": r.get::<_, Option<String>>(7)?,
                }))
            })
            .unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Json(Value::Array(out))
    })
    .await
    .unwrap();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn propose_tags_basic() {
        let tags = propose_tags("Billing migration content about billing system", "Billing");
        assert!(tags.contains(&"billing".to_string()));
        assert!(tags.len() <= 4);
    }

    #[test]
    fn ensure_tag_reuse_and_new() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        let id1 = ensure_tag(&conn, "billing").unwrap();
        let id2 = ensure_tag(&conn, "billing").unwrap();
        assert_eq!(id1, id2, "same billing should reuse");
        let cnt: i64 = conn.query_row("SELECT count FROM tags WHERE id=?1", [id1], |r| r.get(0)).unwrap();
        assert_eq!(cnt, 2);
        let id3 = ensure_tag(&conn, "acme").unwrap();
        assert_ne!(id1, id3, "acme should be new tag");
        let all: i64 = conn.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0)).unwrap();
        assert_eq!(all, 2);
    }

    #[test]
    fn cached_pass_decides_like_uncached_and_loads_once() {
        use std::sync::atomic::Ordering;
        // car/vehicle/auto and database/db/storage share a vector
        // (`deterministic_embed` synonyms): repeated cosine reuse, never an
        // exact hit for the reused names.
        let names = ["car", "billing", "vehicle", "auto", "database", "storage", "car", "auto", "db", "store", "vehicle", "auto", "billing"];
        let dir = tempdir().unwrap();
        let plain = crate::db::init_db(&dir.path().join("plain.db")).unwrap();
        let cached = crate::db::init_db(&dir.path().join("cached.db")).unwrap();
        let mut cache = TagVecCache::default();
        let scans0 = ENSURE_SCANS.load(Ordering::Relaxed);
        let a: Vec<i64> = names.iter().map(|n| ensure_tag(&plain, n).unwrap()).collect();
        let scans_plain = ENSURE_SCANS.load(Ordering::Relaxed) - scans0;
        let scans1 = ENSURE_SCANS.load(Ordering::Relaxed);
        let b: Vec<i64> = names.iter().map(|n| ensure_tag_cached(&cached, n, &mut cache).unwrap()).collect();
        // Other tests bump the global counter concurrently; the cached pass
        // alone loads at most once, so it never exceeds the uncached count.
        assert!(ENSURE_SCANS.load(Ordering::Relaxed) - scans1 <= scans_plain);
        assert_eq!(a, b, "same reuse/create decisions with and without the cache");
        let rows = |c: &Connection| -> Vec<(String, i64)> {
            c.prepare("SELECT name, count FROM tags ORDER BY id").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(rows(&plain), rows(&cached));
        assert_eq!(rows(&cached).len(), 3, "synonyms reuse: car, billing, database");
        assert_eq!(cache.loaded.as_ref().map(|l| l.vecs.len()), Some(rows(&cached).len()), "created tags join the cache");
    }

    /// R30.5: the indexed cache picks exactly what the uncached full scan
    /// picks, across enough names to force hash-bucket reuse.
    #[test]
    fn cached_pass_matches_uncached_on_many_names() {
        let syl = ["ka", "lo", "mi", "ren", "tas", "vo", "qui", "bel", "dor", "fen"];
        let mut x: u64 = 7;
        let names: Vec<String> = (0..800)
            .map(|_| {
                (0..3)
                    .map(|_| {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        syl[(x >> 33) as usize % syl.len()]
                    })
                    .collect()
            })
            .collect();
        let dir = tempdir().unwrap();
        let plain = crate::db::init_db(&dir.path().join("plain.db")).unwrap();
        let cached = crate::db::init_db(&dir.path().join("cached.db")).unwrap();
        let mut cache = TagVecCache::default();
        let a: Vec<i64> = names.iter().map(|n| ensure_tag(&plain, n).unwrap()).collect();
        let b: Vec<i64> = names.iter().map(|n| ensure_tag_cached(&cached, n, &mut cache).unwrap()).collect();
        assert_eq!(a, b);
        let distinct: std::collections::HashSet<&String> = names.iter().collect();
        let tags: i64 = cached.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0)).unwrap();
        assert!((tags as usize) < distinct.len(), "the name set must exercise cosine reuse");
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_tag("  Billing "), "billing");
        assert_eq!(normalize_tag("ACME"), "acme");
    }
}
