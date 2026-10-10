//! B54 — dummy-proof local model setup: readiness probe + pull progress.
//!
//! The wizard and System tab drive model installs through here instead of
//! copy-pasted terminal commands. Pull progress lives in a process-global map
//! (same OnceLock pattern as the auth rate limits) so no AppState change is
//! needed and in-flight pulls survive handler boundaries.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crate::health::AppState;
use crate::settings::SettingsService;

// ----- model-name matching (same family-only rule as settings::test_ollama) -----

/// "llama3.2:3b" -> "llama3.2" — a configured model counts as installed when
/// any tag of the same family is present in Ollama.
pub(crate) fn normalize_model(name: &str) -> String {
    name.split(':').next().unwrap_or(name).trim().to_string()
}

pub(crate) fn installed_families(tags_body: &Value) -> HashSet<String> {
    tags_body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .map(normalize_model)
                .collect()
        })
        .unwrap_or_default()
}

/// Full configured names whose family is absent from `installed`.
pub(crate) fn missing_from_installed(
    configured: &[String],
    installed: &HashSet<String>,
) -> Vec<String> {
    configured
        .iter()
        .filter(|m| !installed.contains(&normalize_model(m)))
        .cloned()
        .collect()
}

// ----- readiness -----

fn is_local_base(base: &str) -> bool {
    base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1")
}

fn ollama_base(settings: &SettingsService) -> String {
    settings
        .get("ollama_base_url", None)
        .unwrap_or_else(|| "http://localhost:11434".to_string())
}

/// Base URL for a role, falling back to the shared Ollama base; None when cloud.
fn local_base_for(settings: &SettingsService, base_key: &str) -> Option<String> {
    let base = settings
        .get(base_key, None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    if is_local_base(&base) {
        Some(base)
    } else {
        None
    }
}

/// Configured models that must come from the local Ollama server, as
/// (role, model) pairs. Roles are display labels for the install UI.
pub(crate) fn configured_local_models(settings: &SettingsService) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if local_base_for(settings, "classifier_base_url").is_some() {
        let m = settings
            .get("classifier_model", None)
            .unwrap_or_else(|| "llama3.2:3b".to_string());
        out.push(("classifier".to_string(), m));
    }
    if local_base_for(settings, "embed_base_url").is_some() {
        let m = settings
            .get("embed_model", None)
            .unwrap_or_else(|| "nomic-embed-text".to_string());
        out.push(("embeddings".to_string(), m));
    }
    if local_base_for(settings, "answer_base_url").is_some() {
        let m = settings
            .get("answer_model", None)
            .unwrap_or_else(|| "llama3.2:3b".to_string());
        out.push(("answers".to_string(), m));
    }
    out
}

pub(crate) async fn fetch_ollama_tags(base: &str) -> Option<Value> {
    if crate::system::ollama_probe_disabled(base) {
        return None;
    }
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelReadiness {
    pub reachable: bool,
    pub missing_models: Vec<String>,
    pub answer_ready: bool,
}

pub(crate) async fn model_readiness(settings: &SettingsService) -> ModelReadiness {
    let base = ollama_base(settings);
    let configured = configured_local_models(settings);
    let Some(tags) = fetch_ollama_tags(&base).await else {
        return ModelReadiness {
            reachable: false,
            missing_models: vec![],
            answer_ready: cloud_answer_configured(settings),
        };
    };
    let installed = installed_families(&tags);
    let models: Vec<String> = configured.iter().map(|(_, m)| m.clone()).collect();
    let missing = missing_from_installed(&models, &installed);
    let missing = dedupe_families(missing);
    let missing_set: HashSet<String> = missing.iter().map(|m| normalize_model(m)).collect();
    let answer_model_missing = configured
        .iter()
        .find(|(role, _)| role == "answers")
        .map(|(_, m)| missing_set.contains(&normalize_model(m)))
        .unwrap_or(false);
    let answer_local = local_base_for(settings, "answer_base_url").is_some();
    ModelReadiness {
        reachable: true,
        missing_models: missing,
        answer_ready: cloud_answer_configured(settings) || (answer_local && !answer_model_missing),
    }
}

/// Keep the first configured name per family (classifier + answers often share one).
fn dedupe_families(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter(|m| seen.insert(normalize_model(m)))
        .collect()
}

fn cloud_answer_configured(settings: &SettingsService) -> bool {
    let base = settings
        .get("answer_base_url", None)
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    !is_local_base(&base) && !key.trim().is_empty()
}

// ----- pull progress (process-global; see module docs) -----

#[derive(Debug, Clone, Serialize)]
struct PullProgress {
    model: String,
    status: String,
    completed: u64,
    total: u64,
    done: bool,
    error: Option<String>,
}

static PULLS: OnceLock<Mutex<HashMap<String, PullProgress>>> = OnceLock::new();

fn pull_map() -> &'static Mutex<HashMap<String, PullProgress>> {
    PULLS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_pull(
    model: &str,
    status: &str,
    completed: u64,
    total: u64,
    done: bool,
    error: Option<String>,
) {
    let mut map = pull_map().lock().unwrap();
    map.insert(
        model.to_string(),
        PullProgress {
            model: model.to_string(),
            status: status.to_string(),
            completed,
            total,
            done,
            error,
        },
    );
}

// ----- pull validation + NDJSON parsing (pure, unit-tested) -----

const MAX_PULL_MODELS: usize = 5;

pub(crate) fn validate_pull_models(models: &[String]) -> Result<Vec<String>, String> {
    if models.is_empty() {
        return Err("no models requested".to_string());
    }
    if models.len() > MAX_PULL_MODELS {
        return Err(format!("at most {} models per pull", MAX_PULL_MODELS));
    }
    let mut clean = Vec::with_capacity(models.len());
    for m in models {
        let name = m.trim();
        if name.is_empty() || name.len() > 128 {
            return Err("invalid model name".to_string());
        }
        if name.contains("..") {
            return Err(format!("invalid model name: {}", name));
        }
        let ok = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c));
        if !ok {
            return Err(format!("invalid model name: {}", name));
        }
        if !clean.contains(&name.to_string()) {
            clean.push(name.to_string());
        }
    }
    Ok(clean)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PullEvent {
    pub status: String,
    pub completed: u64,
    pub total: u64,
}

