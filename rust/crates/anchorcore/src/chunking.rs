//! Prose/code/config splitters: fixed windows (`chunk_text`), heading sections
//! (`chunk_document_with_sections`), symbol sections (`chunk_code_with_sections`)
//! and key sections (`chunk_config_with_sections`).
//!
//! R36.7 prose structure rules: `#`–`######` markers (emitted by `extract.rs`
//! for docx `w:pStyle` / ODT `text:h` / HTML `h1`–`h6`, native in Markdown)
//! section the document; `§`/numbered/ALL-CAPS lines keep their legacy rules;
//! heading-less docs over one window fall back to paragraph windows; tables
//! split by row groups with the header repeated. Auto-generated tables of
//! contents are stripped at extraction, never indexed.

use std::sync::OnceLock;

use regex::Regex;

const CHUNK_SIZE: usize = 800;
const CHUNK_OVERLAP: usize = 100;
const CHUNK_MAX_CHARS: usize = 1600;
const CLASSIFY_WINDOW: usize = 16000;

fn env_usize(key: &str, fallback: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(fallback)
}

/// Shared sliding window over chars (Python str-len semantics, UTF-8 safe).
/// Powers `classify_windows` — `chunk_text` snaps to boundaries instead.
fn slide_windows(text: &str, size: usize, step: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size {
        return vec![text.to_string()];
    }
    let step = step.max(1);
    let max_start = chars.len().saturating_sub(size);
    let mut out = Vec::new();
    let mut i = 0;
    while i <= max_start {
        let end = (i + size).min(chars.len());
        out.push(chars[i..end].iter().collect());
        if end == chars.len() {
            break;
        }
        i += step;
    }
    // handle trailing remainder when max_start not reached due to step misalignment
    // Python: range(0, max(len-size+1,1), step) ensures last window may be partial
    // Our loop already covers that because we iterate until <= max_start.
    // Edge: if chars.len() % step != 0 the last chunk is already included as above.
    // Ensure we don't miss a final partial chunk when step >1 and remainder < size
    // Example len=801 size=800 step=700 -> max_start=1 -> i=0,700 => two chunks, second 700..801 partial 101 chars, covered.
    out
}

/// Fixed-size chunks with overlap — boundary-snapped: window ends pull back
/// to a sentence end, else a word break (bounded by the overlap so coverage
/// never gaps); window starts advance to the next word start (clamped to the
/// previous end). The single-window fast path returns the text untouched.
/// The archived Python `chunk_text` sliced blindly mid-word; Rust snaps so
/// entity summaries and oversized paragraphs stay whole-worded.
pub fn chunk_text(text: &str) -> Vec<String> {
    let size = env_usize("ANCHOR_CHUNK_SIZE", CHUNK_SIZE);
    let overlap = env_usize("ANCHOR_CHUNK_OVERLAP", CHUNK_OVERLAP);
    snap_windows(text, size, overlap)
}

fn snap_windows(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size || size == 0 {
        return vec![text.to_string()];
    }
    let step = size.saturating_sub(overlap).max(1);
    let max_shrink = overlap.min(size - 1);
    let mut out = Vec::new();
    let mut start = snap_start(&chars, 0);
    loop {
        let raw_end = (start + size).min(chars.len());
        // The tail window always covers to the end — never strand a suffix.
        let end = if raw_end == chars.len() { chars.len() } else { snap_end(&chars, raw_end, max_shrink) };
        let chunk = chars[start..end].iter().collect::<String>();
        let trimmed = chunk.trim_end();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
        if end == chars.len() {
            break;
        }
        start = snap_start(&chars, start + step).min(end);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Next word start at/after `p`: first non-space char that opens a word
/// (string start or preceded by whitespace). Falls back to `p` when the
/// rest is one partial word or trailing spaces (clamped/skipped by caller).
fn snap_start(chars: &[char], p: usize) -> usize {
    let mut i = p.min(chars.len());
    while i < chars.len() {
        if !chars[i].is_whitespace() && (i == 0 || chars[i - 1].is_whitespace()) {
            return i;
        }
        i += 1;
    }
    p.min(chars.len())
}

/// Pull a window end back to a sentence end, else a word break, searching
/// at most `max` chars back. Falls back to the raw end (hard cut) when no
/// boundary fits — long tokens still split rather than drop.
fn snap_end(chars: &[char], raw_end: usize, max: usize) -> usize {
    let lo = raw_end.saturating_sub(max);
    if let Some(end) = sentence_end_in(chars, lo, raw_end) {
        return end;
    }
    word_break_in(chars, lo, raw_end).unwrap_or(raw_end)
}

/// Last `[.?!]` followed by whitespace in `[lo, raw_end)` — snap after the
/// space run. Abbreviations and decimals ("U.S.A", "3.5") never match.
fn sentence_end_in(chars: &[char], lo: usize, raw_end: usize) -> Option<usize> {
    let mut i = raw_end;
    while i > lo {
        i -= 1;
        if matches!(chars[i], '.' | '!' | '?') {
            let mut j = i + 1;
            while j < raw_end && chars[j].is_whitespace() {
                j += 1;
            }
            if j > i + 1 {
                return Some(j);
            }
        }
    }
    None
}

/// Last whitespace in `[lo, raw_end)` — snap at it (caller's trim cleans).
fn word_break_in(chars: &[char], lo: usize, raw_end: usize) -> Option<usize> {
    let mut k = raw_end;
    while k > lo {
        k -= 1;
        if chars[k].is_whitespace() {
            return Some(k);
        }
    }
    None
}

/// Page-edge window (R43.2): repeated header/footer lines and page-number
/// lines strip only within the first/last N non-blank content lines of each
/// page segment. Headers/footers run 1–3 lines; anything deeper is content.
const PAGE_EDGE_LINES: usize = 3;

/// B12 document cleaning (archived `cleaning.py` parity, R43.2 page-boundary
/// fix): control chars and whitespace clean doc-wide, but page-number lines
/// and repeated header/footer lines strip ONLY at page boundaries — the old
/// doc-wide strip deleted real table content (`Yes`, `N/A`, years, amounts).
/// Page segments split on R36.10 `[page N]` marker lines; marker-less text is
/// one segment, so only doc-edge boilerplate strips. Markers themselves
/// always survive (the R36.10 breadcrumb plan reads them after chunking).
/// Applied to the document copy that feeds chunking — `ingested_items.text`
/// stays raw, exactly like the archived pipeline. Dropped boilerplate leaves
/// no gap (blank runs collapse across it).
pub fn clean_doc_text(text: &str) -> String {
    let repeated = repeated_lines(text);
    let nls = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = nls.split('\n').collect();
    let edges = page_edge_mask(&lines);
    let mut out: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let noctrl: String = line.chars().filter(|c| !is_stripped_control(*c)).collect();
        if noctrl.trim().is_empty() {
            blanks += 1;
            if blanks == 1 {
                out.push(String::new());
            }
            continue;
        }
        let trimmed = noctrl.trim();
        if crate::extract::page_marker_number(trimmed).is_none() && edges[i] {
            if is_page_number_line(trimmed) || repeated.contains(&repeat_key(trimmed)) {
                continue;
            }
        }
        blanks = 0;
        out.push(collapse_ws_rstrip(&noctrl));
    }
    out.join("\n").trim().to_string()
}

/// Edge mask (R43.2): line indices within the first/last `PAGE_EDGE_LINES`
/// non-blank lines of each page segment. Marker lines split segments but are
/// never edges themselves.
fn page_edge_mask(lines: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; lines.len()];
    let mut seg_start = 0;
    for i in 0..=lines.len() {
        let boundary = i == lines.len() || crate::extract::page_marker_number(lines[i].trim()).is_some();
        if boundary {
            mark_segment_edges(lines, &mut mask, seg_start, i);
            seg_start = i + 1;
        }
    }
    mask
}

fn mark_segment_edges(lines: &[&str], mask: &mut [bool], start: usize, end: usize) {
    let mut head = 0;
    for i in start..end {
        if lines[i].trim().is_empty() {
            continue;
        }
        if head >= PAGE_EDGE_LINES {
            break;
        }
        mask[i] = true;
        head += 1;
    }
    let mut tail = 0;
    for i in (start..end).rev() {
        if lines[i].trim().is_empty() {
            continue;
        }
        if tail >= PAGE_EDGE_LINES {
            break;
        }
        mask[i] = true;
        tail += 1;
    }
}

/// Control chars the cleaner strips (`\t`/`\n` survive; `\r` is normalised
/// to `\n` before this runs).
fn is_stripped_control(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}' | '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{feff}' | '\u{fffd}'
    )
}

/// Collapse `[ \t]` runs to one space, strip the right end (a single
/// leading space survives, like the archived cleaner).
fn collapse_ws_rstrip(line: &str) -> String {
    let mut collapsed = String::with_capacity(line.len());
    let mut in_ws = false;
    for c in line.chars() {
        if c == ' ' || c == '\t' {
            if !in_ws {
                collapsed.push(' ');
            }
            in_ws = true;
        } else {
            collapsed.push(c);
            in_ws = false;
        }
    }
    collapsed.trim_end().to_string()
}

/// Page-number lines: `42`, `Page 3`, `3 of 10`, `- 12 -`, `— 7 —`.
/// Anything else carrying digits (`Section 42`, `42 things`) is kept.
fn is_page_number_line(s: &str) -> bool {
    static RE_PAGE: OnceLock<Regex> = OnceLock::new();
    static RE_ORPHAN: OnceLock<Regex> = OnceLock::new();
    let re = RE_PAGE
        .get_or_init(|| Regex::new(r"(?i)^(?:page\s*)?\d{1,4}(?:\s*of\s*\d{1,4})?\s*$").unwrap());
    if re.is_match(s) {
        return true;
    }
    let orphan =
        RE_ORPHAN.get_or_init(|| Regex::new(r"^[-–—]\s*\d{1,4}\s*[-–—]$").unwrap());
    orphan.is_match(s)
}

/// Repeat-comparison key: trimmed, inner whitespace collapsed, lowercased —
/// counting and stripping share it, so a double-spaced header still matches
/// its cleaned form.
fn repeat_key(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Lines repeated across the document (headers/footers/templates): keys
/// at/above threshold, counted on the raw text like the archived pipeline.
/// Lines over 120 chars never count. R43.2 strips members only at page
/// boundaries (see `page_edge_mask`) — mid-page repeats are table content.
fn repeated_lines(text: &str) -> std::collections::HashSet<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut total = 0usize;
    for line in text.split('\n') {
        let s = repeat_key(line);
        if s.is_empty() || s.chars().count() > 120 {
            continue;
        }
        *counts.entry(s).or_insert(0) += 1;
        total += 1;
    }
    let threshold = repeat_threshold(total);
    counts.into_iter().filter(|(_, c)| *c >= threshold).map(|(s, _)| s).collect()
}

/// `max(3, round(total * 0.01))` with Python banker's rounding, in integer
/// math (exact — float `0.01` never enters).
fn repeat_threshold(total: usize) -> usize {
    let q = total / 100;
    let r = total % 100;
    let rounded = q + if r > 50 { 1 } else if r < 50 { 0 } else { q % 2 };
    rounded.max(3)
}

/// Parser-emitted heading marker (R36.7): `#`–`######` + space, shared by
/// `extract.rs` (docx `w:pStyle`, ODT `text:h`, HTML `h1`–`h6`) and native
/// Markdown. No length cap — a long extractor heading still sections (its
/// title truncates via `truncate_title`); `#nospace` and `#######` (7+) are
/// body text, never headings.
pub(crate) fn marker_heading(line: &str) -> Option<(i32, &str)> {
    let s = line.trim();
    let hashes = s.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    // `#` is one byte, so `hashes` is a safe byte index into the prefix run.
    let rest = s.get(hashes..)?.strip_prefix([' ', '\t'])?;
    let title = rest.trim();
    if title.is_empty() {
        return None;
    }
    Some((hashes as i32, title))
}

/// Heading kind — the prose splitter needs more than a level: ALL-CAPS lines
/// follow context rules (set-off vs run-in) that numbered/marker headings
/// don't. R43.3: `classify_heading` keeps its shape for the TOC-suppression
/// probe; the splitter uses `classify_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingKind {
    Marker,
    Numbered,
    Caps,
}

/// Single heading classifier — was `is_heading` + `heading_level` (+ 3 level
/// helpers), which evaluated the same regexes twice. Returns the section level
/// (`Some`) or `None` for body text. Level rules are unchanged: §-forms carry
/// dots (`§434`→1, `4.2.1`-style→1+dots), article/annex/section words and
/// ALL-CAPS lines are level 1. R36.7 checks `#` markers first (they bypass
/// the 80-char guard — parser structure, not heuristic guesswork).
fn classify_heading(line: &str) -> Option<i32> {
    classify_kind(line).map(|(_, level)| level)
}

