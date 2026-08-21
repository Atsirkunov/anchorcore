//! Classification orchestration — port of `backend/app/classifier.py:1`.
//! Ollama/cloud via `reqwest`, `cloud_trusted` gate (B39), rule fallback, `window_hash` helper.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::settings::SettingsService;

pub const KINDS: &[&str] = &["decision", "document", "action", "note"];
pub const DOC_TYPES: &[&str] = &["standards", "runbook", "meeting", "decision_log", "prd", "general"];
pub const DISTILL_DOC_TYPES: &[&str] = &["meeting", "decision_log", "general"];

const TYPE_DETECT_PROMPT: &str = "You identify what kind of document a piece of text is.\nPick exactly one of: standards, runbook, meeting, decision_log, prd, general.\n- standards: regulatory/normative rules, requirements, procedures\n- runbook: operational steps, escalation rules, how-to guide\n- meeting: meeting notes, retros, planning notes\n- decision_log: a log of recorded decisions\n- prd: product requirements, specifications, scoping docs\n- general: anything else\nRespond ONLY with the single word, nothing else.\n";
const DISTILL_PROMPT: &str = "You distill a conversation / chat-log excerpt into searchable Q&A units.\n\nA unit is a question someone asked AND its substantive answer. Normalize the\nexchange into a consistent format so it can be found later by either side.\n\nFor each distinct Q&A exchange in the text, output exactly one unit with:\n- question: the question, normalized to a standalone one-liner\n- answer: the substantive answer, normalized (no filler, greetings, or chit-chat)\n- terms: 1-5 searchable key terms / acronyms the unit is about\n- systems: any systems, teams, or components mentioned (empty if none)\n\nIgnore filler: greetings, small talk, acknowledgements (\"got it\", \"thanks\"),\nand messages with no information. If the text contains no real Q&A exchange,\nreturn {{\"units\": []}}.\n\nRespond ONLY with a JSON object:\n{{\"units\": [{{\"question\": str, \"answer\": str, \"terms\": [str], \"systems\": [str]}}]}}\n";

