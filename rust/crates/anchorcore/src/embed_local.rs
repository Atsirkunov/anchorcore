//! R45.4: built-in embedder — nomic-embed-text v1.5 on candle, in process.
//! Metal on macOS (CPU when Metal is unavailable), pure-Rust CPU elsewhere;
//! f16 weights everywhere (probe 2026-10-21: f16 parity ≥ 0.99995 vs Ollama
//! on every backend, and faster than f32 on Metal and pure CPU).
//!
//! The probe's three must-keeps: the attention mask goes in as `U8` (Metal
//! has no U32/F16 `where_cond`), hidden states are cast to F32 before mean
//! pooling, and batches are length-sorted (less padding) with the output
//! returned in input order.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::nomic_bert::{l2_normalize, mean_pooling, Config, NomicBertModel};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// Inputs per forward pass (the probe's batch size).
const BATCH: usize = 16;
/// Token cap per input; chunks are ≤ ~1000 chars, far below it.
const MAX_TOKENS: usize = 2048;

pub struct Engine {
    model: NomicBertModel,
    tokenizer: Tokenizer,
    device: Device,
    pub backend: &'static str,
}

fn pick_device() -> (Device, &'static str) {
    #[cfg(target_os = "macos")]
    {
        if std::env::var("ANCHOR_EMBED_DEVICE").as_deref() != Ok("cpu") {
            if let Ok(d) = Device::new_metal(0) {
                return (d, "metal");
            }
        }
    }
    (Device::Cpu, "cpu")
}

impl Engine {
    /// Load from a `model_store::ready` directory.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let (device, backend) = pick_device();
        let cfg: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json")).map_err(|e| e.to_string())?)
            .map_err(|e| format!("config.json: {e}"))?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| format!("tokenizer.json: {e}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
            .map_err(|e| e.to_string())?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        let weights = dir.join(crate::model_store::WEIGHTS_F16);
        // SAFETY: the weights file is ours (verified by `model_store`) and
        // not modified while mapped.
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F16, &device) }.map_err(|e| e.to_string())?;
        let model = NomicBertModel::load(vb, &cfg).map_err(|e| e.to_string())?;
        Ok(Self { model, tokenizer, device, backend })
    }

    /// One padded batch → unit vectors, in batch order.
    fn forward(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let enc = self.tokenizer.encode_batch(texts.to_vec(), true).map_err(|e| e.to_string())?;
        let ids: Vec<Vec<u32>> = enc.iter().map(|e| e.get_ids().to_vec()).collect();
        let mask: Vec<Vec<u8>> = enc.iter().map(|e| e.get_attention_mask().iter().map(|&m| m as u8).collect()).collect();
        let ids = Tensor::new(ids, &self.device).map_err(|e| e.to_string())?;
        let mask = Tensor::new(mask, &self.device).map_err(|e| e.to_string())?;
        let hidden = self.model.forward(&ids, None, Some(&mask)).map_err(|e| e.to_string())?;
        let hidden = hidden.to_dtype(DType::F32).map_err(|e| e.to_string())?;
        let pooled = mean_pooling(&hidden, &mask).and_then(|p| l2_normalize(&p)).map_err(|e| e.to_string())?;
        pooled.to_vec2::<f32>().map_err(|e| e.to_string())
    }

    /// Embed in length-sorted batches; output in input order.
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let order = length_order(texts);
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for idx in order.chunks(BATCH) {
            let batch: Vec<String> = idx.iter().map(|&i| texts[i].clone()).collect();
            for (&i, v) in idx.iter().zip(self.forward(&batch)?) {
                out[i] = Some(v);
            }
        }
        out.into_iter().map(|v| v.ok_or_else(|| "missing vector".to_string())).collect()
    }
}

/// Input positions sorted by length (stable), so each batch pads little.
fn length_order(texts: &[String]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..texts.len()).collect();
    idx.sort_by_key(|&i| texts[i].len());
    idx
}

/// The loaded engine, shared by every embed call; inference is serialized
/// (one device, and concurrent forwards only contend).
static ENGINE: Mutex<Option<(PathBuf, Arc<Engine>)>> = Mutex::new(None);
static RUN: Mutex<()> = Mutex::new(());