fn classify_kind(line: &str) -> Option<(HeadingKind, i32)> {
    static RE_NUM: OnceLock<Regex> = OnceLock::new();
    static RE_CAPS: OnceLock<Regex> = OnceLock::new();
    if let Some((level, _)) = marker_heading(line) {
        return Some((HeadingKind::Marker, level));
    }
    let s = line.trim();
    if s.is_empty() || s.len() > 80 {
        return None;
    }
    // numbered markers §434, 4.2.1, Article 12
    let re = RE_NUM.get_or_init(|| {
        Regex::new(
            r"^\s*(?:§\s*\d+(\.\d+)*|\d{1,4}(\.\d{1,4}){1,3}|(?:article|annex|section|schedule|rule|appendix)\s+\d+)(?:\s|[:.)\-]|$)",
        )
        .unwrap()
    });
    // R43.3: dotted numerics that read as data, not structure, never section
    // (`3.5 million users`, `1.0.31 release`, `21.10.2026 notes`,
    // `10.0.0.1 gateway` used to open 1–2 line ghost sections).
    let numbered = re.is_match(s) && !looks_like_non_heading_number(s);
    let caps = RE_CAPS.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$").unwrap());
    let caps_hit =
        caps.is_match(s) && !s.chars().all(|c| c.is_ascii_digit()) && !is_short_shout(s);
    if !numbered && !caps_hit {
        return None;
    }
    let kind = if numbered {
        HeadingKind::Numbered
    } else {
        HeadingKind::Caps
    };
    Some((kind, numbered_heading_level(s)))
}

/// R43.3: short single-token shouts (`NOTE`, `TODO`, `FIXME`) are admonition
/// labels, never section headings — even set off by blanks. Multi-token and
/// longer single-token CAPS (`ACME CORP`, `INTRODUCTION`) still classify here;
/// the splitter's context rules decide those.
const CAPS_MIN_TOKEN: usize = 8;

fn is_short_shout(s: &str) -> bool {
    !s.contains([' ', '\t']) && s.chars().count() < CAPS_MIN_TOKEN
}

/// R43.3: dotted-numeric openers that read as data, not structure. §-forms
/// and article-word forms are distinctive enough to keep as-is; only
/// dot-separated digit runs are screened: dates, IPv4 addresses, bare
/// numbers with no title text, and quantities/versions whose remainder runs
/// on in lowercase prose (`3.5 million`, `1.0.31 release`).
fn looks_like_non_heading_number(s: &str) -> bool {
    if s.starts_with('§') {
        return false;
    }
    let lower = s.to_lowercase();
    for w in [
        "article ",
        "annex ",
        "section ",
        "schedule ",
        "rule ",
        "appendix ",
    ] {
        if lower.starts_with(w) {
            return false;
        }
    }
    let raw_len: usize = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .map(|c| c.len_utf8())
        .sum();
    let token = s[..raw_len].trim_end_matches('.');
    let rest = s[raw_len..]
        .trim_start_matches([' ', '\t', ':', ')', '-', '.'])
        .trim_start();
    if rest.is_empty() {
        return true;
    }
    let parts: Vec<&str> = token.split('.').collect();
    if is_date_token(&parts) || is_ipv4_token(&parts) {
        return true;
    }
    rest.chars().next().is_some_and(|c| c.is_lowercase())
}

fn is_date_token(parts: &[&str]) -> bool {
    if parts.len() != 3 {
        return false;
    }
    let ymd = parts[0].len() == 4 && parts[1].len() <= 2 && parts[2].len() <= 2;
    let dmy = parts[0].len() <= 2 && parts[1].len() <= 2 && parts[2].len() == 4;
    ymd || dmy
}

fn is_ipv4_token(parts: &[&str]) -> bool {
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u32>().is_ok_and(|n| n <= 255))
}

/// Level for a line already known to be a heading (see `classify_heading`).
fn numbered_heading_level(s: &str) -> i32 {
    if s.starts_with('§') {
        let after = s.trim_start_matches('§').trim_start();
        let prefix: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        return 1 + prefix.matches('.').count() as i32;
    }
    let lower = s.to_lowercase();
    if lower.starts_with("article ")
        || lower.starts_with("annex ")
        || lower.starts_with("section ")
        || lower.starts_with("schedule ")
        || lower.starts_with("rule ")
        || lower.starts_with("appendix ")
    {
        return 1;
    }
    if let Some(c) = s.chars().next() {
        if c.is_ascii_digit() {
            let token = s.split(&[' ', ':', ')', '-', '\t'][..]).next().unwrap_or("");
            let token = token.trim_matches(|c| c == '.' || c == ')' || c == ':');
            if token.contains('.') {
                return 1 + token.matches('.').count() as i32;
            }
        }
    }
    1
}

/// TOC ghost-section suppression (R39.9, sparse-hardened R40.3): PDF contents
/// pages (`8.1 Detection ... 23`) otherwise become near-empty ghost sections
/// next to the real ones, because the numbered/ALL-CAPS classifier
/// section-izes every entry line. Detection slides windows over the
/// non-blank line sequence (blank-delimited blocks miss sparse TOCs whose
/// entries are blank-separated — the common PDF-extraction shape): a window
/// flags when it holds at least `TOC_MIN_ENTRIES` page-numbered entries at
/// `>= 1/2` density within a local span. Flagged lines keep their bytes but
/// never open sections (chunked as normal prose); a `Contents`-style header
/// above a run is suppressed with it. Only would-be headings are ever
/// flagged (only they can ghost — wrapped continuations are harmless prose
/// either way). Real sections survive: their headings carry no trailing page
/// number, lone digit-ending headings never reach the threshold, and
/// scattered ones never share a local span.
const TOC_MIN_ENTRIES: usize = 3;
const TOC_MAX_WINDOW: usize = 8;

/// Pre-scan: one flag per line — `true` keeps the line's bytes but skips
/// section-izing it. Entry probes run once per non-blank line; windows of
/// 3..=8 consecutive non-blank lines flag local runs (subsumes the old
/// dense-block rule — a dense run is a window with span == size - 1).
fn toc_suppressed(lines: &[&str]) -> Vec<bool> {
    let mut out = vec![false; lines.len()];
    let nonblank: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let entry: Vec<bool> = nonblank.iter().map(|&i| toc_entry_candidate(lines[i])).collect();
    for size in TOC_MIN_ENTRIES..=TOC_MAX_WINDOW.min(nonblank.len()) {
        for start in 0..=nonblank.len() - size {
            let window = &nonblank[start..start + size];
            if toc_window_flagged(window, &entry[start..start + size]) {
                for (pos, &i) in window.iter().enumerate() {
                    if entry[start + pos] {
                        out[i] = true;
                        suppress_toc_title_above(&mut out, lines, i);
                    }
                }
            }
        }
    }
    out
}

/// Flagged when page-numbered would-be headings dominate a local window:
/// at least `TOC_MIN_ENTRIES` at `>= 1/2` density, spanning at most twice
/// the window (every-other-line blanks pass; scattered lines don't).
fn toc_window_flagged(window: &[usize], entry: &[bool]) -> bool {
    let Some(&first) = window.first() else {
        return false;
    };
    let Some(&last) = window.last() else {
        return false;
    };
    let entries = entry.iter().filter(|&&e| e).count();
    entries >= TOC_MIN_ENTRIES && entries * 2 >= window.len() && last - first <= 2 * window.len()
}

/// Suppress a `Contents`-style header on the first non-blank line above a
/// flagged entry (in-run lines above are entries, never titles — safe to
/// call per flagged line).
fn suppress_toc_title_above(out: &mut [bool], lines: &[&str], idx: usize) {
    let mut j = idx;
    while j > 0 {
        j -= 1;
        if lines[j].trim().is_empty() {
            continue;
        }
        if toc_title_line(lines[j]) {
            out[j] = true;
        }
        return;
    }
}

/// A contents entry: a would-be heading (`8.1 Detection`, `INTRODUCTION`)
/// plus a trailing page number (`23`, `... 23`). Dot leaders don't count
/// toward the length cap (extracted TOCs pad entries past 80 raw chars);
/// the classifier decides the cleaned prefix, so only lines that could
/// ghost ever count.
fn toc_entry_candidate(line: &str) -> bool {
    let s = line.trim();
    if s.is_empty() {
        return false;
    }
    let Some(rest) = strip_trailing_page_number(s) else {
        return false;
    };
    let entry = strip_dot_leaders(rest);
    if entry.is_empty() || entry.chars().count() > 80 {
        return false;
    }
    classify_heading(entry).is_some()
}

/// Drop dot/ellipsis leader runs off an entry tail — length and
/// classification read the words, not the padding.
fn strip_dot_leaders(s: &str) -> &str {
    s.trim_end_matches(|c: char| c == '.' || c == '·' || c == '…' || c.is_whitespace())
}

/// `Contents` / `Table of Contents` / `Index`, any case — the header above a
/// contents run. Only the ALL-CAPS forms could ghost; matching the rest is
/// harmless (they were body text anyway).
fn toc_title_line(line: &str) -> bool {
    matches!(
        line.trim().to_lowercase().as_str(),
        "contents" | "table of contents" | "index"
    )
}

/// Split `8.1 Detection ... 23` into `8.1 Detection ...`; bare numbers
/// (`42`), bare markers (`8.1`, `Article 12`) and 5+-digit tails never match.
fn strip_trailing_page_number(s: &str) -> Option<&str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(.*\S)\s+\d{1,4}$").unwrap());
    re.captures(s).and_then(|cap| cap.get(1)).map(|m| m.as_str())
}

pub fn chunk_document(text: &str) -> Vec<String> {
    chunk_document_with_sections(text).1.into_iter().map(|c| c.content).collect()
}

/// Hierarchical TOC structures for R14.1.
#[derive(Debug, Clone)]
pub struct SectionMeta {
    pub title: String,
    pub level: i32,
    pub parent: Option<usize>,
    pub path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub kind: String,
    pub meta: String,
}

#[derive(Debug, Clone)]
pub struct ChunkMeta {
    pub content: String,
    pub section_idx: Option<usize>,
    pub level: i32,
    pub path: String,
    pub line_start: i64,
    pub line_end: i64,
}

pub fn chunk_document_with_sections(text: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let lines: Vec<&str> = text.split('\n').collect();
    // R39.9: contents-page lines keep their bytes but never open sections.
    let suppressed = toc_suppressed(&lines);
    let mut raw_sections: Vec<RawSection> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let classified = if suppressed[idx] {
            None
        } else {
            classify_kind(line)
        };
        // R43.3: ALL-CAPS headings need set-off context (see below);
        // numbered/marker headings section unconditionally.
        let headed = match classified {
            Some((HeadingKind::Caps, level)) => {
                caps_section_level(&lines, idx, level, &raw_sections, &stack)
                    .map(|l| (Some(HeadingKind::Caps), l))
            }
            Some((kind, level)) => Some((Some(kind), level)),
            None => None,
        };
        if let Some((kind, level)) = headed {
            let title = section_title(line);
            // pop stack to find parent
            while let Some(&top) = stack.last() {
                if raw_sections[top].level >= level {
                    stack.pop();
                } else {
                    break;
                }
            }
            let parent = stack.last().copied();
            // R40.5: skip an empty parent segment (mirrors `push_symbol`) —
            // top-level paths used to start with `" > "` (" > 8.3 …"), which
            // rode into user-visible breadcrumbs and embed prefixes verbatim.
            let path = match parent.map(|p| raw_sections[p].path.clone()) {
                Some(pp) if !pp.is_empty() => format!("{pp} > {title}"),
                _ => title.clone(),
            };
            let ridx = raw_sections.len();
            raw_sections.push(RawSection {
                title,
                level,
                parent,
                path,
                lines: vec![line.to_string()],
                kind,
                line_start: idx as i64 + 1,
                line_end: 0,
            });
            stack.push(ridx);
        } else {
            if raw_sections.is_empty() {
                // root section for leading content before first heading
                let idx = raw_sections.len();
                raw_sections.push(RawSection {
                    title: String::new(),
                    level: 0,
                    parent: None,
                    path: String::new(),
                    lines: Vec::new(),
                    kind: None,
                    line_start: 1,
                    line_end: 0,
                });
                stack.push(idx);
            }
            if let Some(&top) = stack.last() {
                raw_sections[top].lines.push(line.to_string());
            }
        }
    }

    // if no sections (empty text), create empty root
    if raw_sections.is_empty() {
        raw_sections.push(RawSection {
            title: String::new(),
            level: 0,
            parent: None,
            path: String::new(),
            lines: Vec::new(),
            kind: None,
            line_start: 1,
            line_end: 0,
        });
    }

    // if raw_sections was root-only with empty title, keep section but chunks will map to it
    // filter empty metas? keep all for tree completeness (root with level 0 may be omitted if empty title and no parent)
    // Keep metas as is; pipeline will skip inserting empty-title root if it has no content? but we keep it for now.
    // R36.7: a root-only document has no headings — paragraph windows give
    // parent-return section units instead of one whole-file section.
    let raw_sections = if raw_sections.len() == 1 && raw_sections[0].title.is_empty() {
        fallback_windows(&raw_sections[0].lines.join("\n"))
    } else {
        raw_sections
    };
    materialize_opts(
        assign_line_ranges(raw_sections, lines.len() as i64),
        max_chars,
        true,
    )
}

