//! R45.4: the built-in embedder's weights on disk — nomic-embed-text v1.5,
//! pinned to one Hugging Face revision with per-file SHA-256 (measured in
//! `docs/probes/2026-10-21-embedding-engine.md` §1).
//!
//! Layout: `<models root>/nomic-embed-text-v1.5/` holds `config.json`,
//! `tokenizer.json` and `model.f16.safetensors` (HF ships f32 only; we
//! verify the f32 download, convert once, keep the f16 half). The root is
//! `ANCHOR_MODELS_DIR` (the Team image prefetches there) or
//! `{data_dir}/models`.
//!
//! Getting the files is always an explicit action — the wizard/Settings
//! button, `POST /system/embed-model/download`, or `anchorcore embed-model
//! fetch` — never a side effect of boot (manual-first, like updates). An
//! offline machine can drop the three upstream files into the directory
//! instead; `prepare` verifies and converts them without the network.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

pub const MODEL_NAME: &str = "nomic-embed-text-v1.5";
pub const REPO: &str = "nomic-ai/nomic-embed-text-v1.5";
pub const REVISION: &str = "e9b6763023c676ca8431644204f50c2b100d9aab";
/// Converted weights (the only weights file kept after `prepare`).
pub const WEIGHTS_F16: &str = "model.f16.safetensors";
const WEIGHTS_F32: &str = "model.safetensors";
/// Written after the f16 file is complete — `ready` never hashes 274 MB.
const READY_STAMP: &str = "READY";

/// One pinned upstream file.
pub struct ModelFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const FILES: [ModelFile; 3] = [
    ModelFile { name: "config.json", bytes: 2_538, sha256: "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b" },
    ModelFile { name: "tokenizer.json", bytes: 711_396, sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66" },
    ModelFile {
        name: WEIGHTS_F32,
        bytes: 546_938_168,
        sha256: "9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c",
    },
];

/// Total upstream download size (for the UI before anything starts).
pub fn download_bytes() -> u64 {
    FILES.iter().map(|f| f.bytes).sum()
}

pub fn models_root(data_dir: &Path) -> PathBuf {
    match std::env::var("ANCHOR_MODELS_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => data_dir.join("models"),
    }
}

pub fn model_dir(root: &Path) -> PathBuf {
    root.join(MODEL_NAME)
}

/// Everything the engine loads is present and was finished by `prepare`.
pub fn ready(dir: &Path) -> bool {
    ["config.json", "tokenizer.json", WEIGHTS_F16, READY_STAMP].iter().all(|f| dir.join(f).is_file())
}

fn file_url(base: &str, name: &str) -> String {
    format!("{}/{REPO}/resolve/{REVISION}/{name}", base.trim_end_matches('/'))
}

/// `ANCHOR_MODELS_BASE_URL` points downloads at a mirror (tests use a mock).
fn download_base() -> String {
    match std::env::var("ANCHOR_MODELS_BASE_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => "https://huggingface.co".to_string(),
    }
}

// --- progress, shared with the status endpoint ---

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Downloading,
    Converting,
    Ready,
    Failed,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub file: Option<String>,
    pub error: Option<String>,
}

static PROGRESS: Mutex<Progress> =
    Mutex::new(Progress { phase: Phase::Idle, done_bytes: 0, total_bytes: 0, file: None, error: None });

fn progress_lock() -> std::sync::MutexGuard<'static, Progress> {
    PROGRESS.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn progress() -> Progress {
    progress_lock().clone()
}

fn set_progress(f: impl FnOnce(&mut Progress)) {
    f(&mut progress_lock());
}

/// Claim the single preparation slot; false when one is already running.
pub fn try_begin() -> bool {
    let mut p = progress_lock();
    if matches!(p.phase, Phase::Downloading | Phase::Converting) {
        return false;
    }
    *p = Progress { phase: Phase::Downloading, done_bytes: 0, total_bytes: download_bytes(), file: None, error: None };
    true
}

// --- verify / download / convert ---

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// The file at `path` is exactly the pinned upstream file.
fn verified(path: &Path, file: &ModelFile) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    meta.len() == file.bytes && sha256_file(path).map(|h| h == file.sha256).unwrap_or(false)
}

