//! R45.8: deterministic record parser — decisions and action items read from
//! the structure records already carry, never guessed from prose.
//!
//! Reads: `# Decision: <title> — D0005`, `**Decision:**`, `**Status:**`,
//! `**Owner:**` / inline `Owner: X`, `Date:`/`Due:`, `Attendees:`, ADR
//! front matter (`status:`, `supersedes:`, `deciders:`), and bullets under an
//! `Action items` heading (`- [ ] Name: task — due YYYY-MM-DD`,
//! `- Name: task. Owner: X. Due: Y.`) plus checkbox bullets anywhere.
//! Never reads table rows or fenced code: reference docs (code tables, ISO
//! field paths) produce nothing. Ingest calls it through
//! `classifier::classify_rules_typed` (record bucket, `extract_entities=rules`).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Superseded,
    Rejected,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionRecord {
    pub record_id: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub status: Option<DecisionStatus>,
    pub status_raw: Option<String>,
    pub owner: Option<String>,
    pub date: Option<String>,
    pub supersedes: Vec<String>,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionRecord {
    pub owner: Option<String>,
    pub task: String,
    pub due: Option<String>,
    pub done: bool,
    /// 1-based line of the bullet in the source text.
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ParsedRecord {
    pub decision: Option<DecisionRecord>,
    pub actions: Vec<ActionRecord>,
    pub attendees: Vec<String>,
}

/// R45.8: record columns on `entities` (migration 23); empty for LLM rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordFields {
    pub record_id: String,
    pub record_status: String,
    pub due: String,
    pub done: bool,
    pub supersedes: String,
    pub superseded_by: String,
}

const RECORD_REASONING: &str = "record parser";

/// Parsed record → entity rows: one decision (title + summary) and one row
/// per action item. Confidence 1.0 is "parsed, not guessed" — the review
/// status stays unverified until a person confirms it.
pub fn record_items(parsed: &ParsedRecord, source_ref: &str) -> Vec<crate::classifier::ClassifiedItem> {
    let mut out = Vec::new();
    if let Some(d) = &parsed.decision {
        let summary = match &d.summary {
            Some(s) if !s.eq_ignore_ascii_case(&d.title) => format!("{}: {}", d.title, s),
            _ => d.title.clone(),
        };
        let record = RecordFields {
            record_id: d.record_id.clone().unwrap_or_default(),
            record_status: d.status.map(status_name).unwrap_or_default().to_string(),
            supersedes: d.supersedes.join(", "),
            superseded_by: d.superseded_by.clone().unwrap_or_default(),
            ..Default::default()
        };
        out.push(item("decision", summary, d.owner.clone(), record, source_ref));
    }
    for a in &parsed.actions {
        let record = RecordFields { due: a.due.clone().unwrap_or_default(), done: a.done, ..Default::default() };
        out.push(item("action", a.task.clone(), a.owner.clone(), record, source_ref));
    }
    out
}

fn item(
    kind: &str,
    summary: String,
    owner: Option<String>,
    record: RecordFields,
    source_ref: &str,
) -> crate::classifier::ClassifiedItem {
    crate::classifier::ClassifiedItem {
        kind: kind.to_string(),
        summary: summary.chars().take(1000).collect(),
        reasoning: RECORD_REASONING.to_string(),
        confidence: 1.0,
        author: String::new(),
        source_ref: source_ref.to_string(),
        window_text: String::new(),
        window_index: None,
        owner: owner.unwrap_or_default(),
        record,
    }
}

fn status_name(s: DecisionStatus) -> &'static str {
    match s {
        DecisionStatus::Proposed => "proposed",
        DecisionStatus::Accepted => "accepted",
        DecisionStatus::Superseded => "superseded",
        DecisionStatus::Rejected => "rejected",
        DecisionStatus::Deferred => "deferred",
    }
}

/// Field keys read from `**Key:** value` or a plain `Key: value` line.
const FIELD_KEYS: &[&str] = &["decision", "status", "owner", "date", "attendees", "supersedes", "deciders", "id"];
const SUMMARY_MAX: usize = 400;

pub fn parse_record(text: &str) -> ParsedRecord {
    let (front, body_start) = front_matter(text);
    let lines = prose_lines(text, body_start);
    let fields = body_fields(&lines);
    let decision = parse_decision(&lines, &fields, &front);
    let actions = parse_actions(&lines);
    let attendees = fields
        .get("attendees")
        .map(|v| split_names(v))
        .unwrap_or_default();
    ParsedRecord { decision, actions, attendees }
}

