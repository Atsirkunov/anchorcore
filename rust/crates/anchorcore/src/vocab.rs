//! R40.7: corpus vocabulary — docs define their own jargon (RFP `Long Form
//! (CODE)` tables), so extract it at ingest and expand FTS query terms with
//! it at query time.
//!
//! Two tiers, one table (`vocabulary`, migration 20):
//! - Tier 1 (rules, always on, no model needed): parenthetical acronyms
//!   (`Direct Debit (CA)`) anywhere + `CODE - expansion` / `Term:
//!   definition` lines under glossary headings. Active immediately.
//! - Tier 2 (LLM piggyback, zero extra calls): `defined_terms` on the
//!   existing classify response. Proposed/inactive until corroborated by a
//!   second independent item, rule-confirmed elsewhere, or human-approved
//!   (R40.8) — a single LLM witness never expands (unit-pinned).
//!
//! Activation is sticky (re-sync deletes an item's rows but never
//! deactivates survivors); `rejected` rows (R40.8) survive re-sync and
//! suppress re-proposal globally.
//!
//! Query time (`vocab_expansion_terms`, merged with the R40.6 curated map
//! in `fts_match_query_scoped`): active pairs fire term→expansion,
//! expansion→term, or multiword-expansion-words ⊆ query-terms → term.
//! Project-scoped (`source_ids`, None = all). Proposed/rejected rows and
//! `"unknown"` lookups never expand.
//!
//! Standalone-safe: only rusqlite/regex/std + `retrieval::query_terms` —
//! the R39.10 eval target compiles this module via `#[path]` like the
//! other retrieval pieces, so no `db`/`settings`/`pipeline` imports
//! outside tests.

use regex::Regex;
use rusqlite::Connection;
use std::collections::HashSet;
use std::sync::OnceLock;

/// One vocabulary pair: short form ↔ spelled-out form. Stored lowercase;
/// the expansion may be multiword (matched as an FTS phrase at query time).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DefinedTerm {
    pub term: String,
    pub expansion: String,
}

/// Caps: bounded work per doc, bounded junk per row, bounded query growth.
const MAX_PAIRS_PER_DOC: usize = 50;
const MAX_TERM_CHARS: usize = 40;
const MAX_EXPANSION_CHARS: usize = 120;
const MAX_QUERY_PAIRS: usize = 64;

/// Two-letter English function words — never acronyms. Without this,
/// `Production (US)` would pair `us` ↔ `production` and every query
/// containing "us" would gain a bogus OR term. (`we` is a query stopword
/// anyway; kept for completeness.)
const ACRONYM_STOPWORDS: &[&str] = &[
    "an", "as", "at", "be", "by", "do", "go", "he", "if", "in", "is", "it", "me", "my", "no",
    "of", "on", "or", "so", "to", "up", "us", "we",
];

/// Normalize a candidate pair (rules + LLM share this): trim, lowercase,
/// drop empties/self-maps/oversize/short. `None` rejects.
pub fn normalize_pair(term: &str, expansion: &str) -> Option<DefinedTerm> {
    let term = term.trim().to_lowercase();
    let expansion = expansion.trim().to_lowercase();
    if term.is_empty() || expansion.is_empty() || term == expansion {
        return None;
    }
    if term.chars().count() < 2 || term.chars().count() > MAX_TERM_CHARS {
        return None;
    }
    if expansion.chars().count() < 2 || expansion.chars().count() > MAX_EXPANSION_CHARS {
        return None;
    }
    Some(DefinedTerm { term, expansion })
}

/// Leading determiners the greedy phrase match sweeps up (`Each Long Form
/// (CODE)` defines Long Form, not Each Long Form) — stripped before pairing.
const PHRASE_LEADERS: &[&str] = &["a", "an", "the", "each", "every", "this", "that", "these", "those", "any", "some", "all"];

