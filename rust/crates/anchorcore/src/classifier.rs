//! Classification orchestration — port of `backend/app/classifier.py:1`.
//! Ollama/cloud via `reqwest`, `cloud_trusted` gate (B39), rule fallback.
//!
//! R33.6: prose collapses to two deterministic buckets — `record`
//! (meeting/decision content, the only bucket that may emit decision/action)
//! vs `reference` (everything else, notes only). The 8-way LLM type-detect
//! and the per-type prompt matrix are deleted: the old detect collapsed
//! 20/20 corpus files to `documentation` (see `docs/audit-r33.6-kinds.md`).
//! `code` still skips extraction entirely (R33.2). Legacy 8-way `doc_type`
//! values and legacy `document`-kind rows pass through read-only.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

use crate::settings::SettingsService;

/// Entity kinds new extraction may emit. `document` is retired (R33.6):
/// status updates and descriptions are notes. Legacy `document`-kind rows
/// pass through read-only — nothing re-normalizes stored rows.
pub const KINDS: &[&str] = &["decision", "action", "note"];
/// Prose buckets. `code` is not a bucket — it skips extraction (R33.2).
pub const BUCKETS: &[&str] = &["record", "reference"];

// R33.6: one strict prompt per bucket. The record prompt may emit
// decision/action but demands an explicitly recorded choice — a bare
// approval pointer ("Approved in decisions/d2") with no choice or
// reasoning is not a decision.
const RECORD_PROMPT: &str = "You extract knowledge items from a MEETING or DECISION RECORD.\n\nClassify each item into exactly one of these kinds:\n- decision: ONLY a choice explicitly recorded as made (we decided / we agreed / chose / approved), with its reasoning. A bare approval pointer (\"Approved in decisions/d2\") with no recorded choice or reasoning is NOT a decision — skip it.\n- action: something explicitly assigned to a person, or a tracked follow-up with an owner\n- note: a fact, status update, constraint, revisit trigger, or anything else worth remembering\nDo NOT emit kind \"document\" — status updates and descriptions are notes.\nDo NOT extract the document itself as an item.\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": str, \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": str}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n";
const REFERENCE_PROMPT: &str = "You extract knowledge items from a REFERENCE document (spec, runbook, policy, people, docs).\n\nThis document describes how things work — it does not record organizational decisions. Extract ONLY factual statements worth remembering:\n- note: a rule, requirement, step, definition, responsibility, or operational fact\nDo NOT extract:\n- decision (reference docs describe; they do not decide)\n- action (procedural steps are notes, not assigned to-dos)\n- document (status updates and descriptions are notes)\n\nRespond ONLY with a JSON object:\n{{\"items\": [{{\"kind\": \"note\", \"summary\": str, \"reasoning\": str, \"confidence\": float, \"author\": \"\"}}]}}\nKeep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{\"items\": []}}.\n";
const DISTILL_PROMPT: &str = "You distill a conversation / chat-log excerpt into searchable Q&A units.\n\nA unit is a question someone asked AND its substantive answer. Normalize the\nexchange into a consistent format so it can be found later by either side.\n\nFor each distinct Q&A exchange in the text, output exactly one unit with:\n- question: the question, normalized to a standalone one-liner\n- answer: the substantive answer, normalized (no filler, greetings, or chit-chat)\n- terms: 1-5 searchable key terms / acronyms the unit is about\n- systems: any systems, teams, or components mentioned (empty if none)\n\nIgnore filler: greetings, small talk, acknowledgements (\"got it\", \"thanks\"),\nand messages with no information. If the text contains no real Q&A exchange,\nreturn {{\"units\": []}}.\n\nRespond ONLY with a JSON object:\n{{\"units\": [{{\"question\": str, \"answer\": str, \"terms\": [str], \"systems\": [str]}}]}}\n";

/// R33.6 record markers (owner Q1(a) broadened set): the old detect_rules
/// markers plus agreed/decided/approved, `**Status:**` verdict lines,
/// `# Decision` titles, and revisit triggers — all word-boundaried, run
/// over the lowercased 12k head. `decision logging` ≠ `decision log`.
const RECORD_PATTERNS: &[&str] = &[
    r"\battendees\b",
    r"\bagenda\b",
    r"\bmeeting notes\b",
    r"\bminutes of\b",
    r"\bretro\b",
    r"\bdecision log\b",
    r"\bdecision:",
    r"\badr-",
    r"\bstatus:\s*accepted\b",
    r"\bagreed\b",
    r"\bdecided\b",
    r"\bapproved\b",
    r"\*\*status:\*\*\s*(confirmed|accepted|approved)",
    r"(?m)^#{1,6}\s+decision\b",
    r"\brevisit triggers?\b",
];

