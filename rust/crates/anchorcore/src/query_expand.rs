//! R45.13: query-time acronym expansion by the local answer model — the
//! replacement for index-time Tier-2 guessing. Local mode only (agent mode
//! broadens its own queries), one budgeted call per new query, cached in
//! memory, nothing stored. Acronyms the vocabulary already knows skip the
//! call; opt-in via `acronym_expansion=true` (owner decision 2026-10-21).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::settings::SettingsService;

/// Per-query acronyms considered, expansions kept, call budget, cache size.
const MAX_ACRONYMS: usize = 4;
const MAX_TIMEOUT: Duration = Duration::from_secs(10);
const CACHE_CAP: usize = 512;

/// All-caps tokens that are words, not acronyms.
const NOT_ACRONYMS: &[&str] = &["OK", "AM", "PM", "ID", "TODO", "FYI", "ASAP", "AND", "OR", "NOT", "THE"];

const SYSTEM_PROMPT: &str = "You expand acronyms in a search query. Reply with JSON only: an object mapping each acronym you are confident about to its spelled-out form, e.g. {\"KYC\": \"know your customer\"}. Use {} when unsure. No other text.";

fn cache() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 2–7 char all-caps tokens (digits allowed after the first letter),
/// deduped in order, capped.
pub fn query_acronyms(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in question.split(|c: char| !c.is_alphanumeric()) {
        let shaped = (2..=7).contains(&raw.len())
            && raw.starts_with(|c: char| c.is_ascii_uppercase())
            && raw.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if shaped && !NOT_ACRONYMS.contains(&raw) && !out.iter().any(|a| a == raw) {
            out.push(raw.to_string());
        }
        if out.len() >= MAX_ACRONYMS {
            break;
        }
    }
    out
}

/// Opt-in: on only when `acronym_expansion` is explicitly true; never in agent mode.
pub fn enabled(settings: &SettingsService) -> bool {
    let on = settings
        .get("acronym_expansion", None)
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "on" | "yes"));
    on && !crate::settings::is_agent_mode(settings)
}

/// Expansions for `acronyms` (already filtered to unknown ones). Cached per
/// (model, acronym set); a failed or empty call caches as empty too, so a
/// flaky model costs one call per query, not one per retry.
pub async fn expand(settings: &SettingsService, acronyms: &[String], timeout: Duration) -> Vec<String> {
    if acronyms.is_empty() {
        return Vec::new();
    }
    let model = settings.get("answer_model", None).unwrap_or_default();
    let key = format!("{model}|{}", acronyms.join(","));
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let raw = call_model(settings, acronyms, timeout.min(MAX_TIMEOUT)).await.unwrap_or_default();
    let expansions = parse_expansions(&raw, acronyms);
    if let Ok(mut c) = cache().lock() {
        if c.len() >= CACHE_CAP {
            c.clear();
        }
        c.insert(key, expansions.clone());
    }
    expansions
}

async fn call_model(settings: &SettingsService, acronyms: &[String], timeout: Duration) -> Option<String> {
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    if !base.starts_with("http") {
        return None;
    }
    let api_key = settings.get("answer_api_key", None).unwrap_or_default();
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let ollama_base = settings.get("ollama_base_url", None);
    // R30.6: a local Ollama gets native `/api/chat` with `num_ctx` pinned.
    let spec = crate::llm_chat::ChatSpec {
        base: &base,
        api_key: &api_key,
        model: &model,
        messages: serde_json::json!([
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": format!("Acronyms: {}", acronyms.join(", "))}
        ]),
        temperature: Some(0.0),
        max_tokens: Some(120),
        timeout,
        ollama_base: ollama_base.as_deref(),
    };
    crate::llm_chat::chat(&spec).await.ok()
}

/// Lenient parse: the first `{…}` object; only asked-for acronyms, only
/// pairs that normalize (same caps as stored vocabulary).
fn parse_expansions(raw: &str, acronyms: &[String]) -> Vec<String> {
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else { return Vec::new() };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw.get(start..=end).unwrap_or("")) else {
        return Vec::new();
    };
    let wanted: HashSet<String> = acronyms.iter().map(|a| a.to_lowercase()).collect();
    let mut out: Vec<String> = Vec::new();
    for (k, v) in map {
        let Some(pair) = v.as_str().and_then(|e| crate::vocab::normalize_pair(&k, e)) else { continue };
        if wanted.contains(&pair.term) && !out.contains(&pair.expansion) {
            out.push(pair.expansion);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acronyms_are_caps_tokens_minus_words() {
        assert_eq!(query_acronyms("what is our KYC and P2P policy, OK? (see SLA)"), ["KYC", "P2P", "SLA"]);
        assert!(query_acronyms("plain lowercase question about Billing").is_empty());
        assert_eq!(query_acronyms("A B CA CA").len(), 1, "single letters skipped, dupes once");
    }

    #[test]
    fn parse_keeps_only_asked_valid_pairs() {
        let asked = vec!["KYC".to_string(), "SLA".to_string()];
        let raw = "Sure! {\"KYC\": \"Know Your Customer\", \"XYZ\": \"not asked\", \"SLA\": \"SLA\"} done";
        assert_eq!(parse_expansions(raw, &asked), ["know your customer"]);
        assert!(parse_expansions("no json here", &asked).is_empty());
    }
}