/// R43.3: ALL-CAPS headings need set-off context — a run-in CAPS line inside
/// body text is an annotation, not a section (`ACME CORP` mid-paragraph used
/// to pop the whole numbered tree to level 1). The following line must be
/// blank/missing or short (a subtitle, not body prose); inside a numbered
/// section without a preceding blank line, the CAPS line nests under its
/// parent instead of popping to level 1. Set-off CAPS (`\n\nAPPENDIX\n\n`)
/// still opens a level-1 section.
const CAPS_FOLLOW_SHORT: usize = 60;

fn caps_section_level(
    lines: &[&str],
    idx: usize,
    level: i32,
    raw_sections: &[RawSection],
    stack: &[usize],
) -> Option<i32> {
    let followed = lines.get(idx + 1).map(|l| l.trim()).unwrap_or("");
    if !followed.is_empty() && followed.chars().count() > CAPS_FOLLOW_SHORT {
        return None;
    }
    let numbered_parent_level = stack
        .last()
        .and_then(|&t| raw_sections.get(t))
        .filter(|rs| rs.kind == Some(HeadingKind::Numbered))
        .map(|rs| rs.level);
    if let Some(parent_level) = numbered_parent_level {
        let set_off = idx == 0 || lines[idx - 1].trim().is_empty();
        if !set_off {
            return Some(parent_level + 1);
        }
    }
    Some(level)
}

/// Section title for a heading line: marker headings (`# Title`) drop the
/// markers (breadcrumbs read naturally; the content keeps the marker line),
/// legacy `§`/numbered/ALL-CAPS headings keep today's verbatim title.
fn section_title(line: &str) -> String {
    if let Some((_, title)) = marker_heading(line) {
        return truncate_title(title);
    }
    line.trim().to_string()
}

/// Heading-less fallback (R36.7): paragraph-aligned sliding windows, each
/// ~2–4 KB with ~0.5 KB of shared paragraphs, deterministic. Docs under one
/// window keep today's single root; windows carry empty titles — unit
/// scaffolding for parent-return, not fabricated headings.
const FALLBACK_WINDOW_MIN: usize = 2000;
const FALLBACK_WINDOW_MAX: usize = 4000;
const FALLBACK_OVERLAP: usize = 500;

fn fallback_windows(text: &str) -> Vec<RawSection> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\n\s*\n").unwrap());
    let paras: Vec<String> = re
        .split(text)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let total: usize = paras.iter().map(|p| p.chars().count() + 2).sum();
    if paras.len() < 2 || total <= FALLBACK_WINDOW_MAX {
        return vec![root_section(text)];
    }
    // R36.18a: paragraph line offsets (single-blank separators — the root
    // text is cleaned, so blank runs are uniform and each para advances
    // past exactly one blank line).
    let mut para_starts = Vec::with_capacity(paras.len());
    let mut next = 1i64;
    for p in &paras {
        para_starts.push(next);
        next += p.split('\n').count() as i64 + 1;
    }
    let mut raw = Vec::new();
    let mut s = 0;
    while s < paras.len() {
        let e = window_end(&paras, s);
        raw.push(window_section(&paras[s..e], para_starts[s]));
        if e >= paras.len() {
            break;
        }
        s = overlap_start(&paras, s, e);
    }
    raw
}

/// Grow a window from `s`: stop before exceeding MAX once MIN is held; the
/// first paragraph is always taken, so a lone giant paragraph becomes a
/// single oversized window (hard-split later, never dropped).
fn window_end(paras: &[String], s: usize) -> usize {
    let mut size = 0usize;
    let mut e = s;
    while e < paras.len() {
        let next = paras[e].chars().count() + 2;
        if e > s && size >= FALLBACK_WINDOW_MIN && size + next > FALLBACK_WINDOW_MAX {
            break;
        }
        size += next;
        e += 1;
    }
    e.max(s + 1)
}

/// Next window starts strictly after `s`, sharing ~0.5 KB of trailing
/// paragraphs; a single-paragraph window advances without overlap.
fn overlap_start(paras: &[String], s: usize, e: usize) -> usize {
    let mut j = e;
    let mut shared = 0usize;
    while j > s + 1 && shared < FALLBACK_OVERLAP {
        j -= 1;
        shared += paras[j].chars().count() + 2;
    }
    j.min(e.saturating_sub(1)).max(s + 1)
}

fn window_section(paras: &[String], start_line: i64) -> RawSection {
    let mut lines = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(p.split('\n').map(|l| l.to_string()));
    }
    // R36.18a: windows preset their exact span (they overlap by design, so
    // the post-pass end rule would understate them — see its skip below).
    let line_end = start_line + lines.len() as i64 - 1;
    RawSection {
        title: String::new(),
        level: 0,
        parent: None,
        path: String::new(),
        lines,
        kind: None,
        line_start: start_line,
        line_end: line_end.max(start_line),
    }
}

/// R36.18a: section-granular line ranges (1-based, inclusive, relative to
/// the chunked text — cleaned text for prose, raw for code/config). A
/// section runs from its first line to the line before the next
/// same-or-shallower section starts (or EOF): parents span their children,
/// level-0 roots end at whatever section follows. Sections with a preset
/// end (fallback windows) keep it. Chunks inherit their section's range
/// in `materialize_opts` — the same granularity as markdown's split pieces.
fn assign_line_ranges(mut raw: Vec<RawSection>, total_lines: i64) -> Vec<RawSection> {
    for i in 0..raw.len() {
        if raw[i].line_end != 0 {
            continue;
        }
        let level = raw[i].level;
        let mut end = total_lines;
        for rs in raw.iter().skip(i + 1) {
            if rs.level <= level || level == 0 {
                end = rs.line_start - 1;
                break;
            }
        }
        raw[i].line_end = end.max(raw[i].line_start);
    }
    raw
}

fn total_lines(text: &str) -> i64 {
    text.split('\n').count().max(1) as i64
}

/// Table shape inside a paragraph block (R36.7): Markdown pipe rows or the
/// tab-separated rows `extract.rs` emits for docx/odt/HTML tables.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TableKind {
    Markdown,
    Tsv,
}

/// A paragraph is a table only when it holds 2+ non-blank lines and EVERY
/// line is the same row kind — one caption line degrades the block to plain
/// text (kept whole if it fits, hard-split if not).
fn table_kind(para: &str) -> Option<TableKind> {
    let lines: Vec<&str> = para.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }
    if lines.iter().all(|l| is_md_row(l)) {
        return Some(TableKind::Markdown);
    }
    if lines.iter().all(|l| is_tsv_row(l)) {
        return Some(TableKind::Tsv);
    }
    None
}

fn is_md_row(line: &str) -> bool {
    let t = line.trim();
    // `|` is one byte, so byte index 1 is the char after it.
    t.starts_with('|') && t.len() > 1 && t[1..].contains('|')
}

fn is_tsv_row(line: &str) -> bool {
    // trimmed: leading-indent tabs (code) never count, interior ones do.
    line.trim().contains('\t')
}

fn is_md_delimiter(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t'))
}

/// Split an over-cap table into row groups, repeating the header block (the
/// header row plus the Markdown delimiter row, if any) on every unit — an
/// unheadered row group is unanswerable. Degenerate shapes (a header or a
/// single row past the cap alone) hard-split instead; content is never cut.
fn split_table(para: &str, kind: TableKind, max_chars: usize) -> Vec<String> {
    let rows: Vec<&str> = para.lines().filter(|l| !l.trim().is_empty()).collect();
    let header: Vec<&str> = rows[..table_header_len(&rows, kind)].to_vec();
    let header_len: usize = header.iter().map(|r| r.chars().count() + 1).sum();
    if header_len > max_chars {
        return chunk_text(para);
    }
    let mut out = Vec::new();
    let mut group: Vec<&str> = Vec::new();
    let mut group_len = header_len;
    for row in &rows[header.len()..] {
        let row_len = row.chars().count() + 1;
        if !group.is_empty() && group_len + row_len > max_chars {
            out.push(join_rows(&header, &group));
            group.clear();
            group_len = header_len;
        }
        if group.is_empty() && header_len + row_len > max_chars {
            out.extend(chunk_text(row));
            continue;
        }
        group.push(row);
        group_len += row_len;
    }
    if !group.is_empty() {
        out.push(join_rows(&header, &group));
    }
    if out.is_empty() {
        out.push(para.to_string());
    }
    out
}

fn table_header_len(rows: &[&str], kind: TableKind) -> usize {
    if kind == TableKind::Markdown && rows.len() >= 2 && is_md_delimiter(rows[1]) {
        2
    } else {
        1
    }
}