// --- text preparation ---

/// One prose line: 1-based line number + raw text.
type Line<'a> = (usize, &'a str);

/// YAML-ish front matter (`---` fenced, first line only) as lowercased
/// key → raw value, plus the index of the first body line.
fn front_matter(text: &str) -> (HashMap<String, String>, usize) {
    let mut map = HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (map, 0);
    }
    let mut last_key: Option<String> = None;
    for (i, line) in lines.enumerate() {
        if line.trim() == "---" {
            return (map, i + 2);
        }
        if let Some(item) = line.trim().strip_prefix("- ") {
            if let Some(k) = &last_key {
                let entry: &mut String = map.entry(k.clone()).or_default();
                entry.push_str(if entry.is_empty() { "" } else { ", " });
                entry.push_str(item.trim());
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            map.insert(key.clone(), v.trim().trim_matches(['[', ']', '"', '\'']).to_string());
            last_key = Some(key);
        }
    }
    (HashMap::new(), 0)
}

/// Body lines minus fenced code blocks and table rows.
fn prose_lines(text: &str, body_start: usize) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate().skip(body_start) {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || t.starts_with('|') {
            continue;
        }
        out.push((i + 1, line));
    }
    out
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.bytes().take_while(|b| *b == b'#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    rest.starts_with(' ').then(|| (level, rest.trim()))
}

fn field_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:\*\*([A-Za-z ]+):\*\*|\*\*([A-Za-z ]+)\*\*:|([A-Z][a-z]+):)\s*(.*)$").unwrap())
}

/// `**Key:** value` / `Key: value` fields (first occurrence wins); a value
/// continues over following non-blank, non-field, non-heading lines.
fn body_fields(lines: &[Line]) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((key, value)) = field_at(lines[i].1) else {
            i += 1;
            continue;
        };
        let mut val = value.to_string();
        i += 1;
        while i < lines.len() && continues_value(lines[i].1) {
            val.push(' ');
            val.push_str(lines[i].1.trim());
            i += 1;
        }
        fields.entry(key).or_insert_with(|| val.trim().to_string());
    }
    fields
}

fn field_at(line: &str) -> Option<(String, &str)> {
    let caps = field_re().captures(line)?;
    let key = caps.get(1).or(caps.get(2)).or(caps.get(3))?.as_str().trim().to_ascii_lowercase();
    FIELD_KEYS.contains(&key.as_str()).then(|| (key, caps.get(4).map_or("", |m| m.as_str())))
}

fn continues_value(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && heading(line).is_none() && field_at(line).is_none() && bullet(line).is_none()
}

// --- decisions ---

fn record_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b((?:D|ADR)-?\d{1,5})\b").unwrap())
}

fn parse_decision(
    lines: &[Line],
    fields: &HashMap<String, String>,
    front: &HashMap<String, String>,
) -> Option<DecisionRecord> {
    let h1 = lines.iter().find_map(|(_, l)| heading(l).filter(|(lvl, _)| *lvl == 1).map(|(_, t)| t));
    let explicit_h1 = h1.and_then(|t| strip_prefix_ci(t, "decision:"));
    let status_raw = fields.get("status").or(front.get("status")).map(|s| first_sentence(s));
    let status = status_raw.as_deref().and_then(normalize_status);
    let is_decision = explicit_h1.is_some() || fields.contains_key("decision") || status.is_some();
    if !is_decision {
        return None;
    }
    let raw_title = explicit_h1.or(h1).or(front.get("title").map(String::as_str))?.trim();
    let record_id = front.get("id").cloned().or_else(|| find_id(raw_title));
    Some(DecisionRecord {
        title: strip_id_suffix(raw_title),
        record_id,
        summary: fields.get("decision").cloned().or_else(|| first_paragraph(lines)).map(cap_summary),
        owner: decision_owner(fields, front),
        date: fields.get("date").or(front.get("date")).cloned(),
        supersedes: front.get("supersedes").or(fields.get("supersedes")).map(|v| all_ids(v)).unwrap_or_default(),
        superseded_by: superseded_by(fields.get("status").or(front.get("status"))),
        status,
        status_raw,
    })
}

