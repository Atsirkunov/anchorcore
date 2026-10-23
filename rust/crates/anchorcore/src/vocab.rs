//! R45.13: vocabulary without proposals — a people-owned glossary plus one
//! near-certain automatic rule. Replaces the R40.7/R40.8 tiers (LLM
//! `defined_terms`, corroboration quarantine, "Words to confirm" queue):
//! index-time guessing produced the junk and the queue handed cleanup to
//! people (owner feedback 2026-10-21).
//!
//! One table (`vocabulary`, migrations 20 + 24), three origins, all active:
//! - `manual`: typed in by a person (`/glossary` CRUD). Global (no source).
//! - `glossary`: parsed from a file a person marked as a glossary
//!   (`PATCH /items/:id {"is_glossary": true}`) — CSV/TSV, Markdown tables,
//!   `CODE - expansion` and `Term: definition` lines. Scoped to its source.
//! - `rule`: `Full Phrase (FP)` parentheticals whose letters match the
//!   phrase's word initials (`is_initialism`). Scoped to its source. Code
//!   tables are never auto-extracted — mark the file instead.
//!
//! Query time (`vocab_expansion_terms`, merged with the R40.6 curated map in
//! `fts_match_query_scoped`): pairs fire term→expansion, expansion→term, or
//! multiword-expansion-words ⊆ query-terms → term. Project-scoped
//! (`source_ids`, None = all; manual rows apply everywhere). Agent mode
//! (`llm_mode=agent`) skips `rule` rows — the agent broadens its own
//! queries; the glossary still applies.
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

/// A glossary row as the API shows it (manual + marked-file rows).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GlossaryEntry {
    pub id: i64,
    pub term: String,
    pub expansion: String,
    pub origin: String,
    pub item_id: Option<i64>,
    pub path: Option<String>,
}

/// Caps: bounded work per doc, bounded junk per row, bounded query growth.
const MAX_PAIRS_PER_DOC: usize = 50;
const MAX_GLOSSARY_FILE_PAIRS: usize = 2000;
const MAX_TERM_CHARS: usize = 40;
const MAX_EXPANSION_CHARS: usize = 120;
const MAX_QUERY_PAIRS: usize = 64;

pub const ORIGIN_RULE: &str = "rule";
pub const ORIGIN_GLOSSARY: &str = "glossary";
pub const ORIGIN_MANUAL: &str = "manual";

/// Two-letter English function words — never acronyms. Without this,
/// `Production (US)` would pair `us` ↔ `production` and every query
/// containing "us" would gain a bogus OR term.
const ACRONYM_STOPWORDS: &[&str] = &[
    "an", "as", "at", "be", "by", "do", "go", "he", "if", "in", "is", "it", "me", "my", "no",
    "of", "on", "or", "so", "to", "up", "us", "we",
];

/// Glue words an initialism may skip (`Bank of England (BOE)` / `(BE)`).
const GLUE_WORDS: &[&str] = &["of", "and", "for", "the", "to", "in", "on", "&", "a", "an"];

/// Normalize a candidate pair: trim, lowercase, drop empties/self-maps/
/// oversize/short. `None` rejects.
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

// --- the one automatic rule: initialism-checked parentheticals ---

/// Per-word letter sources: the first letter plus inner capitals
/// (`JavaScript` → j, s), lowercased.
fn word_letters(word: &str) -> Vec<char> {
    let mut chars = word.chars().filter(|c| c.is_alphanumeric());
    let Some(first) = chars.next() else { return Vec::new() };
    let mut out = vec![first.to_ascii_lowercase()];
    out.extend(chars.filter(|c| c.is_uppercase()).map(|c| c.to_ascii_lowercase()));
    out
}

/// Every word contributes its first letter, optionally followed by a
/// prefix of its inner capitals; the acronym must be consumed exactly.
fn consume(letters: &[char], words: &[Vec<char>]) -> bool {
    let Some((w, rest)) = words.split_first() else { return letters.is_empty() };
    (1..=w.len()).any(|n| letters.len() >= n && letters[..n] == w[..n] && consume(&letters[n..], rest))
}