fn join_rows(header: &[&str], group: &[&str]) -> String {
    header
        .iter()
        .chain(group.iter())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_oversized_para(para: &str, max_chars: usize, tables: bool) -> Vec<String> {
    if tables {
        if let Some(kind) = table_kind(para) {
            return split_table(para, kind, max_chars);
        }
    }
    chunk_text(para)
}

fn split_section(section: &str, max_chars: usize) -> Vec<String> {
    split_section_opts(section, max_chars, false)
}

/// Paragraph packing with an R36.7 prose-only table rule: an over-cap table
/// splits by row groups (header repeated) instead of mid-row. Code/config
/// keep `tables = false` — tab-indented code must never read as a table.
///
/// R43.4: paragraph edge trim that preserves indentation (the old `trim()`
/// stripped the first line's indent per paragraph — Python/YAML symbols over
/// the cap lost nesting). Blank edge lines drop; content lines keep leading
/// whitespace and lose trailing whitespace.
fn trim_para_edges(p: &str) -> String {
    let lines: Vec<&str> = p.lines().collect();
    let mut start = 0;
    while start < lines.len() && lines[start].trim().is_empty() {
        start += 1;
    }
    let mut end = lines.len();
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines[start..end]
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}
fn split_section_opts(section: &str, max_chars: usize, tables: bool) -> Vec<String> {
    static RE_SPLIT: OnceLock<Regex> = OnceLock::new();
    if section.chars().count() <= max_chars {
        return vec![section.to_string()];
    }
    let re = RE_SPLIT.get_or_init(|| Regex::new(r"\n\s*\n").unwrap());
    let paras: Vec<String> = re
        .split(section)
        .map(trim_para_edges)
        .filter(|p| !p.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in paras {
        if para.chars().count() > max_chars {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
            out.extend(split_oversized_para(&para, max_chars, tables));
        } else if cur.chars().count() + para.chars().count() + 2 <= max_chars {
            if cur.is_empty() {
                cur = para;
            } else {
                cur = format!("{}\n\n{}", cur, para);
            }
        } else {
            if !cur.is_empty() {
                out.push(cur.clone());
            }
            cur = para;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Intermediate section under construction — shared by the prose heading
/// splitter and the code symbol splitter.
struct RawSection {
    title: String,
    level: i32,
    parent: Option<usize>,
    path: String,
    lines: Vec<String>,
    /// R43.3: prose-splitter heading kind (drives the CAPS context rules).
    /// None for synthetic roots, fallback windows, and code/config sections.
    kind: Option<HeadingKind>,
    /// R36.18a: 1-based inclusive source lines (cleaned text for prose,
    /// raw for code/config). Starts are set at creation; ends are filled
    /// by `assign_line_ranges` (windows preset both).
    line_start: i64,
    line_end: i64,
}

/// R43.3: prose minimum chunk size (mirrors markdown `merge_short`/`MD_MIN`).
/// A chunk under this merges into its section-predecessor instead of
/// standing alone — tiny chunks win BM25 length normalization for free.
/// Applied on the prose path only (see `materialize_opts`): code symbols
/// and config keys are meaningful units at any size, and merging them
/// would corrupt symbol/key citations.
const PROSE_MIN: usize = 150;

fn merge_short_parts(parts: &mut Vec<String>) {
    let mut i = 1;
    while i < parts.len() {
        if parts[i].chars().count() < PROSE_MIN {
            let text = parts.remove(i);
            parts[i - 1].push_str("\n\n");
            parts[i - 1].push_str(&text);
        } else {
            i += 1;
        }
    }
}

/// Shared tail: section rows + paragraph-packed chunks per section.
fn materialize(raw_sections: Vec<RawSection>, max_chars: usize) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    materialize_opts(raw_sections, max_chars, false)
}

/// Shared tail with the R36.7 prose table rule (`tables = true` only from
/// the prose heading splitter — symbol/key splitters keep `false`).
fn materialize_opts(
    raw_sections: Vec<RawSection>,
    max_chars: usize,
    tables: bool,
) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let metas: Vec<SectionMeta> = raw_sections
        .iter()
        .map(|r| SectionMeta {
            title: r.title.clone(),
            level: r.level,
            parent: r.parent,
            path: r.path.clone(),
            // R36.18a: prose/code/config ride the same range columns as
            // markdown (migration 19); 0/0 survives only for old rows.
            line_start: r.line_start,
            line_end: r.line_end,
            kind: "section".to_string(),
            meta: "{}".to_string(),
        })
        .collect();
    let mut chunks: Vec<ChunkMeta> = Vec::new();
    for (idx, rs) in raw_sections.iter().enumerate() {
        let text = rs.lines.join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let mut parts = split_section_opts(&text, max_chars, tables);
        if tables {
            merge_short_parts(&mut parts);
        }
        for p in parts {
            chunks.push(ChunkMeta {
                content: p,
                section_idx: Some(idx),
                level: rs.level,
                path: rs.path.clone(),
                line_start: rs.line_start,
                line_end: rs.line_end,
            });
        }
    }
    (metas, chunks)
}

fn truncate_title(title: &str) -> String {
    let t = title.trim();
    if t.chars().count() <= 80 {
        return t.to_string();
    }
    format!("{}...", t.chars().take(77).collect::<String>())
}

fn root_section(text: &str) -> RawSection {
    RawSection {
        title: String::new(),
        level: 0,
        parent: None,
        path: String::new(),
        lines: text.split('\n').map(|l| l.to_string()).collect(),
        kind: None,
        line_start: 1,
        line_end: 0,
    }
}

/// Append a symbol section. The parent link skips the empty preamble root so
/// top-level paths never start with `" > "`.
fn push_symbol(
    raw: &mut Vec<RawSection>,
    parent: Option<usize>,
    title: String,
    level: i32,
    first_line: &str,
    line_no: i64,
) -> usize {
    let parent = parent.filter(|&p| !raw[p].title.is_empty());
    let path = match parent.map(|p| raw[p].path.clone()) {
        Some(pp) if !pp.is_empty() => format!("{pp} > {title}"),
        _ => title.clone(),
    };
    let idx = raw.len();
    raw.push(RawSection {
        title,
        level,
        parent,
        path,
        lines: vec![first_line.to_string()],
        kind: None,
        line_start: line_no,
        line_end: 0,
    });
    idx
}

/// Non-symbol line: opens a preamble root on first use, else appends to the
/// innermost open symbol. Stack keys are indent (python, yaml/json keys) or
/// brace depth.
fn append_code_line(raw: &mut Vec<RawSection>, stack: &mut Vec<(i64, usize)>, line: &str) {
    if raw.is_empty() {
        raw.push(RawSection {
            title: String::new(),
            level: 0,
            parent: None,
            path: String::new(),
            lines: Vec::new(),
            kind: None,
            line_start: 1,
            line_end: 0,
        });
        stack.push((-1, 0));
    }
    let top = stack.last().map(|&(_, idx)| idx).unwrap_or(0);
    raw[top].lines.push(line.to_string());
}

/// Nesting level, capped at 2 — methods nest under types, deeper nesting
/// keeps level 2 with a truthful parent link.
fn symbol_level(raw: &[RawSection], parent: Option<usize>) -> i32 {
    parent.map(|p| (raw[p].level + 1).min(2)).unwrap_or(1)
}

/// Symbol-splitter families — subsets of the extensions `extract.rs` types
/// as `code`. Anything else falls back to one root section (today's behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeFamily {
    Python,
    Brace,
    Fallback,
}

fn file_ext(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename).to_ascii_lowercase();
    match base.rsplit('.').next() {
        Some(e) if e != base.as_str() => e.to_string(),
        _ => String::new(),
    }
}

fn code_family(filename: &str) -> CodeFamily {
    match file_ext(filename).as_str() {
        "py" | "pyi" | "rb" => CodeFamily::Python,
        "rs" | "go" | "java" | "kt" | "kts" | "scala" | "groovy" | "cs" | "fs" | "fsx" | "js"
        | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "vue" | "svelte" | "astro" | "c" | "h" | "cpp"
        | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "swift" | "dart" | "php" | "m" | "mm" => CodeFamily::Brace,
        _ => CodeFamily::Fallback,
    }
}

/// `const name = ... =>` definitions only exist in JS-like languages —
/// everywhere else that shape is an assignment, not a symbol.
fn is_js_like(filename: &str) -> bool {
    matches!(
        file_ext(filename).as_str(),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "vue" | "svelte" | "astro"
    )
}

/// `type name(` definitions (Java methods, C functions) — gated by extension
/// so prose-like lines elsewhere can never match.
fn is_c_style(filename: &str) -> bool {
    matches!(
        file_ext(filename).as_str(),
        "java" | "cs" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx"
    )
}

/// Minified/generated files: symbol splitting would produce one giant
/// pseudo-symbol — keep them as a single root section instead.
fn is_minified(text: &str) -> bool {
    let newlines = text.bytes().filter(|b| *b == b'\n').count();
    if text.len() > 50_000 && newlines < 10 {
        return true;
    }
    text.len() / newlines.max(1) > 2000
}

/// Pipeline entry: configs split by key/table, markdown by blocks + `#`
/// headings, code by symbols, the rest by headings. The extension is ground
/// truth for the format — a `.yml` file's structure is keys and a `.md`
/// file's is blocks whatever their doc_type.
pub fn chunk_for_doc_type(
    doc_type: &str,
    filename: &str,
    text: &str,
) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    if crate::extract::is_config_filename(filename) {
        chunk_config_with_sections(text, filename)
    } else if crate::markdown::is_markdown_filename(filename) {
        crate::markdown::chunk_markdown(text, filename)
    } else if doc_type == "code" {
        chunk_code_with_sections(text, filename)
    } else {
        chunk_document_with_sections(text)
    }
}

/// Config entry (R33.4): YAML keeps its key splitter; JSON/JSONC nest object
/// keys the same way; TOML splits tables + keys. Minified single-line files
/// stay root-only instead of producing garbage sections.
pub fn chunk_config_with_sections(text: &str, filename: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let ext = file_ext(filename);
    if ext.as_str() == "yaml" || ext.as_str() == "yml" {
        return chunk_yaml_with_sections(text);
    }
    let raw = if is_minified(text) {
        vec![root_section(text)]
    } else {
        match ext.as_str() {
            "json" | "jsonc" => split_json_sections(text),
            "toml" => split_toml_sections(text),
            _ => vec![root_section(text)],
        }
    };
    materialize(assign_line_ranges(raw, total_lines(text)), max_chars)
}

pub fn chunk_yaml_with_sections(text: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    materialize(
        assign_line_ranges(split_yaml_sections(text), total_lines(text)),
        max_chars,
    )
}

/// YAML key splitter: every mapping key opens a section (indent-nested like
/// the python family), so a 68 KB routing-config.yml becomes per-key
/// sections with `treasury > acme` breadcrumbs instead of one root blob.
/// List items and comments stay in the current section; files with no keys
/// degrade to a single root, never to an error.
fn split_yaml_sections(text: &str) -> Vec<RawSection> {
    let mut raw: Vec<RawSection> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    let mut symbols = 0usize;
    for (li, line) in text.split('\n').enumerate() {
        let found = if symbols >= 500 { None } else { detect_yaml_key(line) };
        match found {
            Some((indent, title)) => {
                symbols += 1;
                while stack.last().map(|&(k, _)| k >= indent).unwrap_or(false) {
                    stack.pop();
                }
                let parent = stack.last().map(|&(_, idx)| idx);
                let level = symbol_level(&raw, parent);
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line, li as i64 + 1);
                stack.push((indent, idx));
            }
            None => append_code_line(&mut raw, &mut stack, line),
        }
    }
    raw
}

/// A YAML mapping key (`name:`) at any indent. List items, comments and doc
/// markers never open sections; the colon must be followed by space/EOL so
/// `http://x` and `12:30` can't match. Detection-only — content intact.
fn detect_yaml_key(line: &str) -> Option<(i64, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let indent = (line.len() - line.trim_start_matches([' ', '\t']).len()) as i64;
    let probe = line.trim_start_matches([' ', '\t']);
    if probe.is_empty() || probe.starts_with('#') || probe.starts_with("---") || probe.starts_with("...") {
        return None;
    }
    if probe == "-" || probe.starts_with("- ") || probe.starts_with("-\t") {
        return None;
    }
    let re = RE.get_or_init(|| {
        Regex::new(r#"^(?:"([^"\n]+)"|'([^'\n]+)'|([A-Za-z0-9_][A-Za-z0-9_.\-/]*))\s*:(?:\s|$)"#).unwrap()
    });
    let cap = re.captures(probe)?;
    let key = cap.get(1).or(cap.get(2)).or(cap.get(3))?.as_str();
    Some((indent, key.to_string()))
}

/// JSON key splitter (R33.4): every object key opens a section (indent-nested
/// like YAML), so nested objects get `server > tls` breadcrumbs. Array items
/// and JSONC comments stay in the current section, never open one.
fn split_json_sections(text: &str) -> Vec<RawSection> {
    let mut raw: Vec<RawSection> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    let mut symbols = 0usize;
    let mut in_block = false;
    for (li, line) in text.split('\n').enumerate() {
        let probe = strip_jsonc_comment(line, &mut in_block);
        let found = if symbols >= 500 { None } else { detect_json_key(&probe) };
        match found {
            Some((indent, title)) => {
                symbols += 1;
                while stack.last().map(|&(k, _)| k >= indent).unwrap_or(false) {
                    stack.pop();
                }
                let parent = stack.last().map(|&(_, idx)| idx);
                let level = symbol_level(&raw, parent);
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line, li as i64 + 1);
                stack.push((indent, idx));
            }
            None => append_code_line(&mut raw, &mut stack, line),
        }
    }
    raw
}

/// A JSON object key (`"name":`) at any indent. Runs on the JSONC-stripped
/// probe; the section keeps the original line.
fn detect_json_key(probe: &str) -> Option<(i64, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"^(\s*)"((?:[^"\\]|\\.)*)"\s*:"#).unwrap());
    let cap = re.captures(probe)?;
    let key = cap[2].replace("\\\"", "\"").replace("\\\\", "\\");
    if key.is_empty() {
        return None;
    }
    Some((cap[1].len() as i64, key))
}

/// JSONC detection surface: `//` to end of line plus `/* */` (inline and
/// multi-line via `in_block`) removed. Content keeps the original lines.
/// String-unaware on purpose: truncation only removes matches (a line folds),
/// never opens a false section — stripping text can't invent a `"key":` pair.
fn strip_jsonc_comment(line: &str, in_block: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if *in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block = false;
            }
            continue;
        }
        if c == '/' {
            match chars.peek() {
                Some('/') => break,
                Some('*') => {
                    chars.next();
                    *in_block = true;
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// TOML splitter (R33.4): tables open flat sections under their full dotted
/// name (`[a.b]` is its own breadcrumb, not a child — TOML tables don't nest)
/// and every key opens a sub-section, so values get `server > host` paths.
/// Preamble keys before the first table are top-level sections.
fn split_toml_sections(text: &str) -> Vec<RawSection> {
    static RE_TABLE: OnceLock<Regex> = OnceLock::new();
    static RE_KEY: OnceLock<Regex> = OnceLock::new();
    let re_table = RE_TABLE.get_or_init(|| Regex::new(r"^\s*\[{1,2}\s*([^\]#]+?)\s*\]{1,2}\s*(?:#.*)?$").unwrap());
    let re_key = RE_KEY.get_or_init(|| Regex::new(r"^\s*([^#\[=\s][^=]*?)\s*=").unwrap());
    let mut raw: Vec<RawSection> = Vec::new();
    let mut table: Option<usize> = None;
    let mut current: Option<usize> = None;
    let mut symbols = 0usize;
    for (li, line) in text.split('\n').enumerate() {
        if symbols < 500 {
            if let Some(name) = re_table.captures(line).map(|cap| cap[1].trim().to_string()) {
                if !name.is_empty() {
                    symbols += 1;
                    let idx = push_symbol(&mut raw, None, truncate_title(&name), 1, line, li as i64 + 1);
                    table = Some(idx);
                    current = Some(idx);
                    continue;
                }
            }
            if let Some(key) = re_key.captures(line).map(|cap| unquote_toml_key(cap[1].trim())) {
                if !key.is_empty() {
                    symbols += 1;
                    let level = symbol_level(&raw, table);
                    let idx = push_symbol(&mut raw, table, truncate_title(&key), level, line, li as i64 + 1);
                    current = Some(idx);
                    continue;
                }
            }
        }
        // fold: preamble root until the first section, else the current one
        // (covers comments, blank lines, and multi-line value continuations).
        if raw.is_empty() {
            raw.push(RawSection {
                title: String::new(),
                level: 0,
                parent: None,
                path: String::new(),
                lines: Vec::new(),
                line_start: 1,
                line_end: 0,
                kind: None,
            });
        }
        let top = current.unwrap_or(0);
        raw[top].lines.push(line.to_string());
    }
    raw
}

/// TOML keys may be bare, dotted, or quoted — strip one matching quote pair.
fn unquote_toml_key(key: &str) -> String {
    let b = key.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        key[1..key.len() - 1].to_string()
    } else {
        key.to_string()
    }
}

pub fn chunk_code_with_sections(text: &str, filename: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let raw = if is_minified(text) {
        vec![root_section(text)]
    } else {
        match code_family(filename) {
            CodeFamily::Python => split_python_symbols(text),
            CodeFamily::Brace => split_brace_symbols(text, filename),
            CodeFamily::Fallback => vec![root_section(text)],
        }
    };
    materialize(assign_line_ranges(raw, total_lines(text)), max_chars)
}

fn split_python_symbols(text: &str) -> Vec<RawSection> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^(\s*)(?:async\s+)?(class|def)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
    });
    let mut raw: Vec<RawSection> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    for (li, line) in text.split('\n').enumerate() {
        // `#` starts a comment outside strings; detection-only, content intact
        let probe = line.split('#').next().unwrap_or("");
        let found = re
            .captures(probe)
            .map(|cap| (cap[1].len() as i64, format!("{} {}", &cap[2], &cap[3])));
        match found {
            Some((indent, title)) => {
                while stack.last().map(|&(k, _)| k >= indent).unwrap_or(false) {
                    stack.pop();
                }
                let parent = stack.last().map(|&(_, idx)| idx);
                let level = symbol_level(&raw, parent);
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line, li as i64 + 1);
                stack.push((indent, idx));
            }
            None => append_code_line(&mut raw, &mut stack, line),
        }
    }
    raw
}