fn engine_for(dir: &Path) -> Result<Arc<Engine>, String> {
    let mut slot = ENGINE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((loaded, engine)) = slot.as_ref() {
        if loaded == dir {
            return Ok(engine.clone());
        }
    }
    let started = std::time::Instant::now();
    let engine = Arc::new(Engine::load(dir)?);
    tracing::info!("built-in embedder loaded ({}, {} ms)", engine.backend, started.elapsed().as_millis());
    *slot = Some((dir.to_path_buf(), engine.clone()));
    Ok(engine)
}

/// Blocking: load on first use, then embed. Call from `spawn_blocking`.
pub fn embed_blocking(dir: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let engine = engine_for(dir)?;
    let _run = RUN.lock().unwrap_or_else(|p| p.into_inner());
    engine.embed(texts)
}

/// Which backend a loaded engine uses (`None` before the first embed).
pub fn loaded_backend() -> Option<&'static str> {
    ENGINE.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(|(_, e)| e.backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_order_is_stable_and_complete() {
        let texts: Vec<String> = ["ccc", "a", "bb", "a2", ""].iter().map(|s| s.to_string()).collect();
        assert_eq!(length_order(&texts), vec![4, 1, 2, 3, 0]);
    }

    /// Opt-in: `ANCHOR_TEST_BUILTIN_MODEL_DIR=<ready model dir>` runs the
    /// real model (the suite must pass without weights).
    #[test]
    fn real_model_matches_itself_across_batch_orders() {
        let Ok(dir) = std::env::var("ANCHOR_TEST_BUILTIN_MODEL_DIR") else { return };
        let texts: Vec<String> = (0..20).map(|i| format!("search_document: payments note number {i} {}", "x".repeat(i * 7))).collect();
        let all = embed_blocking(Path::new(&dir), &texts).unwrap();
        assert_eq!(all.len(), 20);
        assert_eq!(all[0].len(), 768);
        let one = embed_blocking(Path::new(&dir), &texts[5..6]).unwrap();
        let cos: f32 = all[5].iter().zip(&one[0]).map(|(a, b)| a * b).sum();
        assert!(cos > 0.9999, "padding must not change a vector: {cos}");
    }

    /// Opt-in timing probe: `ANCHOR_TEST_BUILTIN_MODEL_DIR` + `ANCHOR_TEST_BUILTIN_BENCH=1`.
    #[test]
    fn real_model_call_timings() {
        let (Ok(dir), Ok(_)) = (std::env::var("ANCHOR_TEST_BUILTIN_MODEL_DIR"), std::env::var("ANCHOR_TEST_BUILTIN_BENCH")) else { return };
        let texts: Vec<String> = (0..12).map(|i| format!("search_document: Doc {i} > Part\n{}", "lorem ipsum dolor sit amet ".repeat(14 + i))).collect();
        for round in 0..40 {
            let t = std::time::Instant::now();
            embed_blocking(Path::new(&dir), &texts).unwrap();
            if round % 5 == 0 {
                eprintln!("round {round}: {} ms", t.elapsed().as_millis());
            }
        }
    }

    /// Opt-in parity with Ollama (`ANCHOR_TEST_BUILTIN_MODEL_DIR` + a local
    /// Ollama serving `nomic-embed-text`): same prefixed inputs, cosine ≥ 0.999.
    #[tokio::test]
    async fn real_model_matches_ollama() {
        let Ok(dir) = std::env::var("ANCHOR_TEST_BUILTIN_MODEL_DIR") else { return };
        let texts: Vec<String> = [
            "search_document: Payments > Refunds\nRefunds post to the original card within five days.",
            "search_query: who approves price changes?",
            "search_document: The nightly ledger close runs at 02:00 UTC and pages on-call when it fails.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let body = serde_json::json!({"model": "nomic-embed-text", "input": texts});
        let resp: serde_json::Value = reqwest::Client::new()
            .post("http://localhost:11434/v1/embeddings")
            .json(&body)
            .send()
            .await
            .expect("ollama reachable")
            .json()
            .await
            .unwrap();
        let ours = tokio::task::spawn_blocking({
            let texts = texts.clone();
            move || embed_blocking(Path::new(&dir), &texts)
        })
        .await
        .unwrap()
        .unwrap();
        for (i, item) in resp["data"].as_array().unwrap().iter().enumerate() {
            let theirs: Vec<f32> = item["embedding"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let cos = crate::embedder::cosine_similarity(&ours[i], &theirs);
            assert!(cos >= 0.999, "text {i}: cosine {cos}");
            eprintln!("parity text {i}: {cos:.6}");
        }
    }
}