fn initials_match(letters: &[char], words: &[&str]) -> bool {
    let all: Vec<Vec<char>> = words.iter().map(|w| word_letters(w)).filter(|l| !l.is_empty()).collect();
    if consume(letters, &all) {
        return true;
    }
    let content: Vec<Vec<char>> = words
        .iter()
        .filter(|w| !GLUE_WORDS.contains(&w.to_lowercase().as_str()))
        .map(|w| word_letters(w))
        .filter(|l| !l.is_empty())
        .collect();
    consume(letters, &content)
}

/// R45.13: the acronym's letters (digits ignored) are the phrase's word
/// initials. Returns the index of the first phrase word that starts the
/// match — the greedy phrase regex sweeps up leading capitalized words
/// (`Each Long Form`), so trailing sub-phrases are tried too.
pub fn initialism_start(acronym: &str, words: &[&str]) -> Option<usize> {
    let letters: Vec<char> =
        acronym.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    if letters.len() < 2 {
        return None;
    }
    // A phrase never starts on a glue word (`The Direct Debit (DD)`).
    (0..words.len())
        .filter(|&start| !GLUE_WORDS.contains(&words[start].to_lowercase().as_str()))
        .find(|&start| initials_match(&letters, &words[start..]))
}

/// True when a stored (lowercase) pair passes the initialism check.
pub fn is_initialism_pair(term: &str, expansion: &str) -> bool {
    let words: Vec<&str> = expansion.split_whitespace().collect();
    initialism_start(term, &words) == Some(0)
}

/// `Direct Debit (DD)` — capitalized phrase (1–5 words) + an all-caps
/// acronym (2–10) in parens whose letters are the phrase initials.
fn parenthetical_pairs(text: &str) -> Vec<DefinedTerm> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\b([A-Z][A-Za-z0-9&-]*(?:\s+(?:[A-Z][A-Za-z0-9&-]*|of|and|for|the|to|on|in)){0,5})\s*\(([A-Z][A-Z0-9]{1,9})\)")
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
        if let Some(pair) = parenthetical_pair(phrase.as_str(), acronym.as_str()) {
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

fn parenthetical_pair(phrase: &str, acronym: &str) -> Option<DefinedTerm> {
    if acronym.len() == 2 && ACRONYM_STOPWORDS.contains(&acronym.to_lowercase().as_str()) {
        return None;
    }
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let start = initialism_start(acronym, &words)?;
    normalize_pair(acronym, &words[start..].join(" "))
}

/// Automatic extraction at ingest: initialism-checked parentheticals only.
/// Pure — no model, no queue.
pub fn extract_vocab_rules(text: &str) -> Vec<DefinedTerm> {
    parenthetical_pairs(text)
}

// --- marked glossary files ---

const HEADER_TERMS: &[&str] = &["term", "terms", "acronym", "abbreviation", "code", "name", "short form", "key"];
const HEADER_EXPANSIONS: &[&str] =
    &["expansion", "meaning", "definition", "description", "long form", "full form", "value", "stands for"];

/// Parse a file a person marked as a glossary: Markdown table rows, CSV /
/// TSV / semicolon rows (first two fields), `CODE - expansion` and `Term:
/// definition` lines. Header rows and table separators are skipped. The
/// person chose the file, so nothing is held back for review.
pub fn parse_glossary_file(text: &str) -> Vec<DefinedTerm> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        if out.len() >= MAX_GLOSSARY_FILE_PAIRS {
            break;
        }
        let Some((term, expansion)) = glossary_line_fields(line.trim()) else { continue };
        if is_header_row(&term, &expansion) {
            continue;
        }
        if let Some(pair) = normalize_pair(strip_markup(&term), strip_markup(&expansion)) {
            if seen.insert(pair.clone()) {
                out.push(pair);
            }
        }
    }
    out
}