fn split_brace_symbols(text: &str, filename: &str) -> Vec<RawSection> {
    let js_like = is_js_like(filename);
    let c_style = is_c_style(filename);
    let mut raw: Vec<RawSection> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    let mut depth: i64 = 0;
    let mut symbols = 0usize;
    for (li, line) in text.split('\n').enumerate() {
        let probe = strip_brace_comment(line);
        let found = if symbols >= 500 {
            None
        } else {
            detect_brace_symbol(&probe, js_like, c_style)
        };
        match found {
            Some(title) => {
                symbols += 1;
                while stack.last().map(|&(k, _)| k >= depth).unwrap_or(false) {
                    stack.pop();
                }
                let parent = stack.last().map(|&(_, idx)| idx);
                let level = symbol_level(&raw, parent);
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line, li as i64 + 1);
                stack.push((depth, idx));
            }
            None => append_code_line(&mut raw, &mut stack, line),
        }
        depth = (depth + brace_delta(&probe)).max(0);
    }
    raw
}

fn detect_brace_symbol(probe: &str, js_like: bool, c_style: bool) -> Option<String> {
    detect_keyword_symbol(probe)
        .or_else(|| js_like.then(|| detect_js_const(probe)).flatten())
        .or_else(|| c_style.then(|| detect_c_style_fn(probe)).flatten())
}

fn detect_keyword_symbol(probe: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+|export\s+(?:default\s+)?|async\s+|static\s+|public\s+|private\s+|protected\s+|internal\s+|abstract\s+|final\s+|override\s+|inline\s+)*(fn|function|func|fun|def|class|struct|enum|trait|type|interface|impl)\b(.*)$").unwrap()
    });
    let cap = re.captures(probe)?;
    let keyword = cap[1].to_string();
    let rest = cap[2].trim();
    // a definition carries `(`, `{` or `;` — anything else is a bare mention
    if !rest.contains(['(', '{', ';']) {
        return None;
    }
    let name = brace_symbol_name(&keyword, rest);
    Some(if name.is_empty() {
        keyword
    } else {
        format!("{keyword} {name}")
    })
}

/// Name following the keyword: skips generics (`impl<T> Foo`) and Go
/// receivers (`func (s *Srv) Start` → `Srv.Start`).
fn brace_symbol_name(keyword: &str, rest: &str) -> String {
    let s = strip_generic_prefix(rest.trim_start());
    if keyword == "func" {
        if let Some(after) = strip_receiver(s) {
            let name = take_ident(after.trim_start());
            if name.is_empty() {
                return String::new();
            }
            return match receiver_name(s) {
                Some(recv) => format!("{recv}.{name}"),
                None => name,
            };
        }
    }
    take_ident(s)
}

fn strip_generic_prefix(s: &str) -> &str {
    let t = s.trim_start();
    if !t.starts_with('<') {
        return t;
    }
    t.find('>').map(|i| t[i + 1..].trim_start()).unwrap_or(t)
}

fn strip_receiver(s: &str) -> Option<&str> {
    let t = s.trim_start();
    if !t.starts_with('(') {
        return None;
    }
    t.find(')').map(|i| &t[i + 1..])
}

fn receiver_name(s: &str) -> Option<String> {
    let inner = s.trim_start().strip_prefix('(')?.split(')').next()?;
    let last = inner.split_whitespace().last()?;
    Some(last.trim_start_matches(['*', '&']).to_string())
}

fn take_ident(s: &str) -> String {
    s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
}

fn detect_js_const(probe: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=(.*)$").unwrap()
    });
    let cap = re.captures(probe)?;
    let rhs = cap[2].trim_start();
    // values are not symbols — only arrows and function expressions
    if !rhs.contains("=>") && !rhs.starts_with("function") {
        return None;
    }
    Some(format!("const {}", &cap[1]))
}

/// `type name(` definitions for Java/C-shaped languages. Calls, control flow
/// and forward declarations can never match (see guards).
fn detect_c_style_fn(probe: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^\s*(?:(?:public|private|protected|static|final|inline|extern|virtual|override|async|sealed)\s+)*([\w<>\[\],\s:*&]+?)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
    });
    let first = take_ident(probe.trim_start());
    if CONTROL_WORDS.contains(&first.as_str()) {
        return None;
    }
    let cap = re.captures(probe)?;
    if !probe.contains('{') && probe.contains(';') {
        return None;
    }
    Some(cap[2].to_string())
}

const CONTROL_WORDS: &[&str] = &[
    "if", "for", "while", "switch", "return", "catch", "foreach", "using", "lock", "else", "do", "case",
];

/// Detection surface: same-line `//` and `/* */` comments removed (content
/// keeps the original lines).
fn strip_brace_comment(line: &str) -> String {
    static RE_BLOCK: OnceLock<Regex> = OnceLock::new();
    let re = RE_BLOCK.get_or_init(|| Regex::new(r"/\*.*?\*/").unwrap());
    re.replace_all(line, "").split("//").next().unwrap_or("").to_string()
}

fn brace_delta(probe: &str) -> i64 {
    probe
        .bytes()
        .map(|b| match b {
            b'{' => 1,
            b'}' => -1,
            _ => 0,
        })
        .sum()
}