/// R33.6 deterministic bucket assignment: markers → `record`, everything
/// else → `reference`. No LLM, no trust gate — a pure function of the text.
/// `code` is assigned by the pipeline's extension fast path, never here.
pub fn assign_bucket(text: &str) -> String {
    static RECORD_RES: OnceLock<Vec<Regex>> = OnceLock::new();
    let res = RECORD_RES.get_or_init(|| RECORD_PATTERNS.iter().map(|p| Regex::new(p).unwrap()).collect());
    let head: String = text.chars().take(12000).collect::<String>().to_lowercase();
    if res.iter().any(|re| re.is_match(&head)) {
        "record".to_string()
    } else {
        "reference".to_string()
    }
}

/// Only the `record` bucket may emit decision/action. `reference`, `code`,
/// and every legacy 8-way value are notes-only-or-nothing: no
/// grandfathering — force reclassify re-buckets deterministically.
pub fn is_record_bucket(doc_type: &str) -> bool {
    doc_type == "record"
}

/// R33.6: distill gating folded into the bucket definition (replaces both
/// DISTILL_DOC_TYPES copies). Conversation-like records distill; reference
/// docs never did as a class (only the old `general` catch-all did) and
/// don't start now.
pub fn bucket_distills(doc_type: &str) -> bool {
    doc_type == "record"
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
        crate::common::provider_is_local(&self.base_url())
    }

    /// R33.6: `doc_type` is the deterministic bucket (`record`/`reference`,
    /// `code`) or a legacy 8-way value (treated as notes-only — see
    /// `is_record_bucket`). Newly extracted prose always stores the bucket.
    pub async fn classify(
        &self,
        text: &str,
        source_ref: &str,
        doc_type: &str,
        cloud_trusted: bool,
    ) -> Vec<ClassifiedItem> {
        // R33.2: code is code — no entity extraction from source files. The
        // code itself is the retrievable unit; paraphrases only added ranking
        // noise and review-queue filler. No LLM call is attempted either.
        if doc_type == "code" {
            return Vec::new();
        }
        if !cloud_trusted && !self.provider_is_local() {
            tracing::info!("classification gate: remote provider unconfirmed; using rules");
            return classify_rules_typed(text, source_ref, doc_type);
        }
        match self.classify_llm(text, source_ref, doc_type).await {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!("LLM classification failed ({}); falling back to rules", e);
                classify_rules_typed(text, source_ref, doc_type)
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

    async fn classify_llm(&self, text: &str, source_ref: &str, doc_type: &str) -> Result<Vec<ClassifiedItem>, String> {
        let prompt = if is_record_bucket(doc_type) { RECORD_PROMPT } else { REFERENCE_PROMPT };
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
        Ok(normalize_items(items, source_ref, doc_type))
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

fn normalize_items(items: Vec<serde_json::Value>, source_ref: &str, bucket: &str) -> Vec<ClassifiedItem> {
    let mut out = Vec::new();
    for item in items {
        if let Some(obj) = item.as_object() {
            let raw_kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or("note").to_lowercase();
            // R33.6: new extraction emits decision/action/note only — a stray
            // "document" (legacy kind, disobedient model) degrades to note.
            let mut kind = if KINDS.contains(&raw_kind.as_str()) { raw_kind } else { "note".to_string() };
            // R33.6: reference (and any legacy/non-record value) is
            // notes-only, enforced here — the prompt asks, this guarantees.
            if !is_record_bucket(bucket) && kind != "note" {
                kind = "note".to_string();
            }
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
        // R33.6: rules emit decision/action only (the `document` arm is
        // deleted with the kind — status sentences yield nothing).
        if let Some(kind) = rule_kind(s) {
            let conf = match kind.as_str() {
                "decision" => 0.78,
                "action" => 0.72,
                _ => 0.52,
            };
            items.push(ClassifiedItem {
                kind,
                summary: s.chars().take(240).collect(),
                reasoning: "rule-based classifier".to_string(),
                confidence: conf,
                author: String::new(),
                source_ref: source_ref.to_string(),
                window_text: String::new(),
                window_index: None,
            });
        }
    }
    // Rules run for the record bucket only (see `classify_rules_typed`) —
    // normalize as record so decision/action survive.
    normalize_items(items.into_iter().map(|i| serde_json::json!({"kind": i.kind, "summary": i.summary, "reasoning": i.reasoning, "confidence": i.confidence, "author": i.author})).collect(), source_ref, "record")
}

/// Rules fallback, bucket-aware: only `record` extracts without the LLM.
/// `reference` (+ legacy types, + `code`) emit nothing — chunks still make
/// them findable, and the LLM reference prompt extracts notes when online.
pub fn classify_rules_typed(text: &str, source_ref: &str, doc_type: &str) -> Vec<ClassifiedItem> {
    if !is_record_bucket(doc_type) {
        return Vec::new();
    }
    classify_rules(text, source_ref)
}

fn rule_kind(sentence: &str) -> Option<String> {
    static RE_DECISION: OnceLock<Regex> = OnceLock::new();
    static RE_ACTION: OnceLock<Regex> = OnceLock::new();
    let lower = sentence.to_lowercase();
    if RE_DECISION.get_or_init(|| Regex::new(r"\b(we (decided|agreed|chose|approved|rejected|postponed)\b|decision\s*[:\-](\s|$)|(decided|agreed) (to|that|on)\b|(approved|rejected) the\b)").unwrap()).is_match(&lower) {
        return Some("decision".to_string());
    }
    if RE_ACTION.get_or_init(|| Regex::new(r"\b(action item|todo|to-do|next step|owner|assign|follow[- ]up|deadline|due)\b").unwrap()).is_match(&lower) {
        return Some("action".to_string());
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
    fn rule_kind_status_sentences_yield_nothing() {
        // R33.6: the `document` arm is deleted with the kind — status
        // sentences yield nothing from rules (the LLM reference prompt
        // turns them into notes when online).
        assert_eq!(rule_kind("Status update as of now."), None);
        assert_eq!(rule_kind("The current state is green."), None);
    }
    #[test]
    fn classify_rules_fallback() {
        let items = classify_rules("We decided to approve the budget. Status update as of now. Owner: Alice will review the transfer checklist.", "ref");
        let kinds: Vec<String> = items.iter().map(|i| i.kind.clone()).collect();
        assert!(kinds.contains(&"decision".to_string()), "kinds: {kinds:?}");
        assert!(kinds.contains(&"action".to_string()), "kinds: {kinds:?}");
        assert!(!kinds.iter().any(|k| k == "document"), "rules never emit document: {kinds:?}");
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

    #[test]
    fn code_type_extracts_nothing() {
        // R33.2: code is code — no entity extraction from source files, not
        // even from the rules fallback. `code` stays a stored doc_type
        // (chunking signal + badge) but is not a bucket and yields nothing.
        assert!(!BUCKETS.contains(&"code"));
        let items = classify_rules_typed("We decided to use sqlite-vec for vectors.", "main.rs", "code");
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn classify_short_circuits_code_without_llm_call() {
        // R33.2: the choke point — no LLM call is attempted for code, so this
        // is deterministic with or without a provider running.
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = Arc::new(crate::settings::SettingsService::new(store));
        let c = Classifier::new(svc);
        let items = c.classify("We decided to use sqlite-vec for vectors.", "main.rs", "code", true).await;
        assert!(items.is_empty());
    }

    #[test]
    fn reference_and_legacy_emit_nothing_without_llm() {
        // No LLM → only the record bucket extracts; reference, code, and
        // every legacy 8-way value yield nothing (chunks keep them
        // findable; the LLM reference prompt extracts notes when online).
        let text = "We decided to approve the budget. Next step: Alice will draft the plan.";
        for t in ["reference", "code", "meeting", "decision_log", "documentation", "standards", "runbook", "prd", "general", ""] {
            assert!(classify_rules_typed(text, "ref", t).is_empty(), "{t:?} must emit nothing without the LLM");
        }
    }

    #[test]
    fn record_rules_may_emit_decision_and_action() {
        let items = classify_rules_typed("We decided to approve the budget. Next step: Alice will draft the plan.", "ref", "record");
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert!(kinds.contains(&"decision"), "record may emit decisions: {kinds:?}");
        assert!(kinds.contains(&"action"), "record may emit actions: {kinds:?}");
        assert!(!kinds.contains(&"document"), "new extraction never emits document: {kinds:?}");
    }

    #[test]
    fn decision_mentions_are_not_decisions() {
        let items = classify_rules("The decision-making process is described below. What was decided about the transfer?", "ref");
        assert!(items.iter().all(|i| i.kind != "decision"));
    }

    fn items_of(kinds: &[&str]) -> Vec<serde_json::Value> {
        kinds.iter().map(|k| serde_json::json!({"kind": k, "summary": "s", "reasoning": "r", "confidence": 0.9, "author": "a"})).collect()
    }

    #[test]
    fn normalize_reference_coerces_everything_to_note() {
        // A disobedient model emitting decision/action/document for a
        // reference doc still stores notes — the bucket line is enforced
        // in code, not just in the prompt.
        let out = normalize_items(items_of(&["decision", "action", "document", "note", "bogus"]), "ref", "reference");
        assert_eq!(out.len(), 5);
        assert!(out.iter().all(|i| i.kind == "note"));
    }

    #[test]
    fn normalize_record_keeps_decision_action_maps_document() {
        let out = normalize_items(items_of(&["decision", "action", "document", "note"]), "ref", "record");
        let kinds: Vec<&str> = out.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["decision", "action", "note", "note"]);
    }

    #[test]
    fn normalize_legacy_types_are_notes_only() {
        // No grandfathering: only the literal `record` bucket may emit
        // decision/action; legacy values coerce to note until a force
        // reclassify re-buckets them.
        for legacy in ["meeting", "decision_log", "documentation", "general", "prd", "standards", "runbook", ""] {
            let out = normalize_items(items_of(&["decision"]), "ref", legacy);
            assert_eq!(out[0].kind, "note", "legacy {legacy:?} must be notes-only");
        }
    }

    #[test]
    fn bucket_each_marker_fires() {
        // One text per R33.6 record marker (§9 broadened set) — each fires
        // on its own, without help from other markers.
        let cases = [
            ("attendees", "Attendees: Ana, Bo."),
            ("agenda", "Agenda: scope and billing."),
            ("meeting notes", "Meeting notes from Tuesday."),
            ("minutes of", "Minutes of the sync."),
            ("retro", "Sprint retro actions."),
            ("decision log", "The decision log records this."),
            ("decision:", "**Decision:** Migrate now."),
            ("adr-", "See adr-001 for context."),
            ("status accepted", "Status: accepted by the board."),
            ("agreed", "We agreed to postpone."),
            ("decided", "We decided to launch."),
            ("approved", "Sarah approved it."),
            ("Status Confirmed", "**Status:** Confirmed"),
            ("Status Accepted", "**Status:** Accepted"),
            ("Status Approved", "**Status:** Approved"),
            ("# Decision", "# Decision to migrate"),
            ("revisit trigger", "**Revisit trigger:** vendor freezes API."),
            ("revisit triggers", "Revisit triggers: vendor freezes API."),
        ];
        for (name, text) in cases {
            assert_eq!(assign_bucket(text), "record", "marker {name:?} must fire");
        }
    }

    #[test]
    fn bucket_word_boundaries() {
        // Near-misses that must NOT flip the bucket.
        let cases = [
            ("decision logging", "We improved decision logging for router tiers."),
            ("approver", "Sarah is the final approver for payout changes."),
            ("approve", "Ravi must approve any change to price."),
            ("disagreed", "They disagreed on the scope."),
            ("revisit w/o trigger", "Revisit next planning for the wallet call."),
            ("bare decision", "The launch decision quotes this record, not a spreadsheet."),
            ("Status Done", "**Status:** Done. Source: sprint update."),
            ("retrospective", "Retrospective notes are in the wiki."),
            ("agendas", "Agendas were circulated on Friday."),
        ];
        for (name, text) in cases {
            assert_eq!(assign_bucket(text), "reference", "near-miss {name:?} must not fire");
        }
    }

    #[test]
    fn bucket_d3_d4_shaped_texts() {
        // §9: d3 was WOULD HIDE, d4 FRAGILE — both land in `record` now.
        // The d4 shape omits the incidental "retro" line on purpose:
        // genuine `approved` must carry it alone.
        let d3 = "# Mobile wallet integration postponed to Q3\n\nAisha and Elena agreed that shipping against a moving contract would create rework.\n\n**Status:** Confirmed. Source: sprint 12 planning notes.\n\n**Revisit trigger:** vendor freezes auth API.";
        assert_eq!(assign_bucket(d3), "record");
        let d4 = "# Payments service moves to new infrastructure\n\nDev proposed the migration, Sarah approved it after a dry-run showed zero-downtime deploys.\n\n**Status:** Done.";
        assert_eq!(assign_bucket(d4), "record");
    }

    #[test]
    fn bucket_empty_and_plain_prose_default_reference() {
        assert_eq!(assign_bucket(""), "reference");
        assert_eq!(assign_bucket("The widget renders blue panels on load."), "reference");
    }

    #[test]
    fn audit_replay_pins_corpus_buckets() {
        // §8(d): fixtures shaped like the 20 corpus files (titles + marker
        // sentences) pin the §9 bucket line — 8 record sample / 7 record
        // ai, including both weak flips (runbook, model-router-spec).
        let sample = [
            ("d1", "# Decision: Instant Payments MVP excludes incoming payments\n\n**Decision:** The MVP will NOT include incoming payments.\n\n**Status:** Confirmed.", "record"),
            ("d2", "# Decision: Billing provider migration approved\n\n**Decision:** Migrate to the new billing provider.\n\n**Status:** Approved.", "record"),
            ("d3", "# Mobile wallet integration postponed to Q3\n\nAisha and Elena agreed that shipping would create rework.\n\n**Status:** Confirmed.\n\n**Revisit trigger:** vendor freezes auth API.", "record"),
            ("d4", "# Payments service moves to new infrastructure\n\nSarah approved it after a dry-run showed zero-downtime deploys.", "record"),
            ("m1", "# Sprint 12 retro\n\nAttendees: Ana, Bo, Cy.\n\nAction items: re-baseline coverage.", "record"),
            ("m2", "# Q3 planning\n\nAttendees: all leads.\n\nAgenda: scope, billing, wallet.", "record"),
            ("people", "# People\n\nSarah is the final approver for payout changes.\n\nPriya owns the definition of done.", "reference"),
            ("runbook", "# Billing migration runbook\n\nApproved in decisions/d2. Owner: Sarah.\n\nStep 1: provision sandbox.", "record"),
            ("spec", "# Instant payments spec\n\nAn outgoing payment is composed of six movements.", "reference"),
            ("sprint12", "# Sprint 12\n\n## Decision log\n\nWe decided to postpone the wallet integration.", "record"),
        ];
        let sample_ai = [
            ("d1", "# Decision: Model router v2 (tiered routing)\n\n**Decision:** Approved router v2 for tiered routing.\n\n**Status:** Approved.", "record"),
            ("d2", "# Decision: Eval gate thresholds for launch\n\n**Decision:** Builds ship only if the eval suite clears.\n\n**Status:** Approved.", "record"),
            ("d3", "# Decision: Pricing moves to usage tiers\n\n**Decision:** Pricing moves to three usage tiers.\n\n**Status:** Approved.", "record"),
            ("d4", "# Decision: Rebuild the reasoning holdout set\n\n**Decision:** The holdout set is retired and rebuilt.\n\n**Status:** Approved.", "record"),
            ("m1", "# Sprint 14 planning\n\nAttendees: Noor, Sam.\n\nWe improved decision logging for router tiers.", "record"),
            ("m2", "# Incident postmortem\n\nAttendees: on-call crew.\n\nDecided no for now on the auto-rollback.", "record"),
            ("people", "# People\n\nRavi must approve any change to price or credits.", "reference"),
            ("runbook", "# Eval pipeline runbook\n\nThresholds come from decisions/d2.\n\nThe launch decision quotes this record.", "reference"),
            ("spec", "# Model router spec\n\nApproved in decisions/d1. Owner: Noor.\n\nTenants outside an approved policy page on-call.", "record"),
            ("sprint14", "# Sprint 14\n\nCommitted: router v2 shadow mode. The goal is the September launch.", "reference"),
        ];
        for (name, text, want) in sample.iter().chain(sample_ai.iter()) {
            assert_eq!(assign_bucket(text), (*want).to_string(), "fixture {name} must stay {want}");
        }
        let records = |xs: &[(&str, &str, &str)]| xs.iter().filter(|(_, t, _)| assign_bucket(t) == "record").count();
        assert_eq!(records(&sample), 8, "sample/ must stay 8 record / 2 reference");
        assert_eq!(records(&sample_ai), 7, "sample-ai/ must stay 7 record / 3 reference");
    }
}