fn prompts_for(doc_type: &str) -> &'static str {
    match doc_type {
        "standards" => "You extract knowledge items from a REGULATORY / STANDARDS document.\n\nThis document defines normative rules and procedures. Extract ONLY factual\nstatements worth remembering:\n- note: a rule, requirement, definition, or procedural fact\nDo NOT extract:\n- decision (standards don’t record organizational decisions)\n- action (procedural steps are rules, not assigned to-dos)\n- document (do not emit a summary for the document itself)\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": \"note\", \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": \"\"}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
        "runbook" => "You extract knowledge items from a RUNBOOK / OPERATIONAL GUIDE.\n\nExtract ONLY factual statements worth remembering:\n- note: a step, prerequisite, escalation rule, or operational fact\nDo NOT extract:\n- decision (runbooks don’t record organizational decisions)\n- action (steps are procedural notes, not assigned to-dos)\n- document (do not emit a summary for the document itself)\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": \"note\", \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": \"\"}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
        "meeting" => "You extract knowledge items from MEETING NOTES.\n\nClassify each item into exactly one of these kinds:\n- decision: a choice made in the meeting, with reasoning\n- action: something someone agreed to do (owner + due date if stated)\n- document: a status update / factual report\n- note: anything else worth remembering\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": str, \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": str}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
        "decision_log" => "You extract knowledge items from a DECISION LOG.\n\nClassify each item into exactly one of these kinds:\n- decision: a recorded choice, with reasoning\n- note: context, constraints, or revisit triggers\n- action: a follow-up someone owns\nDo NOT extract the document itself as an item.\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": str, \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": str}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
        "prd" => "You extract knowledge items from a PRODUCT / REQUIREMENTS document.\n\nClassify each item into exactly one of these kinds:\n- decision: a scope or requirement choice, with reasoning\n- document: a factual description / requirement / spec fact\n- action: an explicitly assigned follow-up\n- note: anything else worth remembering\nDo NOT extract the document itself as an item.\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": str, \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": str}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
        _ => "You extract knowledge items from organizational text.\n\nClassify each item into exactly one of these kinds:\n- decision: a choice made, with reasoning behind it\n- document: a factual reference / status update / description\n- action: something to be done, assigned, or tracked\n- note: anything else worth remembering\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": str, \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": str}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedItem {
    pub kind: String,
    pub summary: String,
    pub reasoning: String,
    pub confidence: f64,
    pub author: String,
    pub source_ref: String,
    #[serde(default)]
    pub window_text: String,
    #[serde(default)]
    pub window_index: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistilledUnit {
    pub question: String,
    pub answer: String,
    pub terms: Vec<String>,
    pub systems: Vec<String>,
}

pub struct Classifier {
    pub settings: Arc<SettingsService>,
}

impl Classifier {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings }
    }

    fn base_url(&self) -> String {
        self.settings
            .get("classifier_base_url", None)
            .or_else(|| self.settings.get("ollama_base_url", None))
            .unwrap_or_else(|| "http://localhost:11434".to_string())
    }

    fn provider_is_local(&self) -> bool {
        let u = self.base_url();
        u.starts_with("http://localhost") || u.starts_with("http://127.0.0.1")
    }

    pub async fn detect_document_type(&self, text: &str, cloud_trusted: bool) -> String {
        if !cloud_trusted && !self.provider_is_local() {
            tracing::info!("document-type gate: remote provider unconfirmed; using rules");
            return "general".to_string();
        }
        match self.detect_llm(text).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("document-type detection failed ({}); defaulting to general", e);
                "general".to_string()
            }
        }
    }

    pub async fn classify(
        &self,
        text: &str,
        source_ref: &str,
        doc_type: &str,
        cloud_trusted: bool,
    ) -> Vec<ClassifiedItem> {
        if !cloud_trusted && !self.provider_is_local() {
            tracing::info!("classification gate: remote provider unconfirmed; using rules");
            return classify_rules(text, source_ref);
        }
        match self.classify_llm(text, source_ref, doc_type).await {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!("LLM classification failed ({}); falling back to rules", e);
                classify_rules(text, source_ref)
            }
        }
    }

    pub async fn distill(&self, text: &str, source_ref: &str, cloud_trusted: bool) -> Vec<DistilledUnit> {
        if !cloud_trusted && !self.provider_is_local() {
            tracing::info!("distillation gate: remote provider unconfirmed; using rules");
            return distill_rules(text);
        }
        match self.distill_llm(text, source_ref).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("LLM distillation failed ({}); falling back to rules", e);
                distill_rules(text)
            }
        }
    }

    async fn detect_llm(&self, text: &str) -> Result<String, String> {
        let truncated = text.chars().take(8000).collect::<String>();
        let payload = serde_json::json!({
            "model": self.settings.get("classifier_model", None).unwrap_or_else(|| "llama3.2:3b".to_string()),
            "messages": [
                {"role": "system", "content": TYPE_DETECT_PROMPT},
                {"role": "user", "content": format!("Document text (beginning):\n{}", truncated)}
            ],
            "temperature": 0.0,
            "max_tokens": 10
        });
        let base = self.base_url();
        let api_key = self.settings.get("classifier_api_key", None).unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.settings.get_float("classifier_timeout", 60.0) as u64,
            ))
            .build()
            .map_err(|e| e.to_string())?;
        let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
        let mut req = client.post(&url).json(&payload);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status()));
        }
        let j: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let raw = j.pointer("/choices/0/message/content").and_then(|v| v.as_str()).unwrap_or("general").trim().to_lowercase();
        for dt in DOC_TYPES {
            if raw.contains(dt) {
                return Ok(dt.to_string());
            }
        }
        Ok("general".to_string())
    }

    async fn classify_llm(&self, text: &str, source_ref: &str, doc_type: &str) -> Result<Vec<ClassifiedItem>, String> {
        let prompt = prompts_for(doc_type);
        let payload = serde_json::json!({
            "model": self.settings.get("classifier_model", None).unwrap_or_else(|| "llama3.2:3b".to_string()),
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": format!("Source: {}\n\nContent:\n{}", source_ref, text)}
            ],
            "temperature": 0.1,
            "response_format": {"type": "json_object"}
        });
        let base = self.base_url();
        let api_key = self.settings.get("classifier_api_key", None).unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.settings.get_float("classifier_timeout", 60.0) as u64))
            .build().map_err(|e| e.to_string())?;
        let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
        let mut req = client.post(&url).json(&payload);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status()));
        }
        let j: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let raw = j.pointer("/choices/0/message/content").and_then(|v| v.as_str()).ok_or("missing content")?;
        let data: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let items = data.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        Ok(normalize_items(items, source_ref))
    }

    async fn distill_llm(&self, text: &str, source_ref: &str) -> Result<Vec<DistilledUnit>, String> {
        let payload = serde_json::json!({
            "model": self.settings.get("classifier_model", None).unwrap_or_else(|| "llama3.2:3b".to_string()),
            "messages": [
                {"role": "system", "content": DISTILL_PROMPT},
                {"role": "user", "content": format!("Source: {}\n\nContent:\n{}", source_ref, text)}
            ],
            "temperature": 0.0,
            "response_format": {"type": "json_object"}
        });
        let base = self.base_url();
        let api_key = self.settings.get("classifier_api_key", None).unwrap_or_default();
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().map_err(|e| e.to_string())?;
        let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
        let mut req = client.post(&url).json(&payload);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status()));
        }
        let j: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let raw = j.pointer("/choices/0/message/content").and_then(|v| v.as_str()).ok_or("missing content")?;
        let data: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let units = data.get("units").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for u in units {
            if let Some(obj) = u.as_object() {
                let q = obj.get("question").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let a = obj.get("answer").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                if q.is_empty() || a.is_empty() { continue; }
                let terms = obj.get("terms").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(5).collect()).unwrap_or_default();
                let systems = obj.get("systems").and_then(|v| v.as_array()).map(|arr| arr.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(5).collect()).unwrap_or_default();
                out.push(DistilledUnit { question: q.chars().take(500).collect(), answer: a.chars().take(1200).collect(), terms, systems });
            }
        }
        Ok(out)
    }
}