/// Download `file` into `dir`, resuming a `.part` left by an earlier run.
/// The finished file is renamed into place only after its checksum matches.
async fn download(dir: &Path, file: &ModelFile, done_before: u64) -> Result<(), String> {
    let part = dir.join(format!("{}.part", file.name));
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > file.bytes {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let mut req = reqwest::Client::new().get(file_url(&download_base(), file.name));
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().await.map_err(|e| format!("download {}: {e}", file.name))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("download {}: HTTP {status}", file.name));
    }
    // A server that ignores Range answers 200 with the whole body.
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resumed {
        have = 0;
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(|e| format!("open {}: {e}", part.display()))?;
    while let Some(bytes) = resp.chunk().await.map_err(|e| format!("download {}: {e}", file.name))? {
        out.write_all(&bytes).map_err(|e| format!("write {}: {e}", part.display()))?;
        have += bytes.len() as u64;
        set_progress(|p| p.done_bytes = done_before + have);
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    if !verified(&part, file) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: size or SHA-256 mismatch — removed, download again", file.name));
    }
    std::fs::rename(&part, dir.join(file.name)).map_err(|e| e.to_string())
}

/// Make each upstream file present and verified: a dropped-in file that
/// checks out is used as is; otherwise download (or fail when offline).
async fn fetch_files(dir: &Path, allow_network: bool) -> Result<(), String> {
    let mut done = 0u64;
    for file in &FILES {
        let path = dir.join(file.name);
        let skip = file.name == WEIGHTS_F32 && dir.join(WEIGHTS_F16).is_file() && !path.exists();
        set_progress(|p| p.file = Some(file.name.to_string()));
        if skip || verified(&path, file) {
            done += file.bytes;
            set_progress(|p| p.done_bytes = done);
            continue;
        }
        if !allow_network {
            return Err(format!("{} is missing or does not match the pinned checksum ({})", path.display(), file.sha256));
        }
        download(dir, file, done).await?;
        done += file.bytes;
    }
    Ok(())
}

/// f32 safetensors → f16 safetensors (written to a temp name, then renamed),
/// then the f32 file is removed: the engine runs f16 on Metal and pure CPU.
fn convert_to_f16(dir: &Path) -> Result<(), String> {
    let src = dir.join(WEIGHTS_F32);
    if !src.is_file() {
        return if dir.join(WEIGHTS_F16).is_file() { Ok(()) } else { Err("no weights to convert".to_string()) };
    }
    let tensors = candle_core::safetensors::load(&src, &candle_core::Device::Cpu).map_err(|e| e.to_string())?;
    let mut half = std::collections::HashMap::with_capacity(tensors.len());
    for (name, t) in tensors {
        let t = if t.dtype() == candle_core::DType::F32 { t.to_dtype(candle_core::DType::F16).map_err(|e| e.to_string())? } else { t };
        half.insert(name, t);
    }
    let tmp = dir.join(format!("{WEIGHTS_F16}.tmp"));
    candle_core::safetensors::save(&half, &tmp).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(WEIGHTS_F16)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&src);
    Ok(())
}

/// Download (when allowed) or adopt dropped-in files, verify, convert,
/// stamp ready. Progress is published for the status endpoint; the caller
/// claims the slot with `try_begin` first.
pub async fn prepare(dir: &Path, allow_network: bool) -> Result<(), String> {
    let res = prepare_inner(dir, allow_network).await;
    set_progress(|p| match &res {
        Ok(()) => {
            p.phase = Phase::Ready;
            p.file = None;
        }
        Err(e) => {
            p.phase = Phase::Failed;
            p.error = Some(e.clone());
        }
    });
    res
}