pub fn classify_windows(text: &str) -> Vec<String> {
    let window = env_usize("ANCHOR_CLASSIFY_WINDOW_CHARS", CLASSIFY_WINDOW);
    slide_windows(text, window, (window / 2).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_text_basic() {
        let t = "a".repeat(2000);
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        // first chunk size 800
        assert_eq!(c[0].len(), 800);
    }
    #[test]
    fn chunk_doc_heading() {
        let doc = "§434 Refund\ncontent\n\n§437 Dates\njan";
        let c = chunk_document(doc);
        assert_eq!(c.len(), 2);
    }
    #[test]
    fn classify_heading_rejects_data_lines() {
        // R43.3: dotted numerics that read as data never section — each of
        // these used to open a 1–2 line ghost section.
        for line in [
            "3.5 million users joined this quarter",
            "1.0.31 release notes are ready",
            "21.10.2026 notes from the meeting",
            "10.0.0.1 gateway routes all traffic",
            "21.10.2026 Quarterly Review",
            "10.0.0.1 Gateway Status",
            "4.2.1",
            "NOTE",
            "TODO",
        ] {
            assert_eq!(classify_heading(line), None, "{line}");
        }
        // …while real headings (numbered, §/article forms, set-off CAPS)
        // still classify with unchanged levels.
        assert_eq!(classify_heading("4.2.1 Subsection Detail"), Some(3));
        assert_eq!(classify_heading("4.2 Title"), Some(2));
        assert_eq!(classify_heading("§434 Scope"), Some(1));
        assert_eq!(classify_heading("article 12 membership"), Some(1));
        assert_eq!(classify_heading("4.2 (continued)"), Some(2));
        assert_eq!(classify_heading("INTRODUCTION"), Some(1));
        assert_eq!(classify_heading("ACME CORP"), Some(1));
    }
    #[test]
    fn caps_heading_context_rules() {
        // R43.3: a run-in CAPS line followed by body prose opens nothing …
        let run_in = "Intro line one.\nACME CORP\nSupplies precision parts worldwide from three regional plants every quarter.";
        let (sections, _) = chunk_document_with_sections(run_in);
        assert!(
            sections.iter().all(|s| s.title != "ACME CORP"),
            "{sections:?}"
        );
        // … a set-off CAPS line still sections …
        let set_off = "Intro line one.\n\nACME CORP\n\nSupplies precision parts worldwide.";
        let (sections, _) = chunk_document_with_sections(set_off);
        assert!(
            sections.iter().any(|s| s.title == "ACME CORP"),
            "{sections:?}"
        );
        // … and a run-in CAPS line inside a numbered section nests under it
        // instead of popping the tree to level 1.
        let nested = "4.2.1 Subsection\nSome body text here.\nFINAL VERDICT\nMore detail text.";
        let (sections, _) = chunk_document_with_sections(nested);
        let verdict = sections
            .iter()
            .find(|s| s.title == "FINAL VERDICT")
            .expect("nested CAPS section");
        assert_eq!(verdict.level, 4, "{sections:?}");
        assert_eq!(verdict.path, "4.2.1 Subsection > FINAL VERDICT", "{sections:?}");
        // … while a blank line before the CAPS line keeps it top-level.
        let escaped = "4.2 Summary\nSome body text here.\n\nAPPENDIX\n\nAppendix body text.";
        let (sections, _) = chunk_document_with_sections(escaped);
        let levels: Vec<i32> = sections.iter().map(|s| s.level).collect();
        assert_eq!(levels, vec![2, 1], "{sections:?}");
    }
    #[test]
    fn merge_short_parts_merges_tiny_prose_chunks() {
        // R43.3 port of markdown `merge_short`: a sub-150-char chunk folds
        // into its section-predecessor; a short leading chunk still stands.
        let mut parts = vec!["a".repeat(200), "tiny".to_string(), "b".repeat(200)];
        merge_short_parts(&mut parts);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with(&"a".repeat(200)) && parts[0].ends_with("tiny"));
        let mut leading = vec!["tiny".to_string(), "b".repeat(200)];
        merge_short_parts(&mut leading);
        assert_eq!(leading.len(), 2);
    }
    #[test]
    fn materialize_prose_path_merges_short_tail() {
        // R43.3: the prose tail (`tables = true`) merges a short final
        // chunk; code/config tails (`false`) keep small units intact.
        let para1 = "w ".repeat(775);
        let para2 = "v".repeat(100);
        let mk = || RawSection {
            title: "T".to_string(),
            level: 1,
            parent: None,
            path: "T".to_string(),
            lines: vec![para1.clone(), String::new(), para2.clone()],
            kind: None,
            line_start: 1,
            line_end: 3,
        };
        let (_, prose) = materialize_opts(vec![mk()], 1600, true);
        assert_eq!(prose.len(), 1, "short tail must merge on the prose path");
        assert!(prose[0].content.contains(&para2));
        let (_, units) = materialize_opts(vec![mk()], 1600, false);
        assert_eq!(units.len(), 2, "code/config tails keep small units");
    }
    #[test]
    fn split_section_opts_preserves_indentation() {
        // R43.4: the paragraph edge trim keeps first-line indentation (the
        // old whole-para `trim()` stripped it) while still dropping blank
        // edge lines and trailing whitespace.
        let para1 = format!("{}  ", "x".repeat(68));
        let para2 = "    indented_value = compute_something(arg)";
        let section = format!("{para1}\n\n{para2}");
        let parts = split_section_opts(&section, 80, false);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "x".repeat(68));
        assert_eq!(parts[1], para2);
    }
    /// R43.4 reassembly contract: symbol/key splitters partition source
    /// lines contiguously in order, so each section's chunks rejoin with
    /// the paragraph separator and sections join with a single newline.
    /// Holds when blank runs are single, no paragraph exceeds the cap,
    /// lines carry no trailing whitespace, and no blank line sits right
    /// before a symbol line (a section-trailing blank is edge-trimmed by
    /// design) — all true of the fixtures below.
    fn reassembled(chunks: &[ChunkMeta]) -> String {
        let mut order: Vec<usize> = Vec::new();
        for c in chunks {
            if let Some(i) = c.section_idx {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }
        order
            .iter()
            .map(|&i| {
                chunks
                    .iter()
                    .filter(|c| c.section_idx == Some(i))
                    .map(|c| c.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn code_python_round_trips_bytes() {
        // R43.4: a 2 KB Python file (one >1600-char function exercising the
        // paragraph path, the rest nested symbols) reassembles byte-for-byte
        // with indentation intact.
        let mut groups = Vec::new();
        for i in 0..12 {
            groups.push(format!(
                "        # Group {i}: validate then post the batch.\n        first = [e for e in entries if e.ready]\n        for entry in first:\n            self.post(entry.account, entry.amount)"
            ));
        }
        let big_fn = format!(
            "def reconcile_all(self, entries):\n    \"\"\"Reconcile every ready entry.\"\"\"\n\n{}",
            groups.join("\n\n")
        );
        let mut methods = Vec::new();
        for m in ["post", "summarize", "reset", "audit"] {
            methods.push(format!(
                "    def {m}(self, payload):\n        if payload is None:\n            return None\n        return self.store_{m}(payload)"
            ));
        }
        let src = format!(
            "{big_fn}\nclass Ledger:\n    \"\"\"Tracks balances.\"\"\"\n{}",
            methods.join("\n")
        );
        assert!(src.len() > 2048, "fixture must exceed 2 KB, got {}", src.len());
        let (_, chunks) = chunk_code_with_sections(&src, "ledger.py");
        assert!(chunks.len() > 1);
        assert_eq!(reassembled(&chunks), src);
        assert!(chunks.iter().any(|c| c.content.contains("            self.post(")));
    }
    #[test]
    fn config_yaml_round_trips_bytes() {
        // R43.4: same contract for a 2 KB YAML block (fat keys with
        // internal single blanks; every `key:` line opens a section).
        let mut keys = Vec::new();
        for k in ["gateway_routes", "ledger_routes"] {
            let mut lines = vec![format!("{k}:")];
            for i in 0..60 {
                if i > 0 && i % 10 == 0 {
                    lines.push(String::new());
                }
                lines.push(format!("  - host edge-{i:02}.example.com weight 10"));
            }
            keys.push(lines.join("\n"));
        }
        let src = keys.join("\n");
        assert!(src.len() > 2048, "fixture must exceed 2 KB, got {}", src.len());
        let (_, chunks) = chunk_yaml_with_sections(&src);
        assert!(chunks.len() > 1);
        assert_eq!(reassembled(&chunks), src);
        assert!(chunks.iter().any(|c| c.content.contains("  - host edge-00.example.com")));
    }
    #[test]
    fn prose_sections_carry_line_ranges() {
        // R36.18a: 1-based inclusive ranges; parents span their children and
        // chunks inherit their section's range.
        let doc = "Intro line one.\nIntro line two.\n4.1 Details\nDetail body line.\n4.1.1 Deep Dive\nDeep body line one.\nDeep body line two.\n4.2 Next\nFinal body.";
        let (sections, chunks) = chunk_document_with_sections(doc);
        let ranges: Vec<(i64, i64)> =
            sections.iter().map(|s| (s.line_start, s.line_end)).collect();
        assert_eq!(ranges, vec![(1, 2), (3, 7), (5, 7), (8, 9)], "{sections:?}");
        let deep = chunks
            .iter()
            .find(|c| c.path == "4.1 Details > 4.1.1 Deep Dive")
            .expect("nested chunk");
        assert_eq!((deep.line_start, deep.line_end), (5, 7));
    }
    #[test]
    fn code_symbols_carry_line_ranges() {
        // R36.18a: symbols span their bodies (code citations point at lines).
        let src = "class Ledger:\n    \"\"\"Tracks balances.\"\"\"\n    def post(self, payload):\n        return payload\n    def reset(self):\n        pass\ndef helper():\n    return 1";
        let (sections, chunks) = chunk_code_with_sections(src, "ledger.py");
        let ranges: Vec<(i64, i64)> =
            sections.iter().map(|s| (s.line_start, s.line_end)).collect();
        assert_eq!(ranges, vec![(1, 6), (3, 4), (5, 6), (7, 8)], "{sections:?}");
        let post = chunks.iter().find(|c| c.path.contains("def post")).expect("post chunk");
        assert_eq!((post.line_start, post.line_end), (3, 4));
    }
    #[test]
    fn fallback_windows_carry_line_ranges() {
        // R36.18a: heading-less fallback windows carry exact spans (they
        // overlap by design, so starts increase while ends may overlap).
        let paras: Vec<String> = (0..60)
            .map(|i| format!("Paragraph number {i} with enough words to reach a reasonable length here."))
            .collect();
        let doc = paras.join("\n\n");
        let total = doc.split('\n').count() as i64;
        let (sections, chunks) = chunk_document_with_sections(&doc);
        assert!(sections.len() > 1, "must take the window path");
        let mut prev_start = 0;
        for s in &sections {
            assert!(s.line_start >= 1 && s.line_end >= s.line_start, "{s:?}");
            assert!(s.line_start > prev_start, "starts increase: {sections:?}");
            prev_start = s.line_start;
        }
        assert_eq!(sections[0].line_start, 1);
        assert_eq!(sections[sections.len() - 1].line_end, total);
        assert!(chunks.iter().all(|c| c.line_start >= 1 && c.line_end >= c.line_start));
    }
    #[test]
    fn classify_window() {
        let t = "a".repeat(40000);
        let w = classify_windows(&t);
        assert!(w.len() >= 3);
    }
    #[test]
    fn chunk_text_unicode_safe() {
        let t = "é".repeat(2000); // 2 bytes per char, 2000 chars
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        assert!(c.iter().all(|s| s.chars().count() <= 800));
    }
    #[test]
    fn chunk_text_snaps_to_word_boundaries_without_gaps() {
        let words: Vec<String> = (0..400).map(|i| format!("w{i:03}")).collect();
        let t = words.join(" ");
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        for chunk in &c {
            // whole tokens at both edges — never "w01" / "02".
            assert!(chunk.starts_with('w'), "{chunk:?}");
            assert!(chunk.chars().last().unwrap().is_ascii_digit(), "{chunk:?}");
            assert!(chunk.chars().count() <= 800);
        }
        for w in &words {
            assert!(c.iter().any(|chunk| chunk.split(' ').any(|tok| tok == w)), "gap at {w}");
        }
    }
    #[test]
    fn chunk_text_prefers_sentence_ends() {
        let t = "Sentence one states the rule. Sentence two gives context! Sentence three asks why? ".repeat(30);
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        for chunk in &c[..c.len() - 1] {
            assert!(
                chunk.chars().last().is_some_and(|c| matches!(c, '.' | '!' | '?')),
                "non-final chunk cuts mid-sentence: {chunk:?}"
            );
        }
    }
    #[test]
    fn sentence_end_skips_abbreviations_and_decimals() {
        // Only [.?!] followed by whitespace counts — "U.S.A" and "3.5"
        // never win; the real end does.
        let chars: Vec<char> = "See U.S.A v3.5 notes. Next".chars().collect();
        let end = sentence_end_in(&chars, 0, chars.len()).unwrap();
        assert_eq!(chars[end - 2..end].iter().collect::<String>(), ". ");
        assert_eq!(chars[end], 'N');
    }
    #[test]
    fn chunk_text_hard_cuts_only_past_overlap() {
        // A 900-char token: no boundary fits in the shrink zone, so the
        // first window hard-cuts at 800 and the tail stays covered.
        let t = format!("{} tail words here", "a".repeat(900));
        let c = chunk_text(&t);
        assert_eq!(c[0].chars().count(), 800);
        assert!(c.last().unwrap().contains("tail words here"));
        assert!(c.iter().all(|s| s.chars().count() <= 800));
    }
    #[test]
    fn clean_doc_text_strips_controls_and_page_numbers() {
        // R43.2: controls/whitespace clean doc-wide; page-number lines drop
        // only at page boundaries; `[page N]` markers always survive.
        let raw = "[page 1]\nTitle\r\n\x00\x0bBody\u{200b} text\t\tspaced  \nSection 42 stays\n42\n42 things stay\n12345 stays\nPage 3\nTail words here.\n- 12 -\n[page 2]\nSecond page opens here.\nMore content follows.\n3 of 10\n— 7 —\nFinal words.\n\n\nTail\u{feff}";
        let cleaned = clean_doc_text(raw);
        assert!(cleaned.contains("Title\nBody text spaced"), "{cleaned:?}");
        assert!(cleaned.contains("[page 1]") && cleaned.contains("[page 2]"), "{cleaned:?}");
        assert!(!cleaned.contains("\u{200b}") && !cleaned.contains('\u{feff}'));
        // page-edge numbers drop …
        for dropped in ["Page 3", "3 of 10", "- 12 -", "— 7 —"] {
            assert!(!cleaned.contains(dropped), "{cleaned:?}");
        }
        // … mid-page numbers and number-carrying lines stay …
        assert!(cleaned.contains("Section 42 stays\n42\n42 things stay"), "{cleaned:?}");
        for kept in ["Section 42 stays", "42 things stay", "12345 stays", "Tail", "Final words."] {
            assert!(cleaned.contains(kept), "{cleaned:?}");
        }
        // exactly one bare 42 survives (page 1 mid-page; page 2 has none).
        assert_eq!(cleaned.matches("\n42\n").count(), 1, "{cleaned:?}");
        assert!(!cleaned.contains("\n\n\n"));
    }
    #[test]
    fn clean_doc_text_strips_repeated_headers() {
        // R43.2: repeated headers/footers strip at page boundaries only.
        let page = |body: &str| format!("Acme Confidential\n{body}\nAcme Footer Line");
        let raw = format!(
            "[page 1]\n{}\n[page 2]\n{}\n[page 3]\n{}\n",
            page("Body line one.\nBody line two."),
            page("Body line three.\nBody line four."),
            page("Body line five.\nBody line six."),
        );
        let cleaned = clean_doc_text(&raw);
        assert!(!cleaned.contains("Acme Confidential"), "{cleaned:?}");
        assert!(!cleaned.contains("Acme Footer Line"), "{cleaned:?}");
        assert!(cleaned.contains("Body line one."), "{cleaned:?}");
        assert!(cleaned.contains("Body line six."), "{cleaned:?}");
        assert!(cleaned.contains("[page 2]"), "{cleaned:?}");
        // twice is not a pattern — kept (marker-less: doc edges only).
        let raw2 = "Acme Confidential\nBody.\nAcme Confidential\nOther.";
        assert!(clean_doc_text(raw2).contains("Acme Confidential"));
        // long lines never count toward repetition.
        let long = format!("{}\n{}\n{}\nshort", "L".repeat(121), "L".repeat(121), "L".repeat(121));
        assert!(clean_doc_text(&long).contains(&"L".repeat(121)));
        // marker-less docs strip edge boilerplate repeated 3×+; the
        // mid-document occurrence stays (same rule, one segment).
        let raw3 = "Acme Confidential\nBody one.\nMiddle words here.\nAcme Confidential\nBody two.\nMore middle words.\nAcme Confidential";
        let cleaned3 = clean_doc_text(raw3);
        assert_eq!(cleaned3.matches("Acme Confidential").count(), 1, "{cleaned3:?}");
        assert!(cleaned3.contains("Middle words here."), "{cleaned3:?}");
    }
    #[test]
    fn clean_doc_text_keeps_table_cells_mid_page() {
        // R43.2 regression: bare cell lines (`Yes`, `N/A`, years, amounts)
        // repeated past the threshold used to strip doc-wide — they are
        // content when mid-page. Header/footer/page-number lines at the page
        // edges still strip.
        let row = |vendor: &str, amount: &str, paid: &str, year: &str| {
            format!("Vendor\n{vendor}\nEarned\n{amount}\nPaid\n{paid}\nYear\n{year}")
        };
        let raw = format!(
            "[page 1]\nAcme Confidential\nVendor Rebate Statement\nRebate payouts by vendor follow below.\n{}\n{}\nSummary totals appear at the end of the statement.\nProgram notes continue on the next page here.\nContact finance with questions about payouts.\nPage 1\n\
             [page 2]\nAcme Confidential\nContinued from the previous page here.\nGamma region rows appear first below.\n{}\n{}\nRegional totals were reconciled twice this cycle.\nPaid flags were reviewed by the finance team.\nPage 2\n\
             [page 3]\nAcme Confidential\nFinal page of the rebate statement.\nRemaining vendor rows appear below.\n{}\n{}\nPaid flags are final for every vendor row.\nYear totals were reconciled twice today.\nStatement prepared by the finance team today.\nPage 3 of 3",
            row("Alpha Supply", "1200", "Yes", "2024"),
            row("Beta Parts", "800", "N/A", "2024"),
            row("Gamma Goods", "950", "Yes", "2023"),
            row("Delta Tools", "400", "N/A", "2024"),
            row("Eta Parts", "600", "Yes", "2023"),
            row("Zeta Supply", "750", "N/A", "2023"),
        );
        let cleaned = clean_doc_text(&raw);
        // every cell survives, including above-threshold bare cells …
        for cell in ["Yes", "N/A", "2024", "2023", "Vendor", "Earned", "Paid", "Year"] {
            let want = raw.matches(cell).count();
            assert_eq!(cleaned.matches(cell).count(), want, "{cell}: {cleaned:?}");
        }
        for amount in ["1200", "800", "950", "400", "600", "750"] {
            assert!(cleaned.contains(amount), "{amount}: {cleaned:?}");
        }
        // … while edge boilerplate strips and markers survive.
        assert!(!cleaned.contains("Acme Confidential"), "{cleaned:?}");
        for footer in ["Page 1", "Page 2", "Page 3 of 3"] {
            assert!(!cleaned.contains(footer), "{cleaned:?}");
        }
        assert!(cleaned.contains("[page 1]") && cleaned.contains("[page 3]"), "{cleaned:?}");
    }
    #[test]
    fn clean_doc_text_keeps_eval_fixture_cells() {
        // File-shaped cleaning pin (R43.2): table cells must survive
        // page-boundary cleaning. Lives under eval_data but is NOT in the
        // eval manifest — the harness inserts file bytes directly (no
        // cleaning/chunking), so chunking fixes pin at unit level instead.
        let path = format!(
            "{}/tests/eval_data/corpus/support/vendor-rebate-statement.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let raw = std::fs::read_to_string(&path).expect("eval fixture readable");
        let cleaned = clean_doc_text(&raw);
        for (cell, want) in [("Yes", 3), ("N/A", 3), ("2024", 3), ("2023", 3)] {
            assert_eq!(cleaned.matches(cell).count(), want, "{cell}: {cleaned:?}");
        }
        for amount in ["1200", "800", "950", "400", "600", "750"] {
            assert!(cleaned.contains(amount), "{amount}: {cleaned:?}");
        }
        assert!(!cleaned.contains("Acme Confidential"), "{cleaned:?}");
        assert_eq!(cleaned.matches("[page ").count(), 3, "{cleaned:?}");
    }
    #[test]
    fn repeat_threshold_matches_python_rounding() {
        assert_eq!(repeat_threshold(0), 3);
        assert_eq!(repeat_threshold(50), 3); // round(0.5) = 0 -> floor 3
        assert_eq!(repeat_threshold(250), 3); // round(2.5) = 2 -> floor 3
        assert_eq!(repeat_threshold(450), 4); // round(4.5) = 4 banker
        assert_eq!(repeat_threshold(550), 6); // round(5.5) = 6 banker
        assert_eq!(repeat_threshold(1000), 10);
    }
    #[test]
    fn code_rust_sections_nest_impl_methods() {
        let code = "use std::sync::Arc;\n\n/// Classify text.\npub struct Item {\n    pub name: String,\n}\n\nimpl Item {\n    pub fn new(name: &str) -> Self {\n        Self { name: name.into() }\n    }\n\n    fn helper(&self) -> bool {\n        true\n    }\n}\n\npub fn classify(text: &str) -> Vec<String> {\n    vec![]\n}\n";
        let (secs, chunks) = chunk_code_with_sections(code, "src/lib.rs");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "struct Item", "impl Item", "fn new", "fn helper", "fn classify"]);
        assert_eq!(secs[2].level, 1);
        assert_eq!(secs[3].level, 2);
        assert_eq!(secs[3].parent, Some(2));
        assert_eq!(secs[3].path, "impl Item > fn new");
        assert_eq!(secs[5].level, 1);
        assert!(chunks.iter().all(|c| c.section_idx.is_some()));
        assert!(chunks.iter().any(|c| c.path == "impl Item > fn new"));
    }

    #[test]
    fn code_python_sections_nest_methods() {
        let code = "\"\"\"Module docs.\"\"\"\n\nclass Store:\n    def __init__(self):\n        self.x = 1\n\n    def get(self):\n        return self.x\n\ndef helper():\n    return 1\n";
        let (secs, _) = chunk_code_with_sections(code, "store.py");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "class Store", "def __init__", "def get", "def helper"]);
        assert_eq!(secs[2].level, 2);
        assert_eq!(secs[2].parent, Some(1));
        assert_eq!(secs[2].path, "class Store > def __init__");
        assert_eq!(secs[4].level, 1);
        assert_eq!(secs[4].parent, None);
    }

    #[test]
    fn code_typescript_class_and_const_arrow() {
        let code = "import { x } from './m';\n\nexport class App {\n    render() {\n        return null;\n    }\n}\n\nexport const run = async () => {\n    await main();\n};\n";
        let (secs, chunks) = chunk_code_with_sections(code, "app/app.ts");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "class App", "const run"]);
        assert_eq!(secs[1].level, 1);
        assert_eq!(secs[2].level, 1);
        // `render()` has no keyword: it stays inside the class section
        assert!(chunks.iter().any(|c| c.path == "class App" && c.content.contains("render()")));
    }

    #[test]
    fn code_go_receiver_and_fallbacks() {
        let code = "package main\n\nimport \"fmt\"\n\ntype Server struct{}\n\nfunc (s *Server) Start() {\n    fmt.Println(\"up\")\n}\n\nfunc main() {\n    fmt.Println(\"hi\")\n}\n";
        let (secs, _) = chunk_code_with_sections(code, "cmd/main.go");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "type Server", "func Server.Start", "func main"]);
        // sql + unknown extensions stay root-only
        let (secs_sql, _) = chunk_code_with_sections("SELECT 1;\n", "q.sql");
        assert_eq!(secs_sql.len(), 1);
        let (secs_txt, _) = chunk_code_with_sections("hello\n", "notes.txt");
        assert_eq!(secs_txt.len(), 1);
        // minified single-line bundles stay root-only
        let big = "var a=1;".repeat(20_000);
        let (secs_min, _) = chunk_code_with_sections(&big, "app.min.js");
        assert_eq!(secs_min.len(), 1);
    }

    #[test]
    fn code_c_style_matches_definitions_not_calls() {
        let code = "#include <stdio.h>\n\nint add(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    int x = add(1, 2);\n    if (x > 0) {\n        return 0;\n    }\n    return x;\n}\n";
        let (secs, _) = chunk_code_with_sections(code, "add.c");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "add", "main"]);
    }

    #[test]
    fn code_java_nests_methods_under_class() {
        let code = "package com.acme;\n\npublic class Billing {\n    private int total;\n\n    public int add(int d) {\n        return total + d;\n    }\n}\n";
        let (secs, _) = chunk_code_with_sections(code, "Billing.java");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "class Billing", "add"]);
        assert_eq!(secs[2].level, 2);
        assert_eq!(secs[2].path, "class Billing > add");
    }

    #[test]
    fn chunk_for_doc_type_dispatches_on_code() {
        let code = "pub fn classify(text: &str) -> Vec<String> {\n    vec![]\n}\n";
        let (secs_code, _) = chunk_for_doc_type("code", "lib.rs", code);
        assert!(secs_code.iter().any(|s| s.title == "fn classify"));
        let (secs_prose, _) = chunk_for_doc_type("meeting", "notes.txt", code);
        assert!(secs_prose.iter().all(|s| s.title.is_empty()));
        // Markdown filenames take the block splitter even for prose doc types.
        let (secs_md, _) = chunk_for_doc_type("meeting", "n.md", "# Hi\ntext\n");
        assert_eq!(secs_md[0].title, "Hi");
    }

    #[test]
    fn chunk_with_sections_hierarchy() {
        let doc = "§434 Refund\ncontent a\n\n§437 Dates\njan\n\n4.2.1 Sub\nsub content";
        let (secs, chunks) = chunk_document_with_sections(doc);
        // 3 sections: §434, §437, 4.2.1 (child of §437)
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].level, 1);
        assert_eq!(secs[1].level, 1);
        assert_eq!(secs[2].level, 3);
        assert_eq!(secs[2].parent, Some(1));
        assert_eq!(secs[2].path, "§437 Dates > 4.2.1 Sub");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2].section_idx, Some(2));
        assert_eq!(chunks[2].path, "§437 Dates > 4.2.1 Sub");
    }
    #[test]
    fn chunk_with_sections_root() {
        let doc = "plain intro\nno heading\n\n§434 Refund\ncontent";
        let (secs, chunks) = chunk_document_with_sections(doc);
        assert!(secs.iter().any(|s| s.level == 0));
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn prose_child_of_empty_root_has_no_leading_separator() {
        // R40.5: a top-level heading under the empty preamble root used to
        // build `" > 8.3 …"` paths (PDF-style); breadcrumbs and embed
        // prefixes must carry the bare heading instead.
        let doc = "Some intro paragraph spanning a visual line here.\n8.3 Reversal notifications\nThe beneficiary receives a reversal notice.\n";
        let (secs, chunks) = chunk_document_with_sections(doc);
        let sec = secs.iter().find(|s| s.title == "8.3 Reversal notifications").expect("section");
        assert_eq!(sec.path, "8.3 Reversal notifications");
        let chunk = chunks.iter().find(|c| c.section_idx == secs.iter().position(|s| s.title == sec.title)).expect("chunk");
        assert_eq!(chunk.path, "8.3 Reversal notifications");
        assert!(secs.iter().all(|s| !s.path.starts_with('>')), "no leading separator: {:?}", secs.iter().map(|s| &s.path).collect::<Vec<_>>());
        // The embed prefix for this chunk is heading + text (production
        // `embed_chunks` calls `markdown::embed_input` with the stored path).
        assert_eq!(
            crate::markdown::embed_input(&chunk.path, &chunk.content),
            format!("8.3 Reversal notifications\n{}", chunk.content)
        );
    }

    #[test]
    fn config_yaml_every_key_splits() {
        // every mapping key (leaves included) opens an indent-nested section
        let yaml = "server:\n  host: example.com\n  tls:\n    cert_file: /etc/cert.pem\nreplicas: 3\n";
        let (secs, chunks) = chunk_config_with_sections(yaml, "values.yaml");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["server", "host", "tls", "cert_file", "replicas"]);
        assert_eq!(secs[3].path, "server > tls > cert_file");
        assert!(chunks.iter().any(|c| c.path == "server > tls > cert_file"));
    }

    #[test]
    fn config_yaml_ignores_urls_times_lists_markers() {
        // the colon guard: `http://x` values and `12:30` lines never open
        // sections; list items, comments and doc markers fold
        let yaml = "# comment\nurl: http://example.com:8080/x\nseen at 12:30\nitems:\n  - alpha\n  - beta\n---\nnext: 1\n";
        let (secs, _) = chunk_config_with_sections(yaml, "c.yml");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "url", "items", "next"]);
    }

    #[test]
    fn config_json_every_key_splits() {
        let json = "{\n  \"server\": {\n    \"host\": \"x\",\n    \"tls\": {\n      \"cert\": \"f.pem\"\n    }\n  },\n  \"replicas\": 3\n}\n";
        let (secs, chunks) = chunk_config_with_sections(json, "cfg.json");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "server", "host", "tls", "cert", "replicas"]);
        assert_eq!(secs[4].path, "server > tls > cert");
        assert!(chunks.iter().any(|c| c.path == "server > host"));
    }

    #[test]
    fn config_jsonc_comments_stay_in_section() {
        let json = "{\n  // server block\n  \"a\": 1,\n  /* x */ \"b\": 2\n}\n";
        let (secs, _) = chunk_config_with_sections(json, "s.jsonc");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "a", "b"]);
    }

    #[test]
    fn config_toml_tables_and_keys_split() {
        let toml = "title = \"demo\"\n\n[server]\nhost = \"x\"\n\n[server.tls]\ncert = \"f.pem\"\n";
        let (secs, chunks) = chunk_config_with_sections(toml, "app.toml");
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["title", "server", "host", "server.tls", "cert"]);
        assert_eq!(secs[4].path, "server.tls > cert");
        assert!(chunks.iter().any(|c| c.path == "server > host"));
    }

    #[test]
    fn config_guards_minified_single_line() {
        let big = "{\"a\":1,".repeat(20_000);
        let (secs_min, _) = chunk_config_with_sections(&big, "bundle.json");
        assert_eq!(secs_min.len(), 1);
    }

    #[test]
    fn markers_split_sections_with_clean_titles() {
        let doc = "# Report\nintro line\n\n## Budget\nbudget line\n\n### Notes\nnote line\n";
        let (secs, chunks) = chunk_document_with_sections(doc);
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Report", "Budget", "Notes"]);
        assert_eq!(
            secs.iter().map(|s| s.level).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(secs[1].parent, Some(0));
        assert_eq!(secs[2].parent, Some(1));
        assert_eq!(secs[2].path, "Report > Budget > Notes");
        assert_eq!(chunks.len(), 3);
        // content keeps the marker line; titles/paths drop it
        assert!(chunks[0].content.starts_with("# Report\n"));
        assert!(chunks[1].content.starts_with("## Budget\n"));
        assert_eq!(chunks[1].path, "Report > Budget");
        // a long extractor heading still sections; its title truncates
        let long = format!("# {}\nbody\n", "x".repeat(100));
        let (secs_long, _) = chunk_document_with_sections(&long);
        assert_eq!(secs_long.len(), 1);
        assert_eq!(secs_long[0].level, 1);
        assert!(secs_long[0].title.chars().count() <= 80);
        assert!(secs_long[0].title.ends_with("..."));
    }

    #[test]
    fn marker_nospace_and_deep_hash_are_body() {
        let doc = "#nospace line here\n\n####### seven\n\n# \n\nnormal text";
        let (secs, chunks) = chunk_document_with_sections(doc);
        assert!(secs.iter().all(|s| s.title.is_empty()));
        assert_eq!(secs.len(), 1);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn byte_identical_prose_guards() {
        // No `#` markers, no tables, under the fallback threshold: exact
        // legacy outputs, byte for byte (blank lines fold into sections).
        let (secs, chunks) =
            chunk_document_with_sections("§434 Refund\ncontent a\n\n§437 Dates\njan");
        assert_eq!(secs.len(), 2);
        let bodies: Vec<_> = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(bodies, vec!["§434 Refund\ncontent a\n", "§437 Dates\njan"]);
        let (secs_caps, chunks_caps) = chunk_document_with_sections("QUARTERLY PLAN\nbody here");
        assert_eq!(secs_caps[0].title, "QUARTERLY PLAN");
        assert_eq!(chunks_caps[0].content, "QUARTERLY PLAN\nbody here");
        let (secs_root, chunks_root) =
            chunk_document_with_sections("plain intro\nno heading\n\n§434 Refund\ncontent");
        assert_eq!(secs_root.len(), 2);
        assert_eq!(secs_root[0].level, 0);
        let root_bodies: Vec<_> = chunks_root.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(
            root_bodies,
            vec!["plain intro\nno heading\n", "§434 Refund\ncontent"]
        );
        // small heading-less docs keep their single root
        let small = "just a note\nsecond line\n";
        let (secs_small, chunks_small) = chunk_document_with_sections(small);
        assert_eq!(secs_small.len(), 1);
        assert!(secs_small[0].title.is_empty());
        assert_eq!(chunks_small.len(), 1);
        assert_eq!(chunks_small[0].content, small);
    }

    #[test]
    fn fallback_windows_split_headingless_docs() {
        let paras: Vec<String> = (0..60)
            .map(|i| format!("Paragraph {i:02} {}", "body text ".repeat(20)))
            .collect();
        let doc = paras.join("\n\n");
        assert!(doc.len() > 8000);
        let (secs, chunks) = chunk_document_with_sections(&doc);
        // several untitled level-0 windows, never one whole-file section
        assert!(secs.len() >= 3, "sections: {}", secs.len());
        assert!(secs
            .iter()
            .all(|s| s.title.is_empty() && s.level == 0 && s.parent.is_none()));
        // every paragraph is covered, chunks stay capped ...
        for i in 0..60 {
            let needle = format!("Paragraph {i:02}");
            assert!(
                chunks.iter().any(|c| c.content.contains(&needle)),
                "{needle} missing"
            );
        }
        assert!(chunks.iter().all(|c| c.content.chars().count() <= 1600));
        // ... with overlap (shared paragraphs repeat across windows) ...
        let total: usize = chunks.iter().map(|c| c.content.len()).sum();
        assert!(total > doc.len(), "no overlap: {total} <= {}", doc.len());
        // ... deterministically.
        let (secs2, chunks2) = chunk_document_with_sections(&doc);
        assert_eq!(secs2.len(), secs.len());
        assert_eq!(
            chunks2.iter().map(|c| &c.content).collect::<Vec<_>>(),
            chunks.iter().map(|c| &c.content).collect::<Vec<_>>()
        );
    }

    #[test]
    fn fallback_degenerate_shapes_stay_safe() {
        // one giant paragraph: single root, hard-split into capped chunks
        let big = "x".repeat(10_000);
        let (secs, chunks) = chunk_document_with_sections(&big);
        assert_eq!(secs.len(), 1);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.content.chars().count() <= 800));
        // empty text: one empty root, no chunks (today's shape)
        let (secs_e, chunks_e) = chunk_document_with_sections("");
        assert_eq!(secs_e.len(), 1);
        assert!(chunks_e.is_empty());
    }

    #[test]
    fn tables_fit_one_unit_when_fitting() {
        let md = "# Report\n\n| Name | Cost |\n|---|---|\n| A | 1 |\n| B | 2 |\n";
        let (_, chunks) = chunk_document_with_sections(md);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("| A | 1 |"));
        assert!(chunks[0].content.contains("| B | 2 |"));
        let tsv = "# Report\n\nName\tCost\nA\t1\nB\t2\n";
        let (_, chunks_tsv) = chunk_document_with_sections(tsv);
        assert_eq!(chunks_tsv.len(), 1);
        assert!(chunks_tsv[0].content.contains("A\t1"));
    }

    #[test]
    fn tables_split_with_header_repeat() {
        let header = "| Name | Cost |";
        let delim = "|---|---|";
        let rows: Vec<String> = (0..80)
            .map(|i| format!("| Item {i:02} | description text padded to widen row {i:02} |"))
            .collect();
        let md = format!("# Report\n\n{}\n{}\n{}\n", header, delim, rows.join("\n"));
        let (_, chunks) = chunk_document_with_sections(&md);
        assert!(chunks.len() >= 3, "chunks: {}", chunks.len());
        assert_eq!(chunks[0].content, "# Report");
        for c in &chunks[1..] {
            let head = &c.content[..c.content.len().min(60)];
            assert!(c.content.contains(header), "unheadered unit: {head}");
            assert!(c.content.contains(delim), "delimiter dropped: {head}");
        }
        for row in &rows {
            assert!(
                chunks.iter().any(|c| c.content.contains(row.as_str())),
                "split row: {row}"
            );
        }
        // tab-separated (docx/odt shape): first row repeats, rows stay whole
        let thead = "Name\tCost";
        let trows: Vec<String> = (0..80)
            .map(|i| format!("Item {i:02}\tcost center {i:02} with padding text to widen the row"))
            .collect();
        let tsv = format!("# Report\n\n{}\n{}\n", thead, trows.join("\n"));
        let (_, tchunks) = chunk_document_with_sections(&tsv);
        assert!(tchunks.len() >= 3, "chunks: {}", tchunks.len());
        for c in &tchunks[1..] {
            let head = &c.content[..c.content.len().min(60)];
            assert!(c.content.contains(thead), "unheadered unit: {head}");
        }
        for row in &trows {
            assert!(
                tchunks.iter().any(|c| c.content.contains(row.as_str())),
                "split row: {row}"
            );
        }
    }

    #[test]
    fn table_with_caption_degrades_without_losing_content() {
        // one non-row line breaks the table shape: the block hard-splits,
        // but every cell's text still lands in some chunk.
        let mut lines = vec![
            "# Report".to_string(),
            String::new(),
            "Caption: quarterly costs".to_string(),
        ];
        for i in 0..80 {
            lines.push(format!(
                "| Item {i:02} | value {i:02} padded to widen the row enough |"
            ));
        }
        let doc = lines.join("\n");
        let (_, chunks) = chunk_document_with_sections(&doc);
        assert!(chunks
            .iter()
            .any(|c| c.content.contains("Caption: quarterly costs")));
        for i in 0..80 {
            let needle = format!("Item {i:02}");
            assert!(
                chunks.iter().any(|c| c.content.contains(&needle)),
                "{needle} missing"
            );
        }
    }

    #[test]
    fn toc_contents_page_yields_no_ghost_sections() {
        // PDF-ish fixture: contents page first, real sections after.
        let doc = "Contents\n\
             8.1 Detection 23\n\
             8.2 Prevention 24\n\
             8.3 Response 25\n\
             8.4 Recovery 26\n\
             \n\
             8.1 Detection\n\
             Body text about detection methods and monitoring coverage here.\n\
             \n\
             8.2 Prevention\n\
             Body text about prevention controls and hardening steps here.\n";
        let (secs, chunks) = chunk_document_with_sections(doc);
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["", "8.1 Detection", "8.2 Prevention"]);
        // entries stay in the root chunk — suppressed, never dropped.
        assert!(chunks[0].content.contains("8.1 Detection 23"));
        assert!(chunks[0].content.contains("8.4 Recovery 26"));
        assert!(chunks[0].content.contains("Contents"));
    }

    #[test]
    fn toc_dot_leaders_caps_and_in_block_header_suppressed() {
        let doc = "CONTENTS\n\
             8.1 Detection ........ 23\n\
             INTRODUCTION 5\n\
             8.2 Prevention ..... 24\n\
             8.3 Response ..... 25\n";
        let (secs, chunks) = chunk_document_with_sections(doc);
        assert!(secs.iter().all(|s| s.title.is_empty()), "{secs:?}");
        assert_eq!(chunks.len(), 1);
        for needle in ["CONTENTS", "8.1 Detection ........ 23", "INTRODUCTION 5"] {
            assert!(chunks[0].content.contains(needle), "{needle} missing");
        }
    }

    #[test]
    fn toc_title_header_across_blank_line_suppressed() {
        let doc = "TABLE OF CONTENTS\n\n8.1 Detection 23\n8.2 Prevention 24\n8.3 Response 25\n";
        let (secs, chunks) = chunk_document_with_sections(doc);
        assert!(secs.iter().all(|s| s.title.is_empty()), "{secs:?}");
        assert!(chunks.iter().any(|c| c.content.contains("TABLE OF CONTENTS")));
    }

    #[test]
    fn toc_lone_and_short_runs_still_split() {
        // a single digit-ending heading is a heading, not a contents page
        let (secs, _) = chunk_document_with_sections("8.1 Summary 2024\nBody text here.\n");
        assert_eq!(secs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["8.1 Summary 2024"]);
        // two entries are below the run threshold — both still split
        let (secs2, _) = chunk_document_with_sections("8.1 Detection 23\n8.2 Prevention 24\n");
        assert_eq!(secs2.len(), 2);
        // real numbered sections with body text always split
        let real = "8.1 Detection\nBody text about detection here.\n8.2 Prevention\nMore body text here.\n";
        let (secs3, _) = chunk_document_with_sections(real);
        assert_eq!(
            secs3.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            vec!["8.1 Detection", "8.2 Prevention"]
        );
    }

    #[test]
    fn toc_entry_probe_rejects_bare_markers_and_numbers() {
        assert!(toc_entry_candidate("8.1 Detection 23"));
        assert!(toc_entry_candidate("8.1 Detection ........ 23"));
        assert!(toc_entry_candidate("INTRODUCTION 5"));
        assert!(!toc_entry_candidate("8.1 Detection"));
        assert!(!toc_entry_candidate("QUARTERLY PLAN"));
        assert!(!toc_entry_candidate("42"));
        assert!(!toc_entry_candidate("Article 12"));
        assert!(!toc_entry_candidate("Item 12345"));
        assert!(!toc_entry_candidate(""));
        assert!(!toc_title_line("Contents page footer"));
        assert!(toc_title_line("Contents"));
        assert!(toc_title_line("TABLE OF CONTENTS"));
    }

    #[test]
    fn toc_sparse_blank_separated_entries_suppressed() {
        // R40.3: PDF extraction often blank-separates TOC entries — the old
        // blank-delimited-block rule saw size-1 blocks and never flagged.
        let text = "Contents\n\n8.1 Detection 23\n\n8.2 Prevention 24\n\n8.3 Response 25\n\nBody text here.\n";
        let (sections, _) = chunk_document_with_sections(text);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(!titles.iter().any(|t| t.contains("23") || t.contains("24") || t.contains("25")), "sparse ghosts: {titles:?}");
    }

    #[test]
    fn toc_long_dot_leader_line_suppressed() {
        // R40.3: leader padding pushes raw lines past 80 chars — length and
        // classification read the entry words, not the padding.
        let pad = ".".repeat(70);
        let text = format!("Contents\n8.1 Detection {pad} 23\n8.2 Prevention {pad} 24\n8.3 Response {pad} 25\nBody.\n");
        assert!(text.lines().nth(1).unwrap().len() > 80, "fixture must exceed the old raw cap");
        let (sections, _) = chunk_document_with_sections(&text);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(!titles.iter().any(|t| t.contains("23") || t.contains("24") || t.contains("25")), "leader ghosts: {titles:?}");
    }

    #[test]
    fn toc_scattered_digit_headings_still_split() {
        // R40.3: the span guard — digit-ending real headings far apart never
        // share a local window, so they keep sectioning.
        let filler = "Prose line.\n".repeat(20);
        let text = format!("8.1 Summary 2024\n{filler}8.2 Outlook 2025\n{filler}8.3 Review 2026\nTail.\n");
        let (sections, _) = chunk_document_with_sections(&text);
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"8.1 Summary 2024"), "scattered real headings must split: {titles:?}");
        assert!(titles.contains(&"8.2 Outlook 2025"), "scattered real headings must split: {titles:?}");
        assert!(titles.contains(&"8.3 Review 2026"), "scattered real headings must split: {titles:?}");
    }

    #[test]
    fn chunk_for_doc_type_dispatches_config_by_filename() {
        // extension is ground truth: structured files split by keys whatever
        // their detected doc_type; code + prose dispatch unchanged
        let yaml = "server:\n  host: x\n";
        let (secs, _) = chunk_for_doc_type("general", "values.yaml", yaml);
        assert!(secs.iter().any(|s| s.title == "server"));
        let toml = "[server]\nhost = \"x\"\n";
        let (secs_toml, _) = chunk_for_doc_type("general", "app.toml", toml);
        assert!(secs_toml.iter().any(|s| s.title == "server"));
        let code = "pub fn f() {}\n";
        let (secs_code, _) = chunk_for_doc_type("code", "lib.rs", code);
        assert!(secs_code.iter().any(|s| s.title == "fn f"));
    }

}
