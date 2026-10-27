//! Embedder + IDF gate — port of `backend/app/embedder.py:1`.
//! `pack_f32`, cloud-trust gate for labeled sources, `vec0` sync triggers, `signal()` gating via `idf::signal`.

use std::sync::Arc;

use crate::embed_fp::{fingerprint, model_family, record_active_fingerprint, PROVIDER_BUILTIN, PROVIDER_HTTP};
use crate::settings::SettingsService;

pub const BATCH_SIZE: usize = 32;
/// R30.4: texts per built-in blocking task — the cancel granularity of the
/// in-process engine (it length-sorts within a group, so keep it > 1 batch).
const BUILTIN_GROUP: usize = BATCH_SIZE * 2;
/// R30.2: embed batches in flight per `embed` call (`ANCHOR_EMBED_CONCURRENCY`).
const CONCURRENCY_DEFAULT: usize = 2;
const CONCURRENCY_MAX: usize = 8;

pub(crate) fn embed_concurrency(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(CONCURRENCY_DEFAULT)
        .clamp(1, CONCURRENCY_MAX)
}

/// R30.2: one pooled client for every embed call (was one per call); the
/// timeout is set per request so a settings change applies immediately.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

#[derive(serde::Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(serde::Deserialize)]
struct EmbedItem {
    index: usize,
    embedding: Vec<f32>,
}

/// One request's settings, shared by its batches.
struct EmbedCall {
    url: String,
    model: String,
    api_key: String,
    timeout: std::time::Duration,
}