fn decision_owner(fields: &HashMap<String, String>, front: &HashMap<String, String>) -> Option<String> {
    let direct = fields.get("owner").map(|v| first_sentence(v)).filter(|v| is_name(v));
    direct
        .or_else(|| fields.get("status").and_then(|s| inline_value(s, "owner")).filter(|v| is_name(v)))
        .or_else(|| front.get("deciders").and_then(|d| split_names(d).into_iter().next()))
}

fn normalize_status(raw: &str) -> Option<DecisionStatus> {
    let first = raw.split_whitespace().next()?.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase();
    match first.as_str() {
        "approved" | "accepted" | "confirmed" | "decided" | "agreed" => Some(DecisionStatus::Accepted),
        // A carried-out decision is still in force.
        "done" | "implemented" | "completed" => Some(DecisionStatus::Accepted),
        "superseded" | "replaced" => Some(DecisionStatus::Superseded),
        "rejected" | "declined" => Some(DecisionStatus::Rejected),
        "postponed" | "deferred" => Some(DecisionStatus::Deferred),
        "proposed" | "draft" | "pending" => Some(DecisionStatus::Proposed),
        _ => None,
    }
}

fn superseded_by(status: Option<&String>) -> Option<String> {
    let s = status?;
    let idx = s.to_ascii_lowercase().find("superseded by")?;
    find_id(&s[idx..])
}

fn find_id(text: &str) -> Option<String> {
    record_id_re().captures(text).map(|c| c[1].to_string())
}

fn all_ids(text: &str) -> Vec<String> {
    record_id_re().captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// `Title — D0005` / `Title - D0005` / `Title (D0005)` → `Title`.
fn strip_id_suffix(title: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\s*(?:[—–-]+\s*|\()(?:D|ADR)-?\d{1,5}\)?\s*$").unwrap());
    re.replace(title, "").trim().to_string()
}

/// First prose paragraph after the H1 that is not a field, heading or list.
fn first_paragraph(lines: &[Line]) -> Option<String> {
    let mut para: Vec<&str> = Vec::new();
    for (_, line) in lines {
        let t = line.trim();
        let skip = heading(line).is_some() || field_at(line).is_some() || bullet(line).is_some();
        if t.is_empty() || skip {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push(t);
    }
    (!para.is_empty()).then(|| para.join(" "))
}

fn cap_summary(s: String) -> String {
    if s.chars().count() <= SUMMARY_MAX {
        return s;
    }
    let cut: String = s.chars().take(SUMMARY_MAX).collect();
    format!("{}…", cut.trim_end())
}

// --- action items ---

fn is_action_heading(title: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)^(action items?|actions|next steps|follow[- ]ups?)$").unwrap());
    re.is_match(title.trim_end_matches(':'))
}

/// Bullet marker → (indent, body after the marker).
fn bullet(line: &str) -> Option<(usize, &str)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(\s*)(?:[-*+]|\d+[.)])\s+(.*)$").unwrap());
    let caps = re.captures(line)?;
    Some((caps[1].len(), caps.get(2)?.as_str()))
}

fn checkbox(body: &str) -> Option<(bool, &str)> {
    let rest = body.strip_prefix('[')?;
    let (mark, tail) = rest.split_once(']')?;
    match mark {
        " " => Some((false, tail.trim_start())),
        "x" | "X" => Some((true, tail.trim_start())),
        _ => None,
    }
}

/// Bullets under an action heading (until the next heading at the same or
/// a higher level) plus checkbox bullets anywhere. Same task text dedupes.
fn parse_actions(lines: &[Line]) -> Vec<ActionRecord> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut section: Option<usize> = None;
    let mut i = 0;
    while i < lines.len() {
        let (no, line) = lines[i];
        i += 1;
        if let Some((level, title)) = heading(line) {
            section = match section {
                Some(l) if level > l => Some(l),
                _ => is_action_heading(title).then_some(level),
            };
            continue;
        }
        let Some((indent, body)) = bullet(line) else { continue };
        let text = gather_bullet(lines, &mut i, indent, body);
        let Some(action) = action_from_bullet(&text, section.is_some(), no) else { continue };
        if seen.insert(action.task.to_lowercase()) {
            out.push(action);
        }
    }
    out
}

/// Join a bullet's continuation lines (indented deeper, not a new bullet).
fn gather_bullet(lines: &[Line], i: &mut usize, indent: usize, body: &str) -> String {
    let mut text = body.trim().to_string();
    while *i < lines.len() {
        let line = lines[*i].1;
        let deeper = line.len() - line.trim_start().len() > indent;
        if line.trim().is_empty() || bullet(line).is_some() || heading(line).is_some() || !deeper {
            break;
        }
        text.push(' ');
        text.push_str(line.trim());
        *i += 1;
    }
    text
}