fn glossary_line_fields(t: &str) -> Option<(String, String)> {
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    if t.starts_with('|') {
        return table_row_fields(t);
    }
    if t.contains('\t') {
        return delimited_fields(t, '\t');
    }
    if let Some(fields) = dash_or_colon_fields(t) {
        return Some(fields);
    }
    [';', ','].into_iter().find(|d| t.contains(*d)).and_then(|d| delimited_fields(t, d))
}

fn table_row_fields(t: &str) -> Option<(String, String)> {
    let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
    if cells.iter().all(|c| c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) {
        return None;
    }
    Some((cells.first()?.to_string(), cells.get(1)?.to_string()))
}

/// First two fields of a delimited row; double quotes group (`"a, b"`).
fn delimited_fields(t: &str, delim: char) -> Option<(String, String)> {
    let mut fields: Vec<String> = vec![String::new()];
    let mut quoted = false;
    for ch in t.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c == delim && !quoted => fields.push(String::new()),
            c => fields.last_mut()?.push(c),
        }
    }
    let mut it = fields.into_iter();
    Some((it.next()?, it.next()?))
}

fn dash_or_colon_fields(t: &str) -> Option<(String, String)> {
    static RE_CODE: OnceLock<Regex> = OnceLock::new();
    static RE_TERM: OnceLock<Regex> = OnceLock::new();
    let re_code = RE_CODE.get_or_init(|| Regex::new(r"^([A-Za-z][A-Za-z0-9]{1,15})\s+[-–—]\s+(.+)$").unwrap());
    let re_term = RE_TERM.get_or_init(|| Regex::new(r"^([^:]{2,40}?)\s*:\s+(.{2,})$").unwrap());
    let caps = re_code.captures(t).or_else(|| re_term.captures(t))?;
    Some((caps[1].to_string(), caps[2].to_string()))
}

fn is_header_row(term: &str, expansion: &str) -> bool {
    let t = strip_markup(term).to_lowercase();
    let e = strip_markup(expansion).to_lowercase();
    HEADER_TERMS.contains(&t.as_str()) && HEADER_EXPANSIONS.contains(&e.as_str())
}

/// Bold/code/bullet markup around a cell (`**DD**`, `` `DD` ``, `- DD`).
fn strip_markup(s: &str) -> &str {
    s.trim().trim_start_matches(['-', '*', '+']).trim().trim_matches(['*', '`', '_', '"']).trim()
}

// --- storage ---

/// Persist one item's automatic + marked-file vocabulary (mirrors the
/// chunks delete+insert rhythm). Manual rows are never touched here.
pub fn store_vocab(
    conn: &Connection,
    item_id: i64,
    source_id: i64,
    rules: &[DefinedTerm],
    glossary: &[DefinedTerm],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM vocabulary WHERE item_id = ?1 AND origin IN ('rule','glossary')",
        [item_id],
    )
    .map_err(|e| e.to_string())?;
    insert_pairs(conn, glossary, Some(item_id), Some(source_id), ORIGIN_GLOSSARY)?;
    insert_pairs(conn, rules, Some(item_id), Some(source_id), ORIGIN_RULE)
}

/// Replace one item's marked-file rows (mark/unmark via `PATCH /items`).
pub fn store_glossary_file(conn: &Connection, item_id: i64, source_id: i64, pairs: &[DefinedTerm]) -> Result<usize, String> {
    conn.execute("DELETE FROM vocabulary WHERE item_id = ?1 AND origin = 'glossary'", [item_id])
        .map_err(|e| e.to_string())?;
    insert_pairs(conn, pairs, Some(item_id), Some(source_id), ORIGIN_GLOSSARY)?;
    Ok(pairs.len())
}