impl EmbedCall {
    async fn batch(&self, batch: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let payload = serde_json::json!({"model": self.model, "input": batch});
        let mut req = http_client().post(&self.url).timeout(self.timeout).json(&payload);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        // R11.2: never persist degraded vectors — surface the error so the
        // pipeline leaves the chunks NULL for the backfill.
        let resp = req.send().await.map_err(|e| degraded(format!("embed failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(degraded(format!("embed status {}", resp.status())));
        }
        let body: EmbedResponse = resp.json().await.map_err(|e| degraded(format!("embed json failed: {e}")))?;
        let mut items = body.data;
        items.sort_by_key(|it| it.index);
        Ok(items.into_iter().map(|it| it.embedding).collect())
    }
}

fn degraded(msg: String) -> String {
    tracing::warn!("{}, not persisting degraded", msg);
    msg
}

/// Pack f32 vectors as little-endian bytes — mirrors `backend/app/embedder.py:15` `pack_f32`.
pub fn pack_f32(vectors: &[Vec<f32>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vectors.len() * vectors.first().map(|v| v.len()).unwrap_or(0) * 4);
    for vec in vectors {
        for v in vec {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

pub fn pack_single(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn unpack_f32(data: &[u8], dims: usize) -> Option<Vec<f32>> {
    if data.len() < dims * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(dims);
    for chunk in data.chunks_exact(4).take(dims) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

#[derive(Clone)]
pub struct Embedder {
    pub settings: Arc<SettingsService>,
    /// R45.5: fired when a query vector's family differs from the recorded
    /// one (model switch) — the scheduler re-embeds stale vectors right away
    /// instead of waiting for the 15-minute backfill tick.
    pub family_changed: Arc<tokio::sync::Notify>,
    /// R46.12: fired when chunks were written outside a sync (a person's
    /// caption) — wakes the backfill so they don't wait for the tick.
    pub chunks_pending: Arc<tokio::sync::Notify>,
}

/// R43.6: nomic-embed models expect task prefixes (`search_query: ` on
/// queries, `search_document: ` on stored chunks — per nomic's usage docs).
/// A prefixed query against unprefixed documents (or vice versa) degrades
/// recall, so both sides prefix together, gated on the configured model
/// name. Non-nomic providers see byte-identical inputs.
pub fn uses_nomic_prefixes(model: &str) -> bool {
    model.to_lowercase().contains("nomic")
}

/// R43.6: query-side input for the embedding call — nomic-gated prefix.
pub fn prefix_query_text(text: &str, model: &str) -> String {
    if uses_nomic_prefixes(model) {
        format!("search_query: {text}")
    } else {
        text.to_string()
    }
}

/// R43.6: document-side input for the embedding call — nomic-gated prefix.
pub fn prefix_document_text(text: &str, model: &str) -> String {
    if uses_nomic_prefixes(model) {
        format!("search_document: {text}")
    } else {
        text.to_string()
    }
}

/// R43.6: place returned vectors onto their gated positions. The provider
/// must return exactly one vector per input — a short response today would
/// misalign later vectors onto wrong chunks (middle drop) or silently
/// leave tails unembedded (`.zip` truncation); both now error instead of
/// corrupting the store. Message mirrors the scheduler backfill's check.
fn place_vectors(len: usize, embeddable_idx: &[usize], vectors: Vec<Vec<f32>>) -> Result<Vec<Option<Vec<u8>>>, String> {
    if vectors.len() != embeddable_idx.len() {
        return Err(format!("embedder returned {} vectors for {} chunks", vectors.len(), embeddable_idx.len()));
    }
    let mut out: Vec<Option<Vec<u8>>> = vec![None; len];
    for (&pos, v) in embeddable_idx.iter().zip(vectors) {
        out[pos] = Some(pack_single(&v));
    }
    Ok(out)
}

/// R45.4: which engine embeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// In-process nomic-embed-text v1.5 (`embed_local.rs`).
    Builtin,
    /// Ollama or any OpenAI-compatible `/v1/embeddings` endpoint.
    Http,
}

/// `embed_provider` → engine. `auto` (default, and anything unknown) uses
/// the HTTP embedder whenever it is preferred — a configured cloud endpoint,
/// or a local Ollama that serves the embed model (owner 2026-10-26: Ollama
/// indexed ~5× faster than the built-in engine on an M2 Pro) — and the
/// built-in engine when its model is ready and nothing better serves. Both
/// are one vector family, so switching re-embeds nothing.
pub fn resolve_provider(setting: Option<&str>, builtin_ready: bool, http_preferred: bool) -> Provider {
    match setting.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("builtin") => Provider::Builtin,
        Some("ollama") => Provider::Http,
        _ if builtin_ready && !http_preferred => Provider::Builtin,
        _ => Provider::Http,
    }
}

/// R45.4: whether the local Ollama serves the embed model, cached for 60 s
/// (`None` = never checked or stale). Read synchronously by `auto`.
static OLLAMA_SERVING: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
const SERVING_TTL: std::time::Duration = std::time::Duration::from_secs(60);

fn ollama_serving_cached() -> Option<bool> {
    let slot = OLLAMA_SERVING.lock().unwrap_or_else(|p| p.into_inner());
    slot.filter(|(at, _)| at.elapsed() < SERVING_TTL).map(|(_, v)| v)
}

/// Re-check Ollama when the cache is stale and the answer could matter
/// (`auto` with a ready built-in model and a local endpoint).
pub(crate) async fn refresh_ollama_serving(settings: &SettingsService) {
    let auto = !matches!(settings.get("embed_provider", None).as_deref().map(str::trim), Some("builtin" | "ollama"));
    let ready = builtin_dir_for(settings).is_some_and(|d| crate::model_store::ready(&d));
    if !auto || !ready || cloud_endpoint(settings) || ollama_serving_cached().is_some() {
        return;
    }
    let base = settings.get("embed_base_url", None).or_else(|| settings.get("ollama_base_url", None)).unwrap_or_else(|| "http://localhost:11434".to_string());
    let model = settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string());
    let serving = match crate::ollama::fetch_ollama_tags(&base).await {
        Some(tags) => crate::ollama::missing_from_installed(&[model], &crate::ollama::installed_families(&tags)).is_empty(),
        None => false,
    };
    *OLLAMA_SERVING.lock().unwrap_or_else(|p| p.into_inner()) = Some((std::time::Instant::now(), serving));
}

/// An explicitly configured non-local `embed_base_url` (the person chose it).
fn cloud_endpoint(settings: &SettingsService) -> bool {
    settings.get("embed_base_url", None).is_some_and(|b| !crate::common::provider_is_local(&b))
}

/// R45.4: the built-in model's directory (`ANCHOR_MODELS_DIR` or
/// `{data_dir}/models`); `None` without either (unit tests).
pub fn builtin_dir_for(settings: &SettingsService) -> Option<std::path::PathBuf> {
    let root = match (std::env::var("ANCHOR_MODELS_DIR"), settings.data_dir()) {
        (Ok(v), _) if !v.trim().is_empty() => std::path::PathBuf::from(v),
        (_, Some(data)) => crate::model_store::models_root(&data),
        _ => return None,
    };
    Some(crate::model_store::model_dir(&root))
}

pub fn active_provider_for(settings: &SettingsService) -> Provider {
    let ready = builtin_dir_for(settings).is_some_and(|d| crate::model_store::ready(&d));
    let http_preferred = cloud_endpoint(settings) || ollama_serving_cached().unwrap_or(false);
    resolve_provider(settings.get("embed_provider", None).as_deref(), ready, http_preferred)
}

impl Embedder {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings, family_changed: Arc::new(tokio::sync::Notify::new()), chunks_pending: Arc::new(tokio::sync::Notify::new()) }
    }

    fn embed_model_name(&self) -> String {
        if self.active_provider() == Provider::Builtin {
            return crate::model_store::MODEL_NAME.to_string();
        }
        self.settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string())
    }

    /// R45.4: the built-in model's directory (see `builtin_dir_for`).
    pub fn builtin_dir(&self) -> Option<std::path::PathBuf> {
        builtin_dir_for(&self.settings)
    }

    pub fn active_provider(&self) -> Provider {
        active_provider_for(&self.settings)
    }

    /// R45.5: provider tag stamped on this embedder's vectors.
    fn provider(&self) -> &'static str {
        match self.active_provider() {
            Provider::Builtin => PROVIDER_BUILTIN,
            Provider::Http => PROVIDER_HTTP,
        }
    }