fn action_from_bullet(text: &str, in_section: bool, line: usize) -> Option<ActionRecord> {
    let (checked, body) = match checkbox(text) {
        Some((done, rest)) => (Some(done), rest),
        None if in_section => (None, text),
        None => return None,
    };
    let (lead_owner, rest) = leading_owner(body);
    let owner = inline_value(rest, "owner").filter(|v| is_name(v)).or(lead_owner);
    let mut due = due_value(rest);
    let due_done = due.as_deref().is_some_and(|d| d.eq_ignore_ascii_case("done"));
    if due_done {
        due = None;
    }
    let task = clean_task(rest);
    if task.is_empty() {
        return None;
    }
    Some(ActionRecord { owner, task, due, done: checked.unwrap_or(false) || due_done, line })
}

/// `Name: task` → (Some(Name), task) when the prefix is name-shaped.
fn leading_owner(body: &str) -> (Option<String>, &str) {
    match body.split_once(':') {
        Some((head, tail)) if is_name(head.trim()) && !is_field_word(head.trim()) => {
            (Some(head.trim().to_string()), tail.trim())
        }
        _ => (None, body),
    }
}

fn is_field_word(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "owner" | "due" | "note" | "notes" | "status" | "date" | "todo")
}

fn due_value(text: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\bdue:\s*([^.;\n]+)|\bdue\s+(\d{4}-\d{2}-\d{2})").unwrap());
    let caps = re.captures(text)?;
    caps.get(1).or(caps.get(2)).map(|m| m.as_str().trim().to_string())
}

/// Task text minus `Owner:`/`Due:` sentences and a trailing `— due <date>`.
fn clean_task(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\s*\b(?:owner|due):\s*[^.;\n]*[.;]?|\s*[—–-]+\s*due\s+\d{4}-\d{2}-\d{2}").unwrap()
    });
    re.replace_all(text, "").trim().trim_end_matches(['.', ';', ',']).trim().to_string()
}

// --- shared helpers ---

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| text[prefix.len()..].trim())
}

fn first_sentence(text: &str) -> String {
    text.split(['.', ';', '\n']).next().unwrap_or("").trim().to_string()
}

/// `Key: value` inside free text (value ends at `.`, `;` or newline).
fn inline_value(text: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?i)\b{}:\s*([^.;\n]+)", regex::escape(key))).ok()?;
    re.captures(text).map(|c| c[1].trim().to_string())
}

/// 1–3 capitalized words, e.g. `Priya`, `Mary Ann`, `O'Neil`.
fn is_name(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^\p{Lu}[\p{L}'-]*(?:\s+\p{Lu}[\p{L}'-]*){0,2}$").unwrap());
    text.len() <= 40 && re.is_match(text)
}