fn normalize_items(items: Vec<serde_json::Value>, source_ref: &str) -> Vec<ClassifiedItem> {
    let mut out = Vec::new();
    for item in items {
        if let Some(obj) = item.as_object() {
            let kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or("note").to_lowercase();
            let kind = if KINDS.contains(&kind.as_str()) { kind } else { "note".to_string() };
            let summary = obj.get("summary").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(1000).collect::<String>();
            if summary.is_empty() { continue; }
            let reasoning = obj.get("reasoning").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(1000).collect();
            let confidence = obj.get("confidence").and_then(|v| v.as_f64()).map(|f| f.clamp(0.0,1.0)).unwrap_or(0.5);
            let author = obj.get("author").and_then(|v| v.as_str()).unwrap_or("").trim().chars().take(300).collect();
            out.push(ClassifiedItem { kind, summary, reasoning, confidence, author, source_ref: source_ref.to_string(), window_text: String::new(), window_index: None });
        }
    }
    out
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0usize;
    for i in 0..chars.len() {
        if ".!?".contains(chars[i]) {
            let next_is_ws = i + 1 >= chars.len() || chars[i + 1].is_whitespace();
            if next_is_ws {
                let s: String = chars[start..=i].iter().collect();
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(trimmed);
                }
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                start = j;
            }
        }
    }
    if start < chars.len() {
        let s: String = chars[start..].iter().collect();
        let trimmed = s.trim().to_string();
        if !trimmed.is_empty() {
            out.push(trimmed);
        }
    }
    out
}