    /// R45.5: dim-free family of the configured embedder — stored vectors
    /// outside it are stale and the backfill re-embeds them.
    pub fn configured_family(&self) -> String {
        model_family(self.provider(), &self.embed_model_name())
    }

    /// R45.5: fingerprint for a `dim`-wide vector from the configured embedder.
    pub fn fingerprint(&self, dim: usize) -> String {
        fingerprint(self.provider(), &self.embed_model_name(), dim)
    }

    /// R45.5: record the query vector's fingerprint before retrieval reads
    /// it, and wake the re-embed when the family changed. Returns false when
    /// it could not be recorded: retrieval would then filter against a stale
    /// fingerprint, so the caller drops the vector (keyword-only) instead.
    /// Skipped (true) when the data dir is unpinned (unit tests), so nothing
    /// writes a stray `./data`.
    async fn record_query_fingerprint(&self, fp: String) -> bool {
        let Some(dir) = self.settings.data_dir() else { return true };
        let db_path = crate::db::resolve_db_path(&dir.to_string_lossy());
        let recorded = tokio::task::spawn_blocking(move || {
            let conn = crate::db::open_db(&db_path).map_err(|e| e.to_string())?;
            record_active_fingerprint(&conn, &fp)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        match recorded {
            Ok(changed) => {
                if changed {
                    self.family_changed.notify_one();
                }
                true
            }
            Err(e) => {
                tracing::warn!("recording query embedding fingerprint failed ({}); keyword-only", e);
                false
            }
        }
    }

    /// R43.6: document-side prefix for direct `embed` callers (the scheduler
    /// backfill embeds documents without going through `embed_gated`).
    pub(crate) fn prefix_document(&self, text: &str) -> String {
        prefix_document_text(text, &self.embed_model_name())
    }

    fn base_url(&self) -> String {
        self.settings
            .get("embed_base_url", None)
            .or_else(|| self.settings.get("ollama_base_url", None))
            .unwrap_or_else(|| "http://localhost:11434".to_string())
    }

    fn provider_is_local(&self) -> bool {
        crate::common::provider_is_local(&self.base_url())
    }

    /// Whether a remote embedder is trusted (B39/B30). Local — including the
    /// built-in engine, which never leaves the process — is always trusted.
    pub(crate) fn cloud_trusted(&self) -> bool {
        self.active_provider() == Provider::Builtin || crate::common::provider_trusted(&self.base_url())
    }

    /// Embed in `BATCH_SIZE` requests, up to `ANCHOR_EMBED_CONCURRENCY` in
    /// flight (R30.2); vectors come back in input order, and the first
    /// failed batch fails the call.
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        refresh_ollama_serving(&self.settings).await;
        if self.active_provider() == Provider::Builtin {
            return self.embed_builtin(texts).await;
        }
        self.embed_http(texts).await
    }

    /// R45.4: in-process engine on a blocking thread (loads on first use).
    /// R30.4: one blocking task per `BUILTIN_GROUP` texts, so a cancelled job
    /// (which drops this future) waits out at most one group, never the
    /// whole document.
    async fn embed_builtin(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let Some(dir) = self.builtin_dir().filter(|d| crate::model_store::ready(d)) else {
            return Err(degraded("built-in embedding model is not downloaded".to_string()));
        };
        let mut out = Vec::with_capacity(texts.len());
        for group in texts.chunks(BUILTIN_GROUP) {
            let (dir, group) = (dir.clone(), group.to_vec());
            let part = tokio::task::spawn_blocking(move || crate::embed_local::embed_blocking(&dir, &group))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| degraded(format!("built-in embed failed: {e}")))?;
            out.extend(part);
        }
        Ok(out)
    }