/// Tier 1a: `Direct Debit (CA)` — capitalized phrase (1–5 words) + an
/// all-caps acronym (2–10) in parens, anywhere in the text. Lowercase-led
/// phrases and sentence-case parens (`(see Figure 2)`) never match; a
/// leading determiner is stripped (`Each Long Form` → Long Form).
fn parenthetical_pairs(text: &str) -> Vec<DefinedTerm> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\b([A-Z][A-Za-z0-9&]*(?:\s+[A-Z][A-Za-z0-9&]*){0,4})\s*\(([A-Z][A-Z0-9]{1,9})\)")
            .unwrap()
    });
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        if out.len() >= MAX_PAIRS_PER_DOC {
            break;
        }
        let (Some(phrase), Some(acronym)) = (cap.get(1), cap.get(2)) else {
            continue;
        };
        if acronym.as_str().len() == 2 && ACRONYM_STOPWORDS.contains(&acronym.as_str().to_lowercase().as_str()) {
            continue;
        }
        let mut words: Vec<&str> = phrase.as_str().split_whitespace().collect();
        if words.len() > 1 && PHRASE_LEADERS.contains(&words[0].to_lowercase().as_str()) {
            words.remove(0);
        }
        if let Some(pair) = normalize_pair(acronym.as_str(), &words.join(" ")) {
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

/// A glossary heading opens the Tier 1b gate: markdown `#` (or a bare line
/// starting with) glossary/definitions/terminology/abbreviations/acronyms.
fn is_glossary_heading(line: &str) -> bool {
    let t = line.trim().trim_start_matches('#').trim().to_lowercase();
    ["glossary", "definitions", "terminology", "abbreviations", "acronyms"]
        .iter()
        .any(|w| t == *w || t.starts_with(&format!("{w} ")) || t.starts_with(&format!("{w}:")))
}

/// Any markdown heading closes the Tier 1b gate unless it is a glossary one.
fn is_plain_heading(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// Tier 1b: glossary-shaped lines under a glossary heading — `CODE -
/// expansion` and `Term: definition`. The heading gate is what keeps
/// `Note: blah` prose out. URL-shaped rows never pair.
fn glossary_pairs(text: &str) -> Vec<DefinedTerm> {
    static RE_CODE: OnceLock<Regex> = OnceLock::new();
    static RE_TERM: OnceLock<Regex> = OnceLock::new();
    let re_code = RE_CODE.get_or_init(|| Regex::new(r"^([A-Z][A-Z0-9]{1,15})\s*[-–—]\s*(.+)$").unwrap());
    let re_term =
        RE_TERM.get_or_init(|| Regex::new(r"^([A-Z][^:]{1,39}?)\s*:\s+([^:].{1,119})$").unwrap());
    let mut out = Vec::new();
    let mut in_glossary = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if is_glossary_heading(t) {
            in_glossary = true;
            continue;
        }
        if is_plain_heading(t) {
            in_glossary = false;
            continue;
        }
        if !in_glossary || out.len() >= MAX_PAIRS_PER_DOC {
            continue;
        }
        if let Some(pair) = glossary_line_pair(t, re_code, re_term) {
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

/// One glossary line → pair (CODE-dash first, Term-colon second).
fn glossary_line_pair(t: &str, re_code: &Regex, re_term: &Regex) -> Option<DefinedTerm> {
    if let Some(cap) = re_code.captures(t) {
        let (Some(code), Some(exp)) = (cap.get(1), cap.get(2)) else {
            return None;
        };
        return normalize_pair(code.as_str(), exp.as_str());
    }
    let cap = re_term.captures(t)?;
    let (Some(term), Some(def)) = (cap.get(1), cap.get(2)) else {
        return None;
    };
    if term.as_str().contains('/') || def.as_str().starts_with('/') {
        return None;
    }
    normalize_pair(term.as_str(), def.as_str())
}

/// Tier 1 entry: parenthetical ∪ glossary pairs, deduped, capped. Pure —
/// no-model installs get exactly this (Tier 2 needs the classify call).
pub fn extract_vocab_rules(text: &str) -> Vec<DefinedTerm> {
    let mut out = parenthetical_pairs(text);
    for pair in glossary_pairs(text) {
        if out.len() >= MAX_PAIRS_PER_DOC {
            break;
        }
        if !out.contains(&pair) {
            out.push(pair);
        }
    }
    out
}

/// Persist one item's vocabulary (mirrors the chunks delete+insert rhythm):
/// drop the item's non-rejected rows, skip globally-rejected pairs, insert
/// rules as `active` + LLM as `proposed`, then corroborate. Activation is
/// sticky — re-sync never deactivates survivors.
pub fn store_vocab(
    conn: &Connection,
    item_id: i64,
    source_id: i64,
    rules: &[DefinedTerm],
    llm: &[DefinedTerm],
) -> Result<(), String> {
    conn.execute("DELETE FROM vocabulary WHERE item_id = ?1 AND status != 'rejected'", [item_id])
        .map_err(|e| e.to_string())?;
    let rejected: HashSet<(String, String)> = conn
        .prepare("SELECT term, expansion FROM vocabulary WHERE status = 'rejected'")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for pair in rules.iter() {
        insert_pair(conn, pair, item_id, source_id, "rule", "active", &rejected, &mut seen)?;
    }
    for pair in llm.iter() {
        insert_pair(conn, pair, item_id, source_id, "llm", "proposed", &rejected, &mut seen)?;
    }
    corroborate_item(conn, item_id)?;
    Ok(())
}

fn insert_pair(
    conn: &Connection,
    pair: &DefinedTerm,
    item_id: i64,
    source_id: i64,
    origin: &str,
    status: &str,
    rejected: &HashSet<(String, String)>,
    seen: &mut HashSet<(String, String)>,
) -> Result<(), String> {
    let key = (pair.term.clone(), pair.expansion.clone());
    if rejected.contains(&key) || !seen.insert(key) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO vocabulary (term, expansion, item_id, source_id, origin, status) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![pair.term, pair.expansion, item_id, source_id, origin, status],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Activate every proposed row whose pair this item corroborates: a pair is
/// corroborated when its rows span 2+ items, or any row is rule-origin
/// (mechanism independence — a rule row confirms LLM witnesses anywhere,
/// including the same item).
fn corroborate_item(conn: &Connection, item_id: i64) -> Result<(), String> {
    let pairs: Vec<(String, String)> = conn
        .prepare("SELECT DISTINCT term, expansion FROM vocabulary WHERE item_id = ?1")
        .map_err(|e| e.to_string())?
        .query_map([item_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    for (term, expansion) in pairs {
        // Rejected rows witness nothing (a rejected rule row must not
        // corroborate a live proposal).
        let witnesses: Vec<(String, i64)> = conn
            .prepare("SELECT origin, item_id FROM vocabulary WHERE term = ?1 AND expansion = ?2 AND status != 'rejected'")
            .map_err(|e| e.to_string())?
            .query_map(rusqlite::params![term, expansion], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        let rule_present = witnesses.iter().any(|(o, _)| o == "rule");
        let items: HashSet<i64> = witnesses.iter().map(|(_, i)| *i).collect();
        if rule_present || items.len() >= 2 {
            conn.execute(
                "UPDATE vocabulary SET status = 'active' WHERE term = ?1 AND expansion = ?2 AND status = 'proposed'",
                rusqlite::params![term, expansion],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Load active pairs, project-scoped (`None` = every source). Deterministic
/// id order; bounded (corpora carry dozens, not thousands).
fn load_active_pairs(conn: &Connection, source_ids: Option<&HashSet<i64>>) -> Vec<DefinedTerm> {
    let mut sql = String::from("SELECT term, expansion FROM vocabulary WHERE status = 'active'");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(ids) = source_ids {
        if ids.is_empty() {
            return Vec::new();
        }
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND source_id IN ({placeholders})"));
        params.extend(ids.iter().map(|id| rusqlite::types::Value::Integer(*id)));
    }
    sql.push_str(" ORDER BY id LIMIT 1000");
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(DefinedTerm { term: r.get(0)?, expansion: r.get(1)? })
    })
    .map(|m| m.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Query-time FTS expansion (merges with the R40.6 curated map): for each
/// query term, active pairs fire term→expansion, expansion→term, or — for
/// multiword expansions whose content words all appear in the query —
/// expansion→term. Proposed/rejected rows never expand.
pub fn vocab_expansion_terms(
    conn: &Connection,
    terms: &[String],
    source_ids: Option<&HashSet<i64>>,
) -> Vec<String> {
    let pairs = load_active_pairs(conn, source_ids);
    if pairs.is_empty() || terms.is_empty() {
        return Vec::new();
    }
    let term_set: HashSet<&str> = terms.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    for pair in pairs.iter().take(MAX_QUERY_PAIRS) {
        if term_set.contains(pair.term.as_str()) {
            push_unique(&mut out, pair.expansion.clone());
        } else if term_set.contains(pair.expansion.as_str()) {
            push_unique(&mut out, pair.term.clone());
        } else {
            // Multiword long form spelled out across query terms ("corporate"
            // + "action" → "ca"): content words via the shared tokenizer so
            // glue words (`of`) can never block the match.
            let words = crate::retrieval::query_terms(&pair.expansion);
            if !words.is_empty() && words.iter().all(|w| term_set.contains(w.as_str())) {
                push_unique(&mut out, pair.term.clone());
            }
        }
    }
    out
}

fn push_unique(out: &mut Vec<String>, s: String) {
    if !out.iter().any(|t| t == &s) {
        out.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("vocab.db")).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        (dir, conn)
    }

    fn add_item(conn: &Connection, source_id: i64, external: &str) -> i64 {
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, content_hash) VALUES (?1,?2,'t','h')",
            rusqlite::params![source_id, external],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn pair(term: &str, expansion: &str) -> DefinedTerm {
        normalize_pair(term, expansion).expect("test pair must normalize")
    }

    fn status_of(conn: &Connection, term: &str, item_id: i64) -> String {
        conn.query_row(
            "SELECT status FROM vocabulary WHERE term = ?1 AND item_id = ?2",
            rusqlite::params![term, item_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn rules_catch_parenthetical_acronyms() {
        let pairs = extract_vocab_rules("Each Long Form (CODE) row carries a reason. The Direct Debit (CA) record follows.");
        assert!(pairs.contains(&pair("code", "long form")), "CODE pair: {pairs:?}");
        assert!(pairs.contains(&pair("ca", "direct debit")), "CA pair: {pairs:?}");
    }

    #[test]
    fn rules_reject_sentence_parens_and_function_words() {
        let pairs = extract_vocab_rules("As noted (see Figure 2) the file moved to Production (US) last week.");
        assert!(pairs.is_empty(), "no pairs from prose parens: {pairs:?}");
        // `we`/`us`-class guards live in ACRONYM_STOPWORDS; a real 2-letter
        // code still fires.
        assert!(extract_vocab_rules("Direct Debit (CA) filed.").contains(&pair("ca", "direct debit")));
    }

    #[test]
    fn rules_catch_glossary_lines_only_under_glossary_headings() {
        let doc = "Note: this prose colon must not pair.\n\n## Glossary\nCODE - Long Form of the event\nValue Date: the date value settles\n\n## Other\nZZZ - not a glossary row\n";
        let pairs = extract_vocab_rules(doc);
        assert!(pairs.contains(&pair("code", "long form of the event")), "CODE dash: {pairs:?}");
        assert!(pairs.contains(&pair("value date", "the date value settles")), "Term colon: {pairs:?}");
        assert!(!pairs.iter().any(|p| p.term == "note" || p.term == "zzz"), "gated out: {pairs:?}");
    }

    #[test]
    fn rules_reject_urls_and_normalize_caps() {
        assert!(normalize_pair("", "x").is_none());
        assert!(normalize_pair("x", "x").is_none());
        assert!(normalize_pair("t", "long enough expansion").is_none(), "1-char term");
        assert!(normalize_pair(&"t".repeat(41), "expansion").is_none(), "oversize term");
        assert!(normalize_pair("term", &"e".repeat(121)).is_none(), "oversize expansion");
        assert_eq!(normalize_pair("  CA ", "Direct Debit"), Some(pair("ca", "direct debit")));
        let pairs = extract_vocab_rules("## Glossary\nHTTP://example.com/x\n");
        assert!(pairs.is_empty(), "URL rows never pair: {pairs:?}");
    }

    #[test]
    fn llm_single_witness_never_expands() {
        // R40.7 quarantine pin: one LLM witness stays proposed and contributes
        // no expansion terms.
        let (_dir, conn) = vocab_db();
        let iid = add_item(&conn, 1, "a.md");
        store_vocab(&conn, iid, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        assert_eq!(status_of(&conn, "dunning", iid), "proposed");
        let terms = vec!["dunning".to_string()];
        assert!(vocab_expansion_terms(&conn, &terms, None).is_empty(), "proposed must not expand");
    }

    #[test]
    fn second_independent_item_activates() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        let b = add_item(&conn, 1, "b.md");
        store_vocab(&conn, a, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        assert_eq!(status_of(&conn, "dunning", a), "proposed");
        store_vocab(&conn, b, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        assert_eq!(status_of(&conn, "dunning", a), "active", "first witness activates");
        assert_eq!(status_of(&conn, "dunning", b), "active", "second witness activates");
        let terms = vec!["dunning".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &terms, None), vec!["payment reminder".to_string()]);
        // Same-item re-store is not corroboration: one witness stays proposed.
        store_vocab(&conn, a, 1, &[], &[pair("solo", "only here")]).unwrap();
        assert_eq!(status_of(&conn, "solo", a), "proposed");
        store_vocab(&conn, a, 1, &[], &[pair("solo", "only here")]).unwrap();
        assert_eq!(status_of(&conn, "solo", a), "proposed", "re-sync must not self-corroborate");
    }

    #[test]
    fn rule_row_confirms_llm_witnesses() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        let b = add_item(&conn, 1, "b.md");
        store_vocab(&conn, a, 1, &[], &[pair("ca", "direct debit")]).unwrap();
        assert_eq!(status_of(&conn, "ca", a), "proposed");
        // A rule row anywhere (other item here) activates the whole family.
        store_vocab(&conn, b, 1, &[pair("ca", "direct debit")], &[]).unwrap();
        assert_eq!(status_of(&conn, "ca", a), "active", "rule confirms LLM");
        assert_eq!(status_of(&conn, "ca", b), "active", "rules are born active");
    }

    #[test]
    fn rejected_survives_restore_and_suppresses_reproposal() {
        // R40.8 mechanism, pinned early: rejected rows survive their item's
        // re-store and suppress the pair globally.
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        let b = add_item(&conn, 1, "b.md");
        store_vocab(&conn, a, 1, &[pair("us", "production")], &[]).unwrap();
        conn.execute("UPDATE vocabulary SET status = 'rejected' WHERE term = 'us'", []).unwrap();
        store_vocab(&conn, a, 1, &[pair("us", "production")], &[]).unwrap();
        assert_eq!(status_of(&conn, "us", a), "rejected", "re-store preserves rejection");
        store_vocab(&conn, b, 1, &[pair("us", "production")], &[]).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM vocabulary WHERE term = 'us' AND item_id = ?1", [b], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "rejected pair never re-proposed, any item");
        assert!(vocab_expansion_terms(&conn, &["us".to_string()], None).is_empty(), "rejected never expands");
    }

    #[test]
    fn lookup_is_project_scoped() {
        let (_dir, conn) = vocab_db();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s2','folder')", []).unwrap();
        let a = add_item(&conn, 1, "a.md");
        store_vocab(&conn, a, 1, &[pair("dunning", "payment reminder")], &[]).unwrap();
        let terms = vec!["dunning".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &terms, None), vec!["payment reminder".to_string()]);
        let scoped: HashSet<i64> = [2].into_iter().collect();
        assert!(vocab_expansion_terms(&conn, &terms, Some(&scoped)).is_empty(), "other project sees nothing");
        let empty: HashSet<i64> = HashSet::new();
        assert!(vocab_expansion_terms(&conn, &terms, Some(&empty)).is_empty(), "empty scope matches nothing");
    }

    #[test]
    fn multiword_expansion_fires_on_word_subset() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        store_vocab(&conn, a, 1, &[pair("ca", "direct debit")], &[]).unwrap();
        // Long form spelled out across query terms → short form.
        let both = vec!["corporate".to_string(), "action".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &both, None), vec!["ca".to_string()]);
        // One word alone is not the phrase.
        assert!(vocab_expansion_terms(&conn, &["corporate".to_string()], None).is_empty());
        // Reverse direction still works for whole-field hits.
        assert_eq!(vocab_expansion_terms(&conn, &["ca".to_string()], None), vec!["direct debit".to_string()]);
    }
}