fn parse_pull_line(line: &[u8]) -> Option<PullEvent> {
    let v: Value = serde_json::from_slice(line).ok()?;
    Some(PullEvent {
        status: v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        completed: v.get("completed").and_then(|n| n.as_u64()).unwrap_or(0),
        total: v.get("total").and_then(|n| n.as_u64()).unwrap_or(0),
    })
}

/// Drain complete `\n`-terminated JSON lines from `buffer` after pushing `chunk`.
/// A trailing partial line stays buffered for the next chunk.
pub(crate) fn feed_pull_chunk(buffer: &mut Vec<u8>, chunk: &[u8]) -> Vec<PullEvent> {
    buffer.extend_from_slice(chunk);
    let mut events = Vec::new();
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        if let Some(ev) = parse_pull_line(&line) {
            events.push(ev);
        }
    }
    events
}

/// Parse a trailing line left without `\n` at stream end, if it is valid JSON.
pub(crate) fn flush_pull_buffer(buffer: &mut Vec<u8>) -> Option<PullEvent> {
    if buffer
        .iter()
        .all(|&b| b == b'\n' || b == b'\r' || b == b' ' || b == b'\t')
    {
        buffer.clear();
        return None;
    }
    let line: Vec<u8> = std::mem::take(buffer);
    parse_pull_line(&line)
}

fn merge_pull_event(last: &mut PullEvent, ev: PullEvent) {
    if !ev.status.is_empty() {
        last.status = ev.status;
    }
    if ev.total > 0 {
        last.total = ev.total;
    }
    if ev.completed > 0 {
        last.completed = ev.completed;
    }
}

// ----- pull worker -----

async fn drain_pull_stream(model: &str, resp: &mut reqwest::Response) -> Result<PullEvent, String> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut last = PullEvent::default();
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                for ev in feed_pull_chunk(&mut buffer, &bytes) {
                    merge_pull_event(&mut last, ev);
                    set_pull(model, &last.status, last.completed, last.total, false, None);
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("pull interrupted: {}", e)),
        }
    }
    if let Some(ev) = flush_pull_buffer(&mut buffer) {
        merge_pull_event(&mut last, ev);
    }
    Ok(last)
}

async fn pull_one_model(base: &str, model: &str) {
    set_pull(model, "starting", 0, 0, false, None);
    let url = format!("{}/api/pull", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            set_pull(model, "error", 0, 0, true, Some(e.to_string()));
            return;
        }
    };
    let mut resp = match client
        .post(&url)
        .json(&json!({"name": model, "stream": true}))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            set_pull(
                model,
                "error",
                0,
                0,
                true,
                Some(format!("pull request failed: {}", e)),
            );
            return;
        }
    };
    if !resp.status().is_success() {
        let detail = resp.text().await.unwrap_or_default();
        let first = detail
            .lines()
            .next()
            .unwrap_or("pull rejected")
            .chars()
            .take(300)
            .collect::<String>();
        set_pull(model, "error", 0, 0, true, Some(first));
        return;
    }
    match drain_pull_stream(model, &mut resp).await {
        Ok(last) if last.status == "success" => {
            set_pull(model, "success", last.completed, last.total, true, None);
        }
        Ok(last) => {
            let msg = if last.status.is_empty() {
                "pull ended with no status".to_string()
            } else {
                format!("pull ended: {}", last.status)
            };
            set_pull(model, "error", last.completed, last.total, true, Some(msg));
        }
        Err(msg) => {
            set_pull(model, "error", 0, 0, true, Some(msg));
        }
    }
}

async fn run_pulls(base: String, models: Vec<String>) {
    for m in &models {
        pull_one_model(&base, m).await;
    }
}

// ----- handlers -----

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub models: Vec<String>,
}