async fn prepare_inner(dir: &Path, allow_network: bool) -> Result<(), String> {
    if ready(dir) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    fetch_files(dir, allow_network).await?;
    set_progress(|p| p.phase = Phase::Converting);
    let d = dir.to_path_buf();
    tokio::task::spawn_blocking(move || convert_to_f16(&d)).await.map_err(|e| e.to_string())??;
    std::fs::write(dir.join(READY_STAMP), format!("{REPO}@{REVISION}\n")).map_err(|e| e.to_string())?;
    tracing::info!("built-in embedding model ready at {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_readiness() {
        let dir = tempfile::tempdir().unwrap();
        let m = model_dir(dir.path());
        assert!(m.ends_with(MODEL_NAME));
        assert!(!ready(&m));
        std::fs::create_dir_all(&m).unwrap();
        for f in ["config.json", "tokenizer.json", WEIGHTS_F16] {
            std::fs::write(m.join(f), b"x").unwrap();
        }
        assert!(!ready(&m), "no stamp yet: a half-converted dir is not ready");
        std::fs::write(m.join(READY_STAMP), b"x").unwrap();
        assert!(ready(&m));
        assert_eq!(download_bytes(), 2_538 + 711_396 + 546_938_168);
    }

    #[test]
    fn urls_pin_the_revision() {
        assert_eq!(
            file_url("https://huggingface.co/", "config.json"),
            format!("https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/{REVISION}/config.json")
        );
    }

    #[test]
    fn verified_checks_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        let good = ModelFile { name: "f", bytes: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" };
        assert!(verified(&p, &good));
        let wrong_size = ModelFile { name: "f", bytes: 4, sha256: good.sha256 };
        assert!(!verified(&p, &wrong_size));
        let wrong_hash = ModelFile { name: "f", bytes: 3, sha256: "00" };
        assert!(!verified(&p, &wrong_hash));
    }

    /// Mock mirror serving `body` with Range support; counts requests that
    /// asked for a range.
    async fn mirror(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ranged = std::sync::Arc::new(AtomicUsize::new(0));
        let r = ranged.clone();
        let app = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| {
            let (body, r) = (body.clone(), r.clone());
            async move {
                let start = headers
                    .get("range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok());
                match start {
                    Some(s) => {
                        r.fetch_add(1, Ordering::SeqCst);
                        (axum::http::StatusCode::PARTIAL_CONTENT, body[s..].to_vec())
                    }
                    None => (axum::http::StatusCode::OK, body),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}"), ranged)
    }

    #[tokio::test]
    async fn download_resumes_and_verifies() {
        let body = b"0123456789abcdef".to_vec();
        let sha = hex::encode(Sha256::digest(&body));
        let file = ModelFile { name: "w.bin", bytes: body.len() as u64, sha256: Box::leak(sha.into_boxed_str()) };
        let (base, ranged) = mirror(body.clone()).await;
        let _g = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::set_var("ANCHOR_MODELS_BASE_URL", &base);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("w.bin.part"), &body[..6]).unwrap();
        let res = download(dir.path(), &file, 0).await;
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(std::fs::read(dir.path().join("w.bin")).unwrap(), body);
        assert!(!dir.path().join("w.bin.part").exists());
        assert_eq!(ranged.load(std::sync::atomic::Ordering::SeqCst), 1, "resumed with a Range request");
        // A checksum mismatch removes the partial file and fails.
        let bad = ModelFile { name: "x.bin", bytes: body.len() as u64, sha256: "00" };
        let err = download(dir.path(), &bad, 0).await.unwrap_err();
        std::env::remove_var("ANCHOR_MODELS_BASE_URL");
        assert!(err.contains("mismatch"), "{err}");
        assert!(!dir.path().join("x.bin.part").exists() && !dir.path().join("x.bin").exists());
    }

    #[tokio::test]
    async fn offline_prepare_names_the_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = fetch_files(dir.path(), false).await.unwrap_err();
        assert!(err.contains("config.json") && err.contains("missing"), "{err}");
    }

    #[test]
    fn convert_writes_f16_and_drops_f32() {
        let dir = tempfile::tempdir().unwrap();
        let t = candle_core::Tensor::new(&[1.5f32, -2.0, 0.25], &candle_core::Device::Cpu).unwrap();
        let ids = candle_core::Tensor::new(&[1u32, 2], &candle_core::Device::Cpu).unwrap();
        let map = std::collections::HashMap::from([("w".to_string(), t), ("ids".to_string(), ids)]);
        candle_core::safetensors::save(&map, dir.path().join(WEIGHTS_F32)).unwrap();
        convert_to_f16(dir.path()).unwrap();
        assert!(!dir.path().join(WEIGHTS_F32).exists());
        let back = candle_core::safetensors::load(dir.path().join(WEIGHTS_F16), &candle_core::Device::Cpu).unwrap();
        assert_eq!(back["w"].dtype(), candle_core::DType::F16);
        assert_eq!(back["ids"].dtype(), candle_core::DType::U32, "non-f32 tensors pass through");
        let v: Vec<f32> = back["w"].to_dtype(candle_core::DType::F32).unwrap().to_vec1().unwrap();
        assert_eq!(v, vec![1.5, -2.0, 0.25]);
    }
}