fn split_names(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .flat_map(|p| p.split(" and "))
        .map(|p| p.trim().trim_end_matches('.').trim().to_string())
        .filter(|p| is_name(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    #[test]
    fn sample_decision_with_inline_status_owner() {
        let r = parse_record(&fixture("sample/decisions/d2-billing-provider-approval.md"));
        let d = r.decision.expect("decision");
        assert_eq!(d.title, "Billing provider migration approved");
        assert_eq!(d.status, Some(DecisionStatus::Accepted));
        assert_eq!(d.owner.as_deref(), Some("Sarah"));
        assert_eq!(d.summary.as_deref(), Some("We approved moving to the new billing provider."));
        assert_eq!(d.record_id, None);
        assert!(r.actions.is_empty(), "constraints are not action items: {:?}", r.actions);
    }

    #[test]
    fn sample_decision_without_decision_prefix() {
        let r = parse_record(&fixture("sample/decisions/d3-mobile-wallet-postponed.md"));
        let d = r.decision.expect("Status: Confirmed marks a decision");
        assert_eq!(d.title, "Mobile wallet integration postponed to Q3");
        assert_eq!(d.status, Some(DecisionStatus::Accepted));
        assert_eq!(d.owner.as_deref(), Some("Aisha"));
        assert_eq!(d.summary.as_deref(), Some("The mobile wallet integration is postponed to Q3."));
    }

    #[test]
    fn sample_retro_actions_and_attendees() {
        let r = parse_record(&fixture("sample/meetings/m1-sprint12-retro.md"));
        assert!(r.decision.is_none(), "a retro is not a decision");
        assert_eq!(r.attendees, ["Sarah", "Mike", "Priya", "Dev", "Elena", "Aisha"]);
        let got: Vec<(Option<&str>, Option<&str>, bool)> =
            r.actions.iter().map(|a| (a.owner.as_deref(), a.due.as_deref(), a.done)).collect();
        assert_eq!(
            got,
            [
                (Some("Priya"), Some("end of this week"), false),
                (Some("Mike"), None, false),
                (Some("Elena"), Some("next sprint planning"), false),
                (Some("Sarah"), None, false),
            ]
        );
        assert_eq!(r.actions[0].task, "re-baseline test coverage on new hosts and publish corrected number");
        assert_eq!(r.actions[1].task, "close the two remaining alerting gaps from the migration");
        assert!(r.actions[3].task.starts_with("confirm the queue provider decision"));
    }

    #[test]
    fn sample_ai_postmortem_due_done_marks_done() {
        let r = parse_record(&fixture("sample-ai/meetings/m2-incident-postmortem.md"));
        assert_eq!(r.actions.len(), 3, "only the Action items section: {:?}", r.actions);
        let noor = &r.actions[0];
        assert_eq!((noor.owner.as_deref(), noor.done, noor.due.as_deref()), (Some("Noor"), true, None));
        assert_eq!(r.actions[2].due.as_deref(), Some("sprint 15"));
    }

    #[test]
    fn every_sample_decision_parses() {
        for dir in ["sample/decisions", "sample-ai/decisions"] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join(dir);
            for entry in std::fs::read_dir(&root).unwrap() {
                let path = entry.unwrap().path();
                let d = parse_record(&std::fs::read_to_string(&path).unwrap()).decision;
                let d = d.unwrap_or_else(|| panic!("{} must parse as a decision", path.display()));
                assert!(d.status.is_some() && d.owner.is_some(), "{}: {d:?}", path.display());
            }
        }
    }

    #[test]
    fn id_checkbox_and_superseded() {
        let text = "# Decision: Stripe provider — D0005\n\n**Status:** Superseded by D0012\n**Owner:** Maya\n\n\
                    ## Action items\n- [ ] Alex: draft spec — due 2024-06-23\n- [x] Maya: sign contract\n";
        let r = parse_record(text);
        let d = r.decision.unwrap();
        assert_eq!((d.title.as_str(), d.record_id.as_deref()), ("Stripe provider", Some("D0005")));
        assert_eq!(d.status, Some(DecisionStatus::Superseded));
        assert_eq!(d.superseded_by.as_deref(), Some("D0012"));
        assert_eq!(d.owner.as_deref(), Some("Maya"));
        let a = &r.actions;
        assert_eq!((a[0].task.as_str(), a[0].due.as_deref(), a[0].done), ("draft spec", Some("2024-06-23"), false));
        assert_eq!((a[1].owner.as_deref(), a[1].done), (Some("Maya"), true));
        assert_eq!(a[0].line, 7);
    }

    #[test]
    fn adr_front_matter() {
        let text = "---\nstatus: accepted\nsupersedes: [ADR-3]\ndeciders:\n  - Lena\n  - Sam\n---\n# Use SQLite for local state\n\nWe store state in SQLite.\n";
        let d = parse_record(text).decision.unwrap();
        assert_eq!(d.title, "Use SQLite for local state");
        assert_eq!(d.status, Some(DecisionStatus::Accepted));
        assert_eq!(d.supersedes, ["ADR-3"]);
        assert_eq!(d.owner.as_deref(), Some("Lena"));
        assert_eq!(d.summary.as_deref(), Some("We store state in SQLite."));
    }

    #[test]
    fn reference_docs_produce_nothing() {
        // Code tables and field paths used to become "decisions"/"actions".
        let text = "# Refund reason codes\n\n| Code | Owner | Meaning |\n|---|---|---|\n| RFND | Agent | Refund |\n\n\
                    The account owner field maps to `Document/Owner/Id`.\n\n```\n- [ ] Owner: not a task\n**Status:** Approved\n```\n\
                    - Owner: the issuer that holds the account\n";
        assert_eq!(parse_record(text), ParsedRecord::default());
    }

    #[test]
    fn unknown_status_is_not_a_decision() {
        let r = parse_record("# Service runbook\n\n**Status:** Living document\n");
        assert!(r.decision.is_none());
    }
}