pub async fn pull_handler(
    State(state): State<AppState>,
    Json(payload): Json<PullRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let models = match validate_pull_models(&payload.models) {
        Ok(m) => m,
        Err(e) => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": e})))),
    };
    let base = ollama_base(&state.settings);
    if !is_local_base(&base) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "refusing to pull through a non-local Ollama URL"})),
        ));
    }
    if fetch_ollama_tags(&base).await.is_none() {
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({"detail": "Ollama is not reachable — start it first"})),
        ));
    }
    for m in &models {
        set_pull(m, "queued", 0, 0, false, None);
    }
    tokio::spawn(run_pulls(base, models.clone()));
    Ok(Json(json!({"started": models})))
}

pub async fn pulls_handler() -> Json<Value> {
    let map = pull_map().lock().unwrap();
    let mut pulls: Vec<PullProgress> = map.values().cloned().collect();
    pulls.sort_by(|a, b| a.model.cmp(&b.model));
    Json(json!({"pulls": pulls}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_tag() {
        assert_eq!(normalize_model("llama3.2:3b"), "llama3.2");
        assert_eq!(normalize_model("nomic-embed-text"), "nomic-embed-text");
        assert_eq!(normalize_model("  qwen3:8b  "), "qwen3");
    }

    #[test]
    fn installed_families_parses_tags() {
        let body = serde_json::json!({"models": [{"name": "llama3.2:3b"}, {"name": "nomic-embed-text:latest"}]});
        let set = installed_families(&body);
        assert!(set.contains("llama3.2"));
        assert!(set.contains("nomic-embed-text"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn installed_families_empty_on_garbage() {
        assert!(installed_families(&serde_json::json!({})).is_empty());
        assert!(installed_families(&serde_json::json!({"models": "x"})).is_empty());
        assert!(installed_families(&serde_json::json!({"models": [{"nope": 1}]})).is_empty());
    }

    #[test]
    fn missing_keeps_full_configured_names() {
        let installed: HashSet<String> = ["llama3.2".to_string()].into_iter().collect();
        let got = missing_from_installed(
            &["llama3.2:3b".to_string(), "nomic-embed-text".to_string()],
            &installed,
        );
        assert_eq!(got, vec!["nomic-embed-text".to_string()]);
    }

    #[test]
    fn missing_matches_any_tag_of_family() {
        let installed: HashSet<String> = ["llama3.2".to_string()].into_iter().collect();
        let got = missing_from_installed(&["llama3.2:latest".to_string()], &installed);
        assert!(got.is_empty());
    }

    #[test]
    fn dedupe_keeps_first_name_per_family() {
        let got = dedupe_families(vec![
            "llama3.2:3b".to_string(),
            "llama3.2:latest".to_string(),
        ]);
        assert_eq!(got, vec!["llama3.2:3b".to_string()]);
    }

    #[test]
    fn validate_ok_and_dedupes() {
        let got = validate_pull_models(&[
            "llama3.2:3b".to_string(),
            " llama3.2:3b ".to_string(),
            "nomic-embed-text".to_string(),
        ])
        .unwrap();
        assert_eq!(
            got,
            vec!["llama3.2:3b".to_string(), "nomic-embed-text".to_string()]
        );
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(validate_pull_models(&[]).is_err());
        assert!(validate_pull_models(&vec!["a".to_string(); 6]).is_err());
        assert!(validate_pull_models(&["  ".to_string()]).is_err());
        assert!(validate_pull_models(&["../evil".to_string()]).is_err());
        assert!(validate_pull_models(&["has space".to_string()]).is_err());
        assert!(validate_pull_models(&["x".repeat(129)]).is_err());
    }

    #[test]
    fn feed_splits_lines_and_buffers_partial() {
        let mut buf = Vec::new();
        let evs = feed_pull_chunk(
            &mut buf,
            b"{\"status\":\"pulling\",\"completed\":1,\"total\":9}\n{\"status\":\"pull",
        );
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].completed, 1);
        assert_eq!(evs[0].total, 9);
        let evs2 = feed_pull_chunk(&mut buf, b"ing\",\"completed\":2}\n");
        assert_eq!(evs2.len(), 1);
        assert_eq!(evs2[0].status, "pulling");
        assert_eq!(evs2[0].completed, 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn feed_ignores_bad_json_lines() {
        let mut buf = Vec::new();
        let evs = feed_pull_chunk(&mut buf, b"not json\n{\"status\":\"ok\"}\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].status, "ok");
    }

    #[test]
    fn flush_parses_trailing_line() {
        let mut buf = b"{\"status\":\"success\"}".to_vec();
        let ev = flush_pull_buffer(&mut buf).unwrap();
        assert_eq!(ev.status, "success");
        assert!(buf.is_empty());
        let mut blank = b" \n".to_vec();
        assert!(flush_pull_buffer(&mut blank).is_none());
    }

    #[test]
    fn merge_keeps_best_known_values() {
        let mut last = PullEvent::default();
        merge_pull_event(
            &mut last,
            PullEvent {
                status: "pulling".to_string(),
                completed: 5,
                total: 9,
            },
        );
        merge_pull_event(
            &mut last,
            PullEvent {
                status: "".to_string(),
                completed: 0,
                total: 0,
            },
        );
        assert_eq!(last.status, "pulling");
        assert_eq!(last.completed, 5);
        assert_eq!(last.total, 9);
    }
}
