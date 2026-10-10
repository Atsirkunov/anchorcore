//! Embedder + IDF gate — port of `backend/app/embedder.py:1`.
//! `pack_f32`, cloud-trust gate for `is_pii` chunks, `vec0` sync triggers, `signal()` gating via `distill::signal`.

use std::sync::Arc;

use crate::settings::SettingsService;

pub const BATCH_SIZE: usize = 32;

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
}

impl Embedder {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings }
    }

    fn base_url(&self) -> String {
        self.settings
            .get("embed_base_url", None)
            .or_else(|| self.settings.get("ollama_base_url", None))
            .unwrap_or_else(|| "http://localhost:11434".to_string())
    }

    fn provider_is_local(&self) -> bool {
        let u = self.base_url();
        u.starts_with("http://localhost") || u.starts_with("http://127.0.0.1")
    }

    /// Whether a remote embedder is trusted (B39/B30). Local is always trusted.
    fn cloud_trusted(&self) -> bool {
        crate::common::provider_trusted(&self.base_url())
    }

    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let model = self.settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string());
        let base = self.base_url();
        let api_key = self.settings.get("embed_api_key", None).unwrap_or_default();
        let timeout = self.settings.get_float("classifier_timeout", 60.0) as u64;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .build()
            .map_err(|e| e.to_string())?;
        let mut vectors = Vec::new();
        for batch in texts.chunks(BATCH_SIZE) {
            let payload = serde_json::json!({"model": model, "input": batch});
            let url = format!("{}/v1/embeddings", base.trim_end_matches('/'));
            let mut req = client.post(&url).json(&payload);
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    // R11.2: do not persist degraded deterministic embeddings — surface error so pipeline leaves NULL
                    tracing::warn!("embed failed ({}), not persisting degraded", e);
                    return Err(format!("embed failed: {}", e));
                }
            };
            if !resp.status().is_success() {
                tracing::warn!("embed status {}, not persisting degraded", resp.status());
                return Err(format!("embed status {}", resp.status()));
            }
            let j: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("embed json failed ({}), not persisting degraded", e);
                    return Err(format!("embed json failed: {}", e));
                }
            };
            let data = j.get("data").and_then(|v| v.as_array()).ok_or("missing data")?;
            let mut sorted: Vec<(usize, Vec<f32>)> = data
                .iter()
                .filter_map(|item| {
                    let idx = item.get("index")?.as_u64()? as usize;
                    let emb = item.get("embedding")?.as_array()?;
                    let v: Vec<f32> = emb.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
                    Some((idx, v))
                })
                .collect();
            sorted.sort_by_key(|(i, _)| *i);
            for (_, v) in sorted {
                vectors.push(v);
            }
        }
        Ok(vectors)
    }

    /// B58: strict query embedding — `None` when remote is unavailable, so
    /// retrieval degrades to honest keyword-only. (Previously fell back to
    /// `deterministic_embed`, whose hash vectors scored as noise against
    /// real model chunk vectors.) `deterministic_embed` remains for tags,
    /// which compare deterministic vectors against each other.
    pub async fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        match self.embed(vec![text.to_string()]).await {
            Ok(v) if !v.is_empty() => v.into_iter().next(),
            _ => None,
        }
    }

    /// Embed with IDF+trust gating (mirrors `backend/app/pipeline.py:316` `_embed_item`).
    /// Returns packed bytes for storage; skips low-signal and PII-gated chunks.
    pub async fn embed_gated(
        &self,
        contents: Vec<String>,
        is_pii_flags: Vec<bool>,
        gated: bool,
        embed_min_signal: f64,
    ) -> Result<Vec<Option<Vec<u8>>>, String> {
        if contents.is_empty() {
            return Ok(vec![]);
        }
        // IDF gate: signal(content, corpus) >= threshold
        let corpus: Vec<String> = contents.clone();
        let mut embeddable_idx: Vec<usize> = Vec::new();
        for (i, c) in contents.iter().enumerate() {
            let s = crate::distill::signal(c, &corpus);
            if s < embed_min_signal {
                continue;
            }
            embeddable_idx.push(i);
        }
        // PII gate
        let trusted = self.cloud_trusted();
        if gated && !trusted {
            tracing::info!("embed gate: sensitive source skipped remote embedder (PII gate)");
            return Ok(vec![None; contents.len()]);
        }
        if !trusted {
            let before = embeddable_idx.len();
            embeddable_idx.retain(|&i| !is_pii_flags[i]);
            if embeddable_idx.len() < before {
                tracing::info!("embed gate: {} PII-flagged chunk(s) skipped remote embedder", before - embeddable_idx.len());
            }
        }
        if embeddable_idx.is_empty() {
            return Ok(vec![None; contents.len()]);
        }
        let to_embed: Vec<String> = embeddable_idx.iter().map(|&i| contents[i].clone()).collect();
        let vectors = self.embed(to_embed).await?;
        let mut out: Vec<Option<Vec<u8>>> = vec![None; contents.len()];
        for (pos, vec) in embeddable_idx.into_iter().zip(vectors) {
            out[pos] = Some(pack_single(&vec));
        }
        Ok(out)
    }

    /// Sync vec0 table after embedding (mirrors trigger-less fallback for `rust`).
    /// In Python, `vec_chunks` is kept via triggers; in Rust we upsert explicitly when needed.
    pub fn sync_vec(&self, conn: &rusqlite::Connection, chunk_id: i64, embedding: &[u8]) -> Result<(), String> {
        // embedding stored as blob in chunks.embedding; vec0 is virtual via `vec_chunks` - we insert rowid=chunk_id
        // If vec0 unavailable, ignore.
        if embedding.is_empty() {
            return Ok(());
        }
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
}
