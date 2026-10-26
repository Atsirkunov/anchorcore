//! Entity extraction — deterministic, no model (R45.18 deleted the LLM
//! extraction opt-in and distillation; R45.1 had already made rules the
//! default). Record structure is read by `records.rs`.
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
    // R45.14: an `Action items` heading or bold label. Stray checkboxes
    // alone never flip the bucket — `records::parse_record` reads them.
    r"(?m)^#{1,6}\s+action items?\s*:?\s*$",
    r"(?m)^\*\*action items?:?\*\*:?\s*$",
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

/// R45.1/R45.18: how ingest extracts entities (`extract_entities` setting).
/// `rules` (default) reads record structure; `off` stores no entities.
/// Anything else — including the retired `llm` — reads as `rules`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractMode {
    Off,
    Rules,
}

pub fn extract_mode(settings: &SettingsService) -> ExtractMode {
    let raw = settings.get("extract_entities", None).unwrap_or_default();
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" => ExtractMode::Off,
        _ => ExtractMode::Rules,
    }
}

/// One classify call's output. R45.13: the R40.7 `defined_terms`
/// piggyback is gone — vocabulary is glossary + initialism rule only.
#[derive(Debug, Clone)]
pub struct ClassifyOutput {
    pub items: Vec<ClassifiedItem>,
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
    /// R45.8: parser-filled owner + record fields.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub record: crate::records::RecordFields,
}

pub struct Classifier {
    pub settings: Arc<SettingsService>,
}

impl Classifier {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings }
    }

    /// R33.6: `doc_type` is the deterministic bucket (`record`/`reference`,
    /// `code`) or a legacy 8-way value (treated as notes-only — see
    /// `is_record_bucket`). Newly extracted prose always stores the bucket.
    pub fn classify(&self, text: &str, source_ref: &str, doc_type: &str) -> ClassifyOutput {
        // R33.2: code is code — no entity extraction from source files. The
        // code itself is the retrievable unit.
        if doc_type == "code" {
            return ClassifyOutput { items: Vec::new() };
        }
        let items = match extract_mode(&self.settings) {
            ExtractMode::Off => Vec::new(),
            ExtractMode::Rules => classify_rules_typed(text, source_ref, doc_type),
        };
        ClassifyOutput { items }
    }
}

/// Rules extraction, bucket-aware: only `record` extracts.
/// `reference` (+ legacy types, + `code`) emit nothing — chunks still make
/// them findable.
/// R45.8: records are read by the deterministic parser (`records.rs`), not
/// sentence regexes — those turned code tables and field paths into
/// decisions/actions.
pub fn classify_rules_typed(text: &str, source_ref: &str, doc_type: &str) -> Vec<ClassifiedItem> {
    if !is_record_bucket(doc_type) {
        return Vec::new();
    }
    crate::records::record_items(&crate::records::parse_record(text), source_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;
    #[test]
    fn classify_rules_reads_record_structure() {
        let text = "# Decision: Approve the budget — D0007\n\n**Status:** Approved. Owner: Alice.\n\n## Action items\n- Bob: draft the plan. Due: Friday.\n";
        let items = classify_rules_typed(text, "ref", "record");
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["decision", "action"]);
        assert_eq!((items[0].owner.as_str(), items[0].record.record_id.as_str()), ("Alice", "D0007"));
        assert_eq!((items[1].owner.as_str(), items[1].record.due.as_str()), ("Bob", "Friday"));
        assert!(classify_rules_typed(text, "ref", "reference").is_empty(), "reference never extracts");
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

    #[test]
    fn classify_short_circuits_code() {
        // R33.2: the choke point — code yields nothing, whatever it reads like.
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = Arc::new(crate::settings::SettingsService::new(store));
        let c = Classifier::new(svc);
        let out = c.classify("We decided to use sqlite-vec for vectors.", "main.rs", "code");
        assert!(out.items.is_empty());
    }

    #[test]
    fn reference_and_legacy_emit_nothing() {
        // Only the record bucket extracts; reference, code, and every
        // legacy 8-way value yield nothing (chunks keep them findable).
        let text = "We decided to approve the budget. Next step: Alice will draft the plan.";
        for t in ["reference", "code", "meeting", "decision_log", "documentation", "standards", "runbook", "prd", "general", ""] {
            assert!(classify_rules_typed(text, "ref", t).is_empty(), "{t:?} must emit nothing");
        }
    }

    #[test]
    fn prose_sentences_are_not_records() {
        // R45.8: "we decided" / "next step" / "owner" in running prose no
        // longer mint entities — only record structure does.
        for text in [
            "We decided to approve the budget. Next step: Alice will draft the plan.",
            "The decision-making process is described below. What was decided about the payment?",
        ] {
            assert!(classify_rules_typed(text, "ref", "record").is_empty(), "{text}");
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
    fn bucket_action_items_section_is_record() {
        // R45.14: the section is the only marker; it buckets `record` and parses.
        for text in [
            "# Billing sync\n\n## Action items\n- [ ] Alex: draft spec — due 2024-06-23\n- [x] Maya: sign contract\n",
            "# Billing sync\n\n**Action items:**\n- [ ] Alex: draft spec — due 2024-06-23\n",
        ] {
            assert_eq!(assign_bucket(text), "record", "{text:?}");
            assert!(!crate::records::parse_record(text).actions.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn bucket_stray_checkboxes_stay_reference() {
        for text in [
            "# Setup guide\n\n- [ ] install the CLI\n- [x] create a token\n",
            "# Workflow\n\nList the action items in the ticket body.\n",
            "# CI\n\n## Actions\n- build\n",
        ] {
            assert_eq!(assign_bucket(text), "reference", "{text:?}");
        }
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
            ("spec", "# Instant payments spec\n\nAn outgoing payment is composed of six steps.", "reference"),
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