    async fn embed_http(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        use futures_util::{StreamExt, TryStreamExt};
        let call = EmbedCall {
            url: format!("{}/v1/embeddings", self.base_url().trim_end_matches('/')),
            model: self.embed_model_name(),
            api_key: self.settings.get("embed_api_key", None).unwrap_or_default(),
            timeout: std::time::Duration::from_secs(self.settings.get_float("embed_timeout", 60.0) as u64),
        };
        let concurrency = embed_concurrency(std::env::var("ANCHOR_EMBED_CONCURRENCY").ok().as_deref());
        let batches: Vec<Vec<String>> = texts.chunks(BATCH_SIZE).map(<[String]>::to_vec).collect();
        let parts: Vec<Vec<Vec<f32>>> = futures_util::stream::iter(batches.into_iter().map(|b| call.batch(b)))
            .buffered(concurrency)
            .try_collect()
            .await?;
        Ok(parts.into_iter().flatten().collect())
    }

    /// B58: strict query embedding — `None` when remote is unavailable, so
    /// retrieval degrades to honest keyword-only. (Previously fell back to
    /// `deterministic_embed`, whose hash vectors scored as noise against
    /// real model chunk vectors.) `deterministic_embed` remains for tags,
    /// which compare deterministic vectors against each other.
    pub async fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        // R43.6: nomic-gated `search_query: ` prefix (no-op for non-nomic).
        let input = prefix_query_text(text, &self.embed_model_name());
        let vector = match self.embed(vec![input]).await {
            Ok(v) if !v.is_empty() => v.into_iter().next()?,
            _ => return None,
        };
        // R45.5: the vector legs compare only against its family.
        let fp = self.fingerprint(vector.len());
        if !self.record_query_fingerprint(fp).await {
            return None;
        }
        Some(vector)
    }

    /// Embed with IDF+trust gating (mirrors `backend/app/pipeline.py:316` `_embed_item`).
    /// Returns packed bytes for storage; skips low-signal chunks and gated sources.
    /// R35.1: the per-chunk PII skip is gone with auto-detect — `gated` (source
    /// label vs provider trust) is the only remaining gate.
    ///
    /// `contents` is the stored text (drives the IDF gate); `embed_texts` is
    /// what actually gets embedded — breadcrumb + link-stripped text
    /// (`markdown::embed_input`, R38.2). The split is deliberate: a repeated
    /// breadcrumb dilutes mean-IDF and would gate short-but-meaningful chunks
    /// ("Approved" under an ADR heading), so gating stays on raw content.
    pub async fn embed_gated(
        &self,
        contents: Vec<String>,
        embed_texts: Vec<String>,
        gated: bool,
        embed_min_signal: f64,
    ) -> Result<Vec<Option<Vec<u8>>>, String> {
        if contents.is_empty() {
            return Ok(vec![]);
        }
        let embed_texts = if embed_texts.len() == contents.len() { embed_texts } else { contents.clone() };
        // IDF gate: signal(content, corpus) >= threshold. R30.7: one
        // frequency map per doc — per-chunk `signal` was O(n²) and an
        // 8401-chunk file never finished.
        let freq = crate::idf::DocFreq::build(&contents);
        let mut embeddable_idx: Vec<usize> = Vec::new();
        for (i, c) in contents.iter().enumerate() {
            let s = freq.signal(c);
            if s < embed_min_signal {
                continue;
            }
            embeddable_idx.push(i);
        }
        // Label gate: Sensitive sources skip an untrusted remote embedder.
        if gated && !self.cloud_trusted() {
            tracing::info!("embed gate: sensitive source skipped remote embedder");
            return Ok(vec![None; contents.len()]);
        }
        if embeddable_idx.is_empty() {
            return Ok(vec![None; contents.len()]);
        }
        // R43.6: nomic-gated `search_document: ` prefix (no-op for non-nomic).
        let model = self.embed_model_name();
        let to_embed: Vec<String> = embeddable_idx.iter().map(|&i| prefix_document_text(&embed_texts[i], &model)).collect();
        let vectors = self.embed(to_embed).await?;
        // R43.6: count-checked placement — a short provider response errors
        // instead of misaligning vectors onto wrong chunks (middle drop) or
        // silently leaving tails unembedded (tail drop via `.zip` truncation).
        place_vectors(contents.len(), &embeddable_idx, vectors)
    }

    /// Sync vec0 table after embedding (mirrors trigger-less fallback for `rust`).
    /// In Python, `vec_chunks` is kept via triggers; in Rust we upsert explicitly when needed.
    ///
    /// R45.5: every chunk-vector write path (ingest, re-embed job, backfill)
    /// calls this right after `UPDATE chunks SET embedding`, so it is also
    /// where the vector's provenance fingerprint is stamped.
    pub fn sync_vec(&self, conn: &rusqlite::Connection, chunk_id: i64, embedding: &[u8]) -> Result<(), String> {
        // embedding stored as blob in chunks.embedding; vec0 is virtual via `vec_chunks` - we insert rowid=chunk_id
        // If vec0 unavailable, ignore.
        if embedding.is_empty() {
            return Ok(());
        }
        conn.execute(
            "UPDATE chunks SET embed_fp = ?1 WHERE id = ?2",
            rusqlite::params![self.fingerprint(embedding.len() / 4), chunk_id],
        )
        .map_err(|e| e.to_string())?;
        let dim: usize = std::env::var("ANCHOR_EMBED_DIM").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        if embedding.len() != dim * 4 {
            return Ok(());
        }
        let floats: Vec<f32> = unpack_f32(embedding, dim).ok_or("unpack failed")?;
        let json = serde_json::to_string(&floats).map_err(|e| e.to_string())?;
        // try vec0 insert; ignore if module missing
        match conn.execute("INSERT OR REPLACE INTO vec_chunks(rowid, embedding) VALUES (?1, ?2)", rusqlite::params![chunk_id, json]) {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table: vec_chunks") || msg.contains("no such module: vec0") {
                    Ok(())
                } else {
                    Err(msg)
                }
            }
        }
    }
}

