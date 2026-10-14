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

/// B12 document cleaning (archived `cleaning.py` parity): control chars,
/// page-number lines and whitespace per `clean_text`, plus repeated
/// header/footer line stripping. Applied to the document copy that feeds
/// chunking — `ingested_items.text` stays raw, exactly like the archived
/// pipeline (`strip_repeated(clean_text(t), repeated_lines(t))`).
pub fn clean_doc_text(text: &str) -> String {
    let cleaned = clean_text_lines(text);
    strip_repeated_lines(&cleaned, &repeated_lines(text))
}

/// Per-line cleaning: `\r` normalisation, control-char strip, page-number
/// line drop, intra-line whitespace collapse, blank-run collapse.
fn clean_text_lines(text: &str) -> String {
    let nls = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    for line in nls.split('\n') {
        let noctrl: String = line.chars().filter(|c| !is_stripped_control(*c)).collect();
        if noctrl.trim().is_empty() {
            blanks += 1;
            if blanks == 1 {
                out.push(String::new());
            }
            continue;
        }
        blanks = 0;
        if is_page_number_line(noctrl.trim()) {
            continue;
        }
        out.push(collapse_ws_rstrip(&noctrl));
    }
    out.join("\n").trim().to_string()
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

/// Lines repeated across the document (headers/footers/templates): lowercased
/// stripped lines at/above threshold, computed on the raw text like the
/// archived pipeline. Lines over 120 chars never count.
fn repeated_lines(text: &str) -> std::collections::HashSet<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut total = 0usize;
    for line in text.split('\n') {
        let s = line.trim().to_lowercase();
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

fn strip_repeated_lines(text: &str, repeated: &std::collections::HashSet<String>) -> String {
    if repeated.is_empty() {
        return text.to_string();
    }
    text.split('\n')
        .filter(|l| !repeated.contains(l.trim().to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join("\n")
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

/// Single heading classifier — was `is_heading` + `heading_level` (+ 3 level
/// helpers), which evaluated the same regexes twice. Returns the section level
/// (`Some`) or `None` for body text. Level rules are unchanged: §-forms carry
/// dots (`§434`→1, `4.2.1`-style→1+dots), article/annex/section words and
/// ALL-CAPS lines are level 1. R36.7 checks `#` markers first (they bypass
/// the 80-char guard — parser structure, not heuristic guesswork).
fn classify_heading(line: &str) -> Option<i32> {
    static RE_NUM: OnceLock<Regex> = OnceLock::new();
    static RE_CAPS: OnceLock<Regex> = OnceLock::new();
    if let Some((level, _)) = marker_heading(line) {
        return Some(level);
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
    let numbered = re.is_match(s);
    let caps = RE_CAPS.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$").unwrap());
    if !numbered && !(caps.is_match(s) && !s.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some(numbered_heading_level(s))
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
    let mut raw_sections: Vec<RawSection> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for line in lines {
        if let Some(level) = classify_heading(line) {
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
            let path = if let Some(p) = parent {
                format!("{} > {}", raw_sections[p].path, title)
            } else {
                title.clone()
            };
            let idx = raw_sections.len();
            raw_sections.push(RawSection {
                title,
                level,
                parent,
                path,
                lines: vec![line.to_string()],
            });
            stack.push(idx);
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
    materialize_opts(raw_sections, max_chars, true)
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
    let mut raw = Vec::new();
    let mut s = 0;
    while s < paras.len() {
        let e = window_end(&paras, s);
        raw.push(window_section(&paras[s..e]));
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

fn window_section(paras: &[String]) -> RawSection {
    let mut lines = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(p.split('\n').map(|l| l.to_string()));
    }
    RawSection {
        title: String::new(),
        level: 0,
        parent: None,
        path: String::new(),
        lines,
    }
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
fn split_section_opts(section: &str, max_chars: usize, tables: bool) -> Vec<String> {
    static RE_SPLIT: OnceLock<Regex> = OnceLock::new();
    if section.chars().count() <= max_chars {
        return vec![section.to_string()];
    }
    let re = RE_SPLIT.get_or_init(|| Regex::new(r"\n\s*\n").unwrap());
    let paras: Vec<String> = re
        .split(section)
        .map(|p| p.trim().to_string())
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
            // The prose/code/config splitters don't track source lines — only
            // the markdown path fills ranges/kind/meta (migration 19 columns).
            line_start: 0,
            line_end: 0,
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
        let parts = split_section_opts(&text, max_chars, tables);
        for p in parts {
            chunks.push(ChunkMeta {
                content: p,
                section_idx: Some(idx),
                level: rs.level,
                path: rs.path.clone(),
                line_start: 0,
                line_end: 0,
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
    if is_minified(text) {
        return materialize(vec![root_section(text)], max_chars);
    }
    match file_ext(filename).as_str() {
        "yaml" | "yml" => chunk_yaml_with_sections(text),
        "json" | "jsonc" => materialize(split_json_sections(text), max_chars),
        "toml" => materialize(split_toml_sections(text), max_chars),
        _ => materialize(vec![root_section(text)], max_chars),
    }
}

pub fn chunk_yaml_with_sections(text: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    materialize(split_yaml_sections(text), max_chars)
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
    for line in text.split('\n') {
        let found = if symbols >= 500 { None } else { detect_yaml_key(line) };
        match found {
            Some((indent, title)) => {
                symbols += 1;
                while stack.last().map(|&(k, _)| k >= indent).unwrap_or(false) {
                    stack.pop();
                }
                let parent = stack.last().map(|&(_, idx)| idx);
                let level = symbol_level(&raw, parent);
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line);
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
    for line in text.split('\n') {
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
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line);
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
    for line in text.split('\n') {
        if symbols < 500 {
            if let Some(name) = re_table.captures(line).map(|cap| cap[1].trim().to_string()) {
                if !name.is_empty() {
                    symbols += 1;
                    let idx = push_symbol(&mut raw, None, truncate_title(&name), 1, line);
                    table = Some(idx);
                    current = Some(idx);
                    continue;
                }
            }
            if let Some(key) = re_key.captures(line).map(|cap| unquote_toml_key(cap[1].trim())) {
                if !key.is_empty() {
                    symbols += 1;
                    let level = symbol_level(&raw, table);
                    let idx = push_symbol(&mut raw, table, truncate_title(&key), level, line);
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
    if is_minified(text) {
        return materialize(vec![root_section(text)], max_chars);
    }
    let raw = match code_family(filename) {
        CodeFamily::Python => split_python_symbols(text),
        CodeFamily::Brace => split_brace_symbols(text, filename),
        CodeFamily::Fallback => vec![root_section(text)],
    };
    materialize(raw, max_chars)
}

fn split_python_symbols(text: &str) -> Vec<RawSection> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^(\s*)(?:async\s+)?(class|def)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
    });
    let mut raw: Vec<RawSection> = Vec::new();
    let mut stack: Vec<(i64, usize)> = Vec::new();
    for line in text.split('\n') {
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
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line);
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
    for line in text.split('\n') {
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
                let idx = push_symbol(&mut raw, parent, truncate_title(&title), level, line);
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
        let raw = "Title\r\n\x00\x0bBody\u{200b} text\t\tspaced  \n42\nPage 3\n3 of 10\n- 12 -\n— 7 —\nSection 42 stays\n42 things stay\n12345 stays\n\n\nTail\u{feff}";
        let cleaned = clean_doc_text(raw);
        assert!(cleaned.contains("Title\nBody text spaced"), "{cleaned:?}");
        assert!(!cleaned.contains("\u{200b}") && !cleaned.contains('\u{feff}'));
        for dropped in ["\n42\n", "Page 3", "3 of 10", "- 12 -", "— 7 —"] {
            assert!(!cleaned.contains(dropped), "{cleaned:?}");
        }
        for kept in ["Section 42 stays", "42 things stay", "12345 stays", "Tail"] {
            assert!(cleaned.contains(kept), "{cleaned:?}");
        }
        assert!(!cleaned.contains("\n\n\n"));
    }
    #[test]
    fn clean_doc_text_strips_repeated_headers() {
        let page = "Acme Confidential\nBody line one.\nBody line two.";
        let raw = format!("{page}\n{page}\n{page}\nFinal words here.");
        let cleaned = clean_doc_text(&raw);
        assert!(!cleaned.contains("Acme Confidential"), "{cleaned:?}");
        assert!(cleaned.contains("Final words here."), "{cleaned:?}");
        // twice is not a pattern — kept.
        let raw2 = "Acme Confidential\nBody.\nAcme Confidential\nOther.";
        assert!(clean_doc_text(raw2).contains("Acme Confidential"));
        // long lines never count toward repetition.
        let long = format!("{}\n{}\n{}\nshort", "L".repeat(121), "L".repeat(121), "L".repeat(121));
        assert!(clean_doc_text(&long).contains(&"L".repeat(121)));
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