pub fn classify_rules(text: &str, source_ref: &str) -> Vec<ClassifiedItem> {
    let mut items = Vec::new();
    for sentence in split_sentences(text) {
        let s = sentence.trim();
        if s.is_empty() { continue; }
        if let Some(kind) = rule_kind(s) {
            items.push(ClassifiedItem {
                kind,
                summary: s.chars().take(240).collect(),
                reasoning: "rule-based classifier".to_string(),
                confidence: 0.5,
                author: String::new(),
                source_ref: source_ref.to_string(),
                window_text: String::new(),
                window_index: None,
            });
        }
    }
    normalize_items(items.into_iter().map(|i| serde_json::json!({"kind": i.kind, "summary": i.summary, "reasoning": i.reasoning, "confidence": i.confidence, "author": i.author})).collect(), source_ref)
}

fn rule_kind(sentence: &str) -> Option<String> {
    static RE_DECISION: OnceLock<Regex> = OnceLock::new();
    static RE_ACTION: OnceLock<Regex> = OnceLock::new();
    static RE_DOCUMENT: OnceLock<Regex> = OnceLock::new();
    let lower = sentence.to_lowercase();
    if RE_DECISION.get_or_init(|| Regex::new(r"\b(we (should|will|must|need to|decided)|decision|decided to|approved|rejected|postpone)\b").unwrap()).is_match(&lower) {
        return Some("decision".to_string());
    }
    if RE_ACTION.get_or_init(|| Regex::new(r"\b(action item|todo|to-do|next step|owner|assign|follow[- ]up|deadline|due)\b").unwrap()).is_match(&lower) {
        return Some("action".to_string());
    }
    if RE_DOCUMENT.get_or_init(|| Regex::new(r"\b(status|update|summary|as of|current state|is now|has been)\b").unwrap()).is_match(&lower) {
        return Some("document".to_string());
    }
    None
}

pub fn distill_rules(text: &str) -> Vec<DistilledUnit> {
    static RE_FILLER: OnceLock<Regex> = OnceLock::new();
    static RE_TERMS: OnceLock<Regex> = OnceLock::new();
    let sentences: Vec<String> = split_sentences(text);
    let mut units = Vec::new();
    for i in 0..sentences.len() {
        let sentence = &sentences[i];
        if !sentence.ends_with('?') || i+1 >= sentences.len() { continue; }
        let answer = &sentences[i+1];
        if answer.len() < 5 || RE_FILLER.get_or_init(|| Regex::new(r"(?i)^(got it|ok|thanks|thx|yes|no|sure)[.!]*$").unwrap()).is_match(answer) {
            continue;
        }
        let terms: Vec<String> = RE_TERMS.get_or_init(|| Regex::new(r"[A-Za-z][A-Za-z0-9_\-]{2,}").unwrap()).find_iter(&format!("{} {}", sentence, answer)).map(|m| m.as_str().to_lowercase()).take(5).collect();
        units.push(DistilledUnit { question: sentence.chars().take(500).collect(), answer: answer.chars().take(1200).collect(), terms, systems: vec![] });
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;
    #[test]
    fn rule_kind_decision() {
        assert_eq!(rule_kind("We decided to launch next month."), Some("decision".to_string()));
        assert_eq!(rule_kind("No match here."), None);
    }
    #[test]
    fn classify_rules_fallback() {
        let items = classify_rules("We decided to approve the budget. Status update as of now.", "ref");
        assert!(items.len() >= 2);
    }
    #[test]
    fn distill_rules_pairs() {
        let text = "What is the deadline? The deadline is next Friday. Got it. Another question? Answer here.";
        let units = distill_rules(text);
        assert!(!units.is_empty());
        assert_eq!(units[0].question, "What is the deadline?");
    }
    #[test]
    fn provider_local_gate() {
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = Arc::new(crate::settings::SettingsService::new(store));
        // default ollama is local -> trusted
        let c = Classifier::new(svc);
        assert!(c.provider_is_local());
    }
}