fn insert_pairs(
    conn: &Connection,
    pairs: &[DefinedTerm],
    item_id: Option<i64>,
    source_id: Option<i64>,
    origin: &str,
) -> Result<(), String> {
    let mut seen: HashSet<&DefinedTerm> = HashSet::new();
    for pair in pairs.iter().filter(|p| seen.insert(p)) {
        conn.execute(
            "INSERT INTO vocabulary (term, expansion, item_id, source_id, origin, status) VALUES (?1,?2,?3,?4,?5,'active')",
            rusqlite::params![pair.term, pair.expansion, item_id, source_id, origin],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Manual + marked-file rows, the people-owned glossary (rule rows are
/// automatic and not listed). Manual first, then by term.
pub fn list_glossary(conn: &Connection) -> Vec<GlossaryEntry> {
    let sql = "SELECT v.id, v.term, v.expansion, v.origin, v.item_id, i.external_id \
               FROM vocabulary v LEFT JOIN ingested_items i ON i.id = v.item_id \
               WHERE v.origin IN ('manual','glossary') \
               ORDER BY v.origin = 'glossary', v.term, v.id LIMIT 5000";
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    stmt.query_map([], |r| {
        Ok(GlossaryEntry {
            id: r.get(0)?,
            term: r.get(1)?,
            expansion: r.get(2)?,
            origin: r.get(3)?,
            item_id: r.get(4)?,
            path: r.get(5)?,
        })
    })
    .map(|m| m.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Add a manual entry. Errors are user-facing `detail` strings.
pub fn add_manual(conn: &Connection, term: &str, expansion: &str) -> Result<GlossaryEntry, String> {
    let pair = normalize_pair(term, expansion).ok_or_else(|| {
        format!("term must be 2-{MAX_TERM_CHARS} chars and expansion 2-{MAX_EXPANSION_CHARS} chars, and differ")
    })?;
    let dup: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM vocabulary WHERE origin = 'manual' AND term = ?1 AND expansion = ?2",
            rusqlite::params![pair.term, pair.expansion],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if dup > 0 {
        return Err("That entry is already in the glossary".to_string());
    }
    insert_pairs(conn, std::slice::from_ref(&pair), None, None, ORIGIN_MANUAL)?;
    Ok(GlossaryEntry {
        id: conn.last_insert_rowid(),
        term: pair.term,
        expansion: pair.expansion,
        origin: ORIGIN_MANUAL.to_string(),
        item_id: None,
        path: None,
    })
}

/// Delete a manual entry; `Ok(false)` when no manual row has that id
/// (marked-file rows go away by unmarking the file).
pub fn delete_manual(conn: &Connection, id: i64) -> Result<bool, String> {
    conn.execute("DELETE FROM vocabulary WHERE id = ?1 AND origin = 'manual'", [id])
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
}

/// Migration 24 hook body: keep only rows a person owns or the
/// initialism rule would still produce. Returns rows deleted.
pub fn prune_to_trusted(conn: &Connection) -> rusqlite::Result<usize> {
    let mut deleted = conn.execute("DELETE FROM vocabulary WHERE origin NOT IN ('rule','manual','glossary') OR status != 'active'", [])?;
    let rules: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, term, expansion FROM vocabulary WHERE origin = 'rule'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, term, expansion) in rules {
        if !is_initialism_pair(&term, &expansion) {
            deleted += conn.execute("DELETE FROM vocabulary WHERE id = ?1", [id])?;
        }
    }
    Ok(deleted)
}

// --- query time ---

/// Agent mode broadens its own queries — automatic rule rows stay out
/// (the glossary still applies). Reads the same env/DB pair as
/// `settings::llm_mode` without importing it (standalone-safe).
fn rules_apply(conn: &Connection) -> bool {
    let env = std::env::var("ANCHOR_LLM_MODE").ok().filter(|v| !v.trim().is_empty());
    let db = || {
        conn.query_row("SELECT value FROM app_settings WHERE key = 'llm_mode'", [], |r| r.get::<_, String>(0))
            .ok()
            .filter(|v| !v.trim().is_empty())
    };
    let mode = db().or(env).unwrap_or_default();
    !mode.trim().eq_ignore_ascii_case("agent")
}

/// Load pairs for a query, project-scoped (`None` = every source; manual
/// rows apply in every scope). Deterministic id order; bounded.
fn load_pairs(conn: &Connection, source_ids: Option<&HashSet<i64>>, include_rules: bool) -> Vec<DefinedTerm> {
    let mut sql = String::from("SELECT term, expansion FROM vocabulary WHERE status = 'active'");
    if !include_rules {
        sql.push_str(" AND origin != 'rule'");
    }
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(ids) = source_ids {
        if ids.is_empty() {
            return Vec::new();
        }
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND (source_id IS NULL OR source_id IN ({placeholders}))"));
        params.extend(ids.iter().map(|id| rusqlite::types::Value::Integer(*id)));
    }
    sql.push_str(" ORDER BY origin = 'rule', id LIMIT 1000");
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(DefinedTerm { term: r.get(0)?, expansion: r.get(1)? })
    })
    .map(|m| m.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Query-time FTS expansion (merges with the R40.6 curated map): for each
/// query term, pairs fire term→expansion, expansion→term, or — for
/// multiword expansions whose content words all appear in the query —
/// expansion→term.
pub fn vocab_expansion_terms(
    conn: &Connection,
    terms: &[String],
    source_ids: Option<&HashSet<i64>>,
) -> Vec<String> {
    if terms.is_empty() {
        return Vec::new();
    }
    let pairs = load_pairs(conn, source_ids, rules_apply(conn));
    let term_set: HashSet<&str> = terms.iter().map(|s| s.as_str()).collect();
    let mut out = Vec::new();
    for pair in pairs.iter().take(MAX_QUERY_PAIRS) {
        if term_set.contains(pair.term.as_str()) {
            push_unique(&mut out, pair.expansion.clone());
        } else if term_set.contains(pair.expansion.as_str()) {
            push_unique(&mut out, pair.term.clone());
        } else {
            // Multiword long form spelled out across query terms ("direct"
            // + "debit" → "dd"): content words via the shared tokenizer so
            // glue words (`of`) can never block the match.
            let words = crate::retrieval::query_terms(&pair.expansion);
            if !words.is_empty() && words.iter().all(|w| term_set.contains(w.as_str())) {
                push_unique(&mut out, pair.term.clone());
            }
        }
    }
    out
}

/// Lowercased terms that already have any vocabulary row — local-mode
/// acronym expansion skips these (the corpus already answers them).
pub fn known_terms(conn: &Connection, terms: &[String]) -> HashSet<String> {
    terms
        .iter()
        .filter(|t| {
            conn.query_row("SELECT COUNT(*) FROM vocabulary WHERE term = ?1", [t.to_lowercase()], |r| r.get::<_, i64>(0))
                .map(|n| n > 0)
                .unwrap_or(false)
        })
        .map(|t| t.to_lowercase())
        .collect()
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
        // DB beats env: pin local mode so a parallel test's ANCHOR_LLM_MODE
        // can't flip the rule-row assertions.
        conn.execute("INSERT INTO app_settings (key, value) VALUES ('llm_mode','local')", []).unwrap();
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

    #[test]
    fn rules_catch_initialism_parentheticals() {
        let pairs = extract_vocab_rules(
            "Each Service Level Agreement (SLA) row carries a reason. The Direct Debit (DD) record follows. \
             Bank of England (BOE) rates apply. JavaScript Object Notation (JSON) payloads.",
        );
        assert!(pairs.contains(&pair("sla", "service level agreement")), "leading word trimmed: {pairs:?}");
        assert!(pairs.contains(&pair("dd", "direct debit")), "{pairs:?}");
        assert!(pairs.contains(&pair("boe", "bank of england")), "glue word kept in phrase: {pairs:?}");
        assert!(pairs.contains(&pair("json", "javascript object notation")), "inner capitals: {pairs:?}");
    }

    #[test]
    fn rules_reject_non_initialisms_and_owner_db_junk() {
        // R45.13 DoD: the owner-DB junk shapes (§4.4) can't be produced.
        let text = "Card Account Issuer Payment Agent (PSA) field. Update (CNCL) event. \
                    Payments (P2P) platform. Managed Services (XYZ) desk. Refund (RFND). \
                    As noted (see Figure 2) the file moved to Production (US) last week.";
        let pairs = extract_vocab_rules(text);
        assert!(pairs.is_empty(), "no non-initialism pairs: {pairs:?}");
        for (t, e) in [("psa", "card account issuer payment agent"), ("cncl", "update"), ("p2p", "payments")] {
            assert!(!is_initialism_pair(t, e), "{t} → {e} must fail the check");
        }
        assert!(is_initialism_pair("p2p", "peer payments"), "digits are ignored, letters must match");
    }

    #[test]
    fn code_tables_are_never_auto_extracted() {
        let doc = "## Glossary\nCODE - Long Form of the event\nValue Date: the date funds become available\n\
                   | Code | Meaning |\n|---|---|\n| RFND | Refund |\n";
        assert!(extract_vocab_rules(doc).is_empty(), "glossary-shaped lines need a marked file");
    }

    #[test]
    fn glossary_file_parses_tables_csv_and_lines() {
        let doc = "# Glossary\n\n| Term | Meaning |\n|---|---|\n| **DD** | Direct debit |\n| RFND | Refund | extra |\n\n\
                   term,expansion\ndunning,\"payment reminder, before pause\"\nKYC\tknow your customer\n\
                   SLA - service level agreement\nValue Date: the date funds become available\n";
        let pairs = parse_glossary_file(doc);
        for want in [
            pair("dd", "direct debit"),
            pair("rfnd", "refund"),
            pair("dunning", "payment reminder, before pause"),
            pair("kyc", "know your customer"),
            pair("sla", "service level agreement"),
            pair("value date", "the date funds become available"),
        ] {
            assert!(pairs.contains(&want), "missing {want:?} in {pairs:?}");
        }
        assert!(!pairs.iter().any(|p| p.term == "term"), "header rows skipped: {pairs:?}");
        assert_eq!(pairs.len(), 6, "{pairs:?}");
    }

    #[test]
    fn store_replaces_item_rows_but_keeps_manual() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        add_manual(&conn, "dunning", "payment reminder").unwrap();
        store_vocab(&conn, a, 1, &[pair("dd", "direct debit")], &[pair("kyc", "know your customer")]).unwrap();
        store_vocab(&conn, a, 1, &[pair("dd", "direct debit")], &[]).unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT term, origin FROM vocabulary ORDER BY term")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(rows, [("dd".into(), "rule".into()), ("dunning".into(), "manual".into())]);
        let statuses: i64 = conn.query_row("SELECT COUNT(*) FROM vocabulary WHERE status != 'active'", [], |r| r.get(0)).unwrap();
        assert_eq!(statuses, 0, "no proposals exist");
    }

    #[test]
    fn manual_crud_validates_and_dedupes() {
        let (_dir, conn) = vocab_db();
        let e = add_manual(&conn, "  KYC ", "Know Your Customer").unwrap();
        assert_eq!((e.term.as_str(), e.expansion.as_str(), e.origin.as_str()), ("kyc", "know your customer", "manual"));
        assert!(add_manual(&conn, "kyc", "know your customer").unwrap_err().contains("already"));
        assert!(add_manual(&conn, "x", "y").is_err(), "too short");
        assert_eq!(list_glossary(&conn).len(), 1);
        assert!(delete_manual(&conn, e.id).unwrap());
        assert!(!delete_manual(&conn, e.id).unwrap(), "second delete finds nothing");
        // Marked-file rows are not deletable as manual entries.
        let a = add_item(&conn, 1, "g.csv");
        store_glossary_file(&conn, a, 1, &[pair("dd", "direct debit")]).unwrap();
        let gid = list_glossary(&conn)[0].id;
        assert!(!delete_manual(&conn, gid).unwrap());
        assert_eq!(list_glossary(&conn)[0].path.as_deref(), Some("g.csv"));
    }

    #[test]
    fn lookup_is_project_scoped_and_manual_is_global() {
        let (_dir, conn) = vocab_db();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s2','folder')", []).unwrap();
        let a = add_item(&conn, 1, "a.md");
        store_vocab(&conn, a, 1, &[], &[pair("dunning", "payment reminder")]).unwrap();
        add_manual(&conn, "kyc", "know your customer").unwrap();
        let terms = vec!["dunning".to_string(), "kyc".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &terms, None), ["payment reminder", "know your customer"]);
        let scoped: HashSet<i64> = [2].into_iter().collect();
        assert_eq!(vocab_expansion_terms(&conn, &terms, Some(&scoped)), ["know your customer"], "manual applies everywhere");
        let empty: HashSet<i64> = HashSet::new();
        assert!(vocab_expansion_terms(&conn, &terms, Some(&empty)).is_empty(), "empty scope matches nothing");
    }

    #[test]
    fn agent_mode_skips_rule_rows_but_keeps_glossary() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        store_vocab(&conn, a, 1, &[pair("dd", "direct debit")], &[pair("dunning", "payment reminder")]).unwrap();
        let terms = vec!["dd".to_string(), "dunning".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &terms, None).len(), 2);
        conn.execute("UPDATE app_settings SET value = 'agent' WHERE key = 'llm_mode'", []).unwrap();
        assert_eq!(vocab_expansion_terms(&conn, &terms, None), ["payment reminder"]);
    }

    #[test]
    fn multiword_expansion_fires_on_word_subset() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        store_vocab(&conn, a, 1, &[pair("dd", "direct debit")], &[]).unwrap();
        let both = vec!["direct".to_string(), "debit".to_string()];
        assert_eq!(vocab_expansion_terms(&conn, &both, None), vec!["dd".to_string()]);
        assert!(vocab_expansion_terms(&conn, &["direct".to_string()], None).is_empty());
        assert_eq!(vocab_expansion_terms(&conn, &["dd".to_string()], None), vec!["direct debit".to_string()]);
    }

    #[test]
    fn prune_keeps_people_rows_and_initialisms_only() {
        let (_dir, conn) = vocab_db();
        let a = add_item(&conn, 1, "a.md");
        for (t, e, origin, status) in [
            ("sla", "service level agreement", "rule", "active"),
            ("psa", "card account issuer payment agent", "rule", "active"),
            ("rfnd", "refund", "rule", "active"),
            ("ach", "automated clearing house", "llm", "proposed"),
            ("bogus", "bad guess", "llm", "rejected"),
            ("kyc", "know your customer", "manual", "active"),
        ] {
            conn.execute(
                "INSERT INTO vocabulary (term, expansion, item_id, source_id, origin, status) VALUES (?1,?2,?3,1,?4,?5)",
                rusqlite::params![t, e, a, origin, status],
            )
            .unwrap();
        }
        assert_eq!(prune_to_trusted(&conn).unwrap(), 4);
        let left: Vec<String> = conn
            .prepare("SELECT term FROM vocabulary ORDER BY term")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(left, ["kyc", "sla"]);
    }

    #[test]
    fn known_terms_reports_existing_rows() {
        let (_dir, conn) = vocab_db();
        add_manual(&conn, "kyc", "know your customer").unwrap();
        let got = known_terms(&conn, &["KYC".to_string(), "AML".to_string()]);
        assert_eq!(got, ["kyc".to_string()].into_iter().collect());
    }
}