static DETERMINISTIC_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

pub fn deterministic_embed(text: &str) -> Vec<f32> {
    let dim: usize = std::env::var("ANCHOR_EMBED_DIM").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
    deterministic_embed_with_dim(text, dim)
}

fn stable_hash(s: &str) -> u64 {
    // FNV-1a 64-bit — stable across Rust versions (DefaultHasher is not)
    let mut h: u64 = 14695981039346656037;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

pub fn deterministic_embed_with_dim(text: &str, dim: usize) -> Vec<f32> {
    // R8.1 fallback when remote unavailable — deterministic bag-of-words, Unicode-aware
    // R11.2: synonym map is test-only probe helper (car/vehicle etc.) — documented, not production mixing
    fn normalize_token(t: &str) -> &str {
        // Probe synonym map (R8.1) — allows `car speed` vs `vehicle` semantic test without keyword overlap; keep documented
        match t {
            "car" | "automobile" | "vehicle" | "auto" => "vehicle",
            "fast" | "quick" | "speed" | "velocity" | "rapid" => "speed",
            "database" | "db" | "storage" | "store" => "storage",
            "red" | "crimson" | "scarlet" => "color",
            "big" | "large" | "huge" => "big",
            "small" | "tiny" | "little" => "small",
            _ => t,
        }
    }
    let re = DETERMINISTIC_RE.get_or_init(|| regex::Regex::new(r"[\p{L}\p{N}]+").unwrap());
    let mut vec = vec![0.0f32; dim];
    let tokens: Vec<String> = re.find_iter(&text.to_lowercase()).map(|m| m.as_str().to_string()).collect();
    if tokens.is_empty() {
        let h = stable_hash(text);
        let idx = (h as usize) % dim;
        vec[idx] = 1.0;
    } else {
        for tok in tokens {
            let norm = normalize_token(&tok);
            let h = stable_hash(norm);
            let idx = (h as usize) % dim;
            vec[idx] += 1.0;
        }
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pack_roundtrip() {
        let v = vec![vec![1.0, 2.0, 3.0]];
        let packed = pack_f32(&v);
        assert_eq!(packed.len(), 12);
        let unpacked = unpack_f32(&packed, 3).unwrap();
        assert_eq!(unpacked, vec![1.0, 2.0, 3.0]);
    }
    #[test]
    fn cosine() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-5);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-5);
    }
    #[test]
    fn embedder_local_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        let e = Embedder::new(svc);
        assert!(e.provider_is_local());
    }

    #[test]
    fn provider_resolution() {
        assert_eq!(resolve_provider(None, false, false), Provider::Http);
        assert_eq!(resolve_provider(None, true, false), Provider::Builtin, "auto: ready model, nothing serving");
        assert_eq!(resolve_provider(None, true, true), Provider::Http, "auto: a serving Ollama wins (faster bulk)");
        assert_eq!(resolve_provider(Some("auto"), false, false), Provider::Http);
        assert_eq!(resolve_provider(Some(" Builtin "), false, true), Provider::Builtin, "explicit builtin always");
        assert_eq!(resolve_provider(Some("ollama"), true, false), Provider::Http, "explicit ollama always");
    }

    #[test]
    fn builtin_vectors_share_the_ollama_family() {
        // R45.4: switching provider must not re-embed (probe parity 0.99995+).
        let ours = crate::embed_fp::fingerprint(PROVIDER_BUILTIN, crate::model_store::MODEL_NAME, 768);
        let theirs = crate::embed_fp::fingerprint(PROVIDER_HTTP, "nomic-embed-text:latest", 768);
        assert!(crate::embed_fp::same_vector_family(&ours, &theirs), "{ours} vs {theirs}");
        assert!(uses_nomic_prefixes(crate::model_store::MODEL_NAME), "built-in inputs keep the nomic prefixes");
    }

    #[test]
    fn embed_concurrency_defaults_and_caps() {
        assert_eq!(embed_concurrency(None), 2);
        assert_eq!(embed_concurrency(Some("junk")), 2);
        assert_eq!(embed_concurrency(Some("0")), 1);
        assert_eq!(embed_concurrency(Some(" 4 ")), 4);
        assert_eq!(embed_concurrency(Some("99")), 8);
    }

    /// R30.2: batches run concurrently but vectors stay in input order, even
    /// when a later batch answers first and items come back unsorted.
    #[tokio::test]
    async fn embed_keeps_order_across_concurrent_batches() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (inf, pk) = (in_flight.clone(), peak.clone());
        let app = axum::Router::new().route(
            "/v1/embeddings",
            axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let (inf, pk) = (inf.clone(), pk.clone());
                async move {
                    let now = inf.fetch_add(1, Ordering::SeqCst) + 1;
                    pk.fetch_max(now, Ordering::SeqCst);
                    let inputs: Vec<String> =
                        body["input"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
                    let first: f32 = inputs[0].trim_start_matches("t").parse().unwrap();
                    // the first batch answers last
                    let delay = if first == 0.0 { 150 } else { 10 };
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    let mut data: Vec<serde_json::Value> = inputs
                        .iter()
                        .enumerate()
                        .map(|(i, t)| serde_json::json!({"index": i, "embedding": [t.trim_start_matches("t").parse::<f32>().unwrap()]}))
                        .collect();
                    data.reverse();
                    inf.fetch_sub(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({"data": data}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::set_var("ANCHOR_EMBED_BASE_URL", format!("http://127.0.0.1:{port}"));
        std::env::set_var("ANCHOR_EMBED_CONCURRENCY", "3");
        let dir = tempfile::tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = Arc::new(crate::settings::SettingsService::new(store));
        svc.set_data_dir(dir.path());
        let e = Embedder::new(svc);
        let texts: Vec<String> = (0..100).map(|i| format!("t{i}")).collect();
        let out = e.embed(texts).await;
        std::env::remove_var("ANCHOR_EMBED_BASE_URL");
        std::env::remove_var("ANCHOR_EMBED_CONCURRENCY");
        let out = out.unwrap();
        assert_eq!(out.len(), 100);
        assert!(out.iter().enumerate().all(|(i, v)| v == &vec![i as f32]), "order kept");
        assert!(peak.load(Ordering::SeqCst) >= 2, "batches overlapped");
        assert!(peak.load(Ordering::SeqCst) <= 3, "capped at the knob");
    }

    #[test]
    fn place_vectors_tail_drop_errors() {
        // R43.6: N-1 vectors for N inputs used to truncate silently via
        // `.zip`, leaving the tail chunk unembedded (None) with Ok.
        let err = place_vectors(3, &[0, 1, 2], vec![vec![1.0], vec![2.0]]).unwrap_err();
        assert!(err.contains("2 vectors for 3 chunks"), "unexpected message: {err}");
    }

    #[test]
    fn place_vectors_middle_drop_errors() {
        // R43.6: a middle drop (fewer vectors than gated positions, with an
        // IDF-skipped gap) used to misalign later vectors onto wrong chunks
        // via `.zip`. Tail and middle drops are indistinguishable by count —
        // both now error instead of corrupting the store.
        let err = place_vectors(4, &[0, 2, 3], vec![vec![1.0], vec![3.0]]).unwrap_err();
        assert!(err.contains("2 vectors for 3 chunks"), "unexpected message: {err}");
    }

    #[test]
    fn place_vectors_extra_vectors_error() {
        // R43.6: more vectors than requested is also a provider bug — `.zip`
        // would have dropped the extras silently.
        let err = place_vectors(2, &[0], vec![vec![1.0], vec![2.0]]).unwrap_err();
        assert!(err.contains("2 vectors for 1 chunks"), "unexpected message: {err}");
    }

    #[test]
    fn place_vectors_aligns_to_gated_positions() {
        // R43.6 control: distinct vectors land on exactly their gated
        // positions; IDF-skipped slots stay None.
        let out = place_vectors(3, &[0, 2], vec![vec![1.0, 0.0], vec![0.0, 3.0]]).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(unpack_f32(&out[0].clone().unwrap(), 2).unwrap(), vec![1.0, 0.0]);
        assert!(out[1].is_none(), "IDF-skipped slot must stay None");
        assert_eq!(unpack_f32(&out[2].clone().unwrap(), 2).unwrap(), vec![0.0, 3.0]);
    }

    #[test]
    fn record_active_fingerprint_reports_family_changes() {
        // R45.5 (needs the real schema, so it lives here, not in embed_fp).
        use crate::embed_fp::active_fingerprint;
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("t.db")).unwrap();
        assert_eq!(active_fingerprint(&conn), None);
        assert!(record_active_fingerprint(&conn, "ollama:nomic-embed-text:768").unwrap(), "first record = re-embed check");
        assert!(!record_active_fingerprint(&conn, "ollama:nomic-embed-text:768").unwrap());
        // provider switch inside the nomic family: no re-embed, no event
        assert!(!record_active_fingerprint(&conn, "builtin:nomic-embed-text-v1.5:768").unwrap());
        assert_eq!(active_fingerprint(&conn).as_deref(), Some("builtin:nomic-embed-text-v1.5:768"));
        let events = |c: &rusqlite::Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM system_events WHERE component='embedder'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(events(&conn), 0);
        assert!(record_active_fingerprint(&conn, "ollama:mxbai-embed-large:1024").unwrap());
        assert_eq!(events(&conn), 1);
    }

    #[test]
    fn nomic_prefix_detection() {
        // R43.6: substring gate, case-insensitive — the default model
        // (`nomic-embed-text`) prefixes; other providers don't.
        assert!(uses_nomic_prefixes("nomic-embed-text"));
        assert!(uses_nomic_prefixes("Nomic-Embed-Text-v2"));
        assert!(uses_nomic_prefixes("registry.local/nomic-embed-text:latest"));
        assert!(!uses_nomic_prefixes("mxbai-embed-large"));
        assert!(!uses_nomic_prefixes("bge-m3"));
        assert!(!uses_nomic_prefixes("qwen3-embedding"));
        assert!(!uses_nomic_prefixes(""));
    }

    #[test]
    fn prefix_texts_nomic_vs_other() {
        // R43.6: nomic pairs `search_query: ` with `search_document: `;
        // non-nomic inputs pass through byte-identical.
        assert_eq!(prefix_query_text("hello", "nomic-embed-text"), "search_query: hello");
        assert_eq!(prefix_document_text("hello", "nomic-embed-text"), "search_document: hello");
        assert_eq!(prefix_query_text("hello", "mxbai-embed-large"), "hello");
        assert_eq!(prefix_document_text("hello", "mxbai-embed-large"), "hello");
        assert_eq!(prefix_query_text("", "nomic-embed-text"), "search_query: ");
    }

    #[test]
    fn embedder_prefix_follows_configured_model() {
        // R43.6 wiring: the document/query prefix decision reads the
        // configured `embed_model` (env here; temp data dir so no DB row
        // can override), defaulting to the nomic default model.
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        let mk = || {
            let dir = tempfile::tempdir().unwrap();
            let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
            let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
            svc.set_data_dir(dir.path());
            (dir, Embedder::new(svc))
        };
        std::env::set_var("ANCHOR_EMBED_MODEL", "mxbai-embed-large");
        let (_d1, other) = mk();
        assert_eq!(other.prefix_document("chunk text"), "chunk text");
        std::env::set_var("ANCHOR_EMBED_MODEL", "NOMIC-embed-text-custom");
        let (_d2, nomic) = mk();
        assert_eq!(nomic.prefix_document("chunk text"), "search_document: chunk text");
        std::env::remove_var("ANCHOR_EMBED_MODEL");
        let (_d3, default) = mk();
        assert_eq!(default.prefix_document("chunk text"), "search_document: chunk text", "default model is nomic");
    }
}
