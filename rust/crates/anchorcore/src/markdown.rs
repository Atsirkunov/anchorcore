//! Markdown-aware chunking — blocks → section tree → block-packed chunks.
//!
//! Markdown files (`.md`/`.markdown`/`.mdx`) take this path instead of the
//! prose `§434`/ALL-CAPS splitter, which never recognized `#` headings and
//! collapsed every markdown file into one root section. The pipeline is:
//!
//! 1. `split_blocks` — one fence-aware pass into typed blocks with 1-based
//!    line ranges. A `#` inside fenced code or front matter never becomes a
//!    heading; thematic breaks are separators (dropped, not indexed).
//! 2. `build_sections` — headings open sections on a stack (level N closes
//!    every open level ≥ N); the root is titled by front-matter `title`,
//!    else the first H1 (which then doesn't open a duplicate section),
//!    else the filename stem.
//! 3. `pack_blocks` — whole-block packing (target ~900 chars, cap 1500)
//!    with per-kind oversize rules: sentences, list items, table row groups
//!    with repeated header, code kept whole to 3000 else split at blank
//!    lines with the fence repeated.
//!
//! No third-party parser: the tree builds offline, so this is a hand-rolled
//! CommonMark-subset splitter. Deliberate simplifications: indented code
//! blocks read as paragraphs, loose lists split at blank lines (packing
//! usually rejoins them), lazy quote continuations end the quote, and
//! reference-style link definitions are left in place.

use crate::chunking::{ChunkMeta, SectionMeta};

/// Packing sizes (proposal §3): target ~600–1000, hard cap ~1500, code kept
/// whole to a larger cap, tiny trailing chunks merged into their predecessor.
const MD_TARGET: usize = 900;
const MD_CAP: usize = 1500;
const MD_MIN: usize = 150;
const MD_CODE_CAP: usize = 3000;
/// Front matter is short by construction — a `---` pair further apart than
/// this is two thematic breaks, not metadata (avoids swallowing prose).
const FM_MAX_LINES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    FrontMatter,
    Heading,
    Paragraph,
    List,
    Table,
    Code,
    Quote,
    Html,
}

#[derive(Debug, Clone)]
struct Block {
    kind: BlockKind,
    /// Headings only: 1–6.
    level: i32,
    text: String,
    /// 1-based inclusive source lines.
    line_start: i64,
    line_end: i64,
    /// Code only: opening fence char + run length (for the repeat-on-split).
    fence: Option<(char, usize)>,
}

#[derive(Debug, Clone, Copy)]
struct Fence {
    ch: char,
    len: usize,
}

/// True for the extensions the folder/Drive connectors ingest as markdown.
pub fn is_markdown_filename(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit('.').next() {
        Some(ext) if ext.len() < base.len() => {
            matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx")
        }
        _ => false,
    }
}

/// Embedding/keyword input for a chunk (proposal §4): breadcrumb on top so
/// heading words match, link/IRL noise stripped. Stored `content` is never
/// touched — this only shapes what gets embedded.
pub fn embed_input(path: &str, content: &str) -> String {
    let clean = strip_md_links(content);
    if path.trim().is_empty() {
        clean
    } else {
        format!("{}\n{}", path.trim(), clean)
    }
}

/// `[text](url)` → `text`, `![alt](url)` → `alt`, `[text][ref]` → `text`.
/// Bare URLs are kept (their path segments carry signal). Parentheses nest
/// one level (wikipedia-style URLs); unmatched brackets pass through.
pub fn strip_md_links(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let bang = bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'[');
        let link = if bang || bytes[i] == b'[' {
            parse_link_target(text, bytes, if bang { i + 1 } else { i })
        } else {
            None
        };
        if let Some((end, label)) = link {
            out.push_str(label);
            i = end;
        } else {
            // `i` always rests on a char boundary: it advances by ASCII bytes
            // or by whole `parse_link_target` spans (also ASCII-delimited).
            let ch = text[i..].chars().next().unwrap_or('\u{FFFD}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Parse `[label](target)`/`[label][ref]` at `open` (the `[` byte). Returns
/// the first byte past the link and the label slice.
fn parse_link_target<'a>(text: &'a str, bytes: &[u8], open: usize) -> Option<(usize, &'a str)> {
    let mut depth = 0;
    let mut j = open + 1;
    let mut close = None;
    while j < bytes.len() {
        if bytes[j] == b'[' {
            depth += 1;
        } else if bytes[j] == b']' {
            if depth == 0 {
                close = Some(j);
                break;
            }
            depth -= 1;
        }
        j += 1;
    }
    let close = close?;
    let label = text.get(open + 1..close)?;
    match bytes.get(close + 1) {
        Some(b'(') => skip_parens(bytes, close + 2).map(|end| (end, label)),
        Some(b'[') => skip_bracket(bytes, close + 2).map(|end| (end, label)),
        _ => None,
    }
}

fn skip_parens(bytes: &[u8], mut j: usize) -> Option<usize> {
    let mut depth = 0;
    while j < bytes.len() {
        if bytes[j] == b'(' {
            depth += 1;
        } else if bytes[j] == b')' {
            if depth == 0 {
                return Some(j + 1);
            }
            depth -= 1;
        } else if bytes[j] == b'\n' {
            return None;
        }
        j += 1;
    }
    None
}

fn skip_bracket(bytes: &[u8], mut j: usize) -> Option<usize> {
    while j < bytes.len() {
        if bytes[j] == b']' {
            return Some(j + 1);
        }
        if bytes[j] == b'\n' {
            return None;
        }
        j += 1;
    }
    None
}

/// Pipeline entry: markdown text → section rows + packed chunks with line
/// ranges. `filename` titles the root when no front matter / H1 does.
pub fn chunk_markdown(text: &str, filename: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let blocks = split_blocks(text);
    let total = text.split('\n').count().max(1) as i64;
    let sections = build_sections(&blocks, filename, total);
    materialize_md(&sections)
}

fn chars_len(s: &str) -> usize {
    s.chars().count()
}

// --- Block pass ---

fn split_blocks(text: &str) -> Vec<Block> {
    let normalized = text
        .strip_prefix('\u{FEFF}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    if let Some(end) = front_matter_end(&lines) {
        out.push(make_block(BlockKind::FrontMatter, &lines, 0, end + 1, 0, None));
        i = end + 1;
    }
    let mut fence: Option<(Fence, usize)> = None;
    while i < lines.len() {
        if let Some((f, start)) = fence {
            if is_fence_close(lines[i], &f) {
                out.push(code_block(&lines, start, i, f));
                fence = None;
            }
            i += 1;
            continue;
        }
        if let Some(f) = fence_open(lines[i]) {
            fence = Some((f, i));
            i += 1;
            continue;
        }
        if lines[i].trim().is_empty() || is_thematic_break(lines[i]) {
            i += 1;
            continue;
        }
        let (kind, level, end) = scan_block(&lines, i);
        out.push(make_block(kind, &lines, i, end, level, None));
        i = end;
    }
    // An unclosed fence runs to EOF — content is never dropped for it.
    if let Some((f, start)) = fence {
        if start < lines.len() {
            out.push(code_block(&lines, start, lines.len() - 1, f));
        }
    }
    strip_toc(out)
}

/// Classify the non-blank, non-fence line at `i` and consume its block.
/// Order matters: Setext pairs beat tables (`foo` + `---` is an H2, the GFM
/// reading), and lists/quotes/HTML beat Setext (their marker lines are not
/// titles even when an `===` line follows).
fn scan_block(lines: &[&str], i: usize) -> (BlockKind, i32, usize) {
    let line = lines[i];
    if let Some((level, _)) = atx_heading(line) {
        return (BlockKind::Heading, level, i + 1);
    }
    if is_list_item(line) {
        return (BlockKind::List, 0, consume_list(lines, i));
    }
    if is_quote(line) {
        return (BlockKind::Quote, 0, consume_while(lines, i, is_quote));
    }
    if is_html_start(line) {
        return (BlockKind::Html, 0, consume_html(lines, i));
    }
    if let Some(level) = setext_level(lines.get(i + 1)) {
        return (BlockKind::Heading, level, i + 2);
    }
    if is_table_start(lines, i) {
        return (BlockKind::Table, 0, consume_table(lines, i));
    }
    (BlockKind::Paragraph, 0, consume_paragraph(lines, i))
}

fn make_block(kind: BlockKind, lines: &[&str], start: usize, end: usize, level: i32, fence: Option<(char, usize)>) -> Block {
    Block {
        kind,
        level,
        text: lines[start..end].join("\n"),
        line_start: (start + 1) as i64,
        line_end: end as i64,
        fence,
    }
}

fn code_block(lines: &[&str], start: usize, end_incl: usize, f: Fence) -> Block {
    make_block(BlockKind::Code, lines, start, end_incl + 1, 0, Some((f.ch, f.len)))
}

/// Closing `---`/`+++` line index when the file opens with front matter.
/// Requires a `key: value`-shaped first body line within `FM_MAX_LINES` so a
/// leading thematic break can't swallow half the document as metadata.
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    let first = lines.first()?.trim();
    if first != "---" && first != "+++" {
        return None;
    }
    let rel = lines.iter().skip(1).position(|l| l.trim() == first)?;
    let end = rel + 1;
    if end > FM_MAX_LINES {
        return None;
    }
    let first_body = lines[1..end].iter().map(|l| l.trim()).find(|l| !l.is_empty())?;
    if !first_body.contains(':') {
        return None;
    }
    Some(end)
}

fn atx_heading(line: &str) -> Option<(i32, String)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == '#').count();
    if run == 0 || run > 6 {
        return None;
    }
    let rest: String = trimmed.chars().skip(run).collect();
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((run as i32, strip_closing_hashes(rest.trim())))
}

/// `## C#` keeps its `#` — only a space-separated closing run is stripped.
fn strip_closing_hashes(title: &str) -> String {
    let bytes = title.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'#' {
        end -= 1;
    }
    if end < bytes.len() && end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        title[..end].trim_end().to_string()
    } else {
        title.to_string()
    }
}

/// The underline half of a Setext heading (`===` → H1, `---` → H2).
fn setext_level(next: Option<&&str>) -> Option<i32> {
    let u = next?.trim();
    if u.is_empty() {
        return None;
    }
    if u.chars().all(|c| c == '=') {
        Some(1)
    } else if u.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

fn is_setext_underline(line: &str) -> bool {
    setext_level(Some(&line)).is_some()
}

fn is_thematic_break(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 3 {
        return false;
    }
    let ch = compact.chars().next().unwrap_or(' ');
    (ch == '*' || ch == '-' || ch == '_') && compact.chars().all(|c| c == ch)
}

fn fence_open(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == ch).count();
    if run < 3 {
        return None;
    }
    let info: String = trimmed.chars().skip(run).collect();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some(Fence { ch, len: run })
}

fn is_fence_close(line: &str, f: &Fence) -> bool {
    let trimmed = line.trim();
    let run = trimmed.chars().take_while(|c| *c == f.ch).count();
    run >= f.len && trimmed.chars().skip(run).all(|c| c.is_whitespace())
}

/// A delimiter row: `| --- | --- |`, `---`, `:--:`, … — every pipe-separated
/// cell is hyphens with optional edge colons.
fn is_table_delimiter(line: &str) -> bool {
    let inner = line.trim().trim_matches('|').trim();
    if inner.is_empty() {
        return false;
    }
    inner.split('|').all(|cell| {
        let core = cell.trim().trim_matches(':');
        !core.is_empty() && core.chars().all(|c| c == '-')
    })
}

fn is_table_start(lines: &[&str], i: usize) -> bool {
    lines.get(i + 1).map(|l| is_table_delimiter(l)).unwrap_or(false)
}

fn consume_table(lines: &[&str], i: usize) -> usize {
    let mut j = (i + 2).min(lines.len());
    while j < lines.len() {
        let line = lines[j];
        if line.trim().is_empty() || (!line.contains('|') && !is_table_delimiter(line)) {
            break;
        }
        j += 1;
    }
    j
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] == b'-' || bytes[0] == b'+' || bytes[0] == b'*' {
        return bytes.get(1).map(|c| *c == b' ' || *c == b'\t').unwrap_or(false);
    }
    let digits = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return false;
    }
    matches!(bytes.get(digits), Some(b'.') | Some(b')'))
        && bytes.get(digits + 1).map(|c| *c == b' ' || *c == b'\t').unwrap_or(false)
}

/// Items plus indented continuations; a blank line ends the block (loose
/// lists become adjacent List blocks — packing usually rejoins them).
fn consume_list(lines: &[&str], i: usize) -> usize {
    consume_while(lines, i, |l| is_list_item(l) || is_indented_cont(l))
}

fn is_indented_cont(line: &str) -> bool {
    if line.trim().is_empty() {
        return false;
    }
    line.starts_with("  ") || line.starts_with('\t')
}

fn is_quote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn consume_while(lines: &[&str], i: usize, keep: fn(&str) -> bool) -> usize {
    let mut j = i + 1;
    while j < lines.len() && keep(lines[j]) {
        j += 1;
    }
    j
}

fn is_html_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    matches!(chars.next(), Some('<'))
        && matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '/' || c == '!' || c == '?')
}

fn consume_html(lines: &[&str], i: usize) -> usize {
    if lines[i].trim_start().starts_with("<!--") {
        let mut j = i;
        while j < lines.len() && !lines[j].contains("-->") {
            j += 1;
        }
        return (j + 1).min(lines.len());
    }
    // Tag blocks run to the next blank line (`<div>` … `</div>`).
    let mut j = i + 1;
    while j < lines.len() && !lines[j].trim().is_empty() {
        j += 1;
    }
    j
}

fn consume_paragraph(lines: &[&str], i: usize) -> usize {
    let mut j = i + 1;
    while j < lines.len() {
        if lines[j].trim().is_empty() || is_paragraph_break(lines[j]) {
            break;
        }
        // `lines[j]` pairs with its successor as a Setext heading — leave it.
        if lines.get(j + 1).map(|n| is_setext_underline(n)).unwrap_or(false) {
            break;
        }
        j += 1;
    }
    j
}

fn is_paragraph_break(line: &str) -> bool {
    fence_open(line).is_some()
        || atx_heading(line).is_some()
        || is_thematic_break(line)
        || is_list_item(line)
        || is_quote(line)
        || is_html_start(line)
        || is_table_delimiter(line)
}

/// A top-of-file list whose every item links to an in-file anchor is the
/// file's own table of contents — index the sections, not the pointer list.
/// "Top" skips front matter and headings (`# Title` + anchor list is the
/// usual shape); the first real content block decides.
fn strip_toc(mut blocks: Vec<Block>) -> Vec<Block> {
    let first = blocks
        .iter()
        .position(|b| b.kind != BlockKind::FrontMatter && b.kind != BlockKind::Heading);
    if let Some(idx) = first {
        if blocks[idx].kind == BlockKind::List && is_anchor_list(&blocks[idx]) {
            blocks.remove(idx);
        }
    }
    blocks
}

fn is_anchor_list(block: &Block) -> bool {
    let lines: Vec<&str> = block.text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.len() >= 2 && lines.iter().all(|l| l.contains("](#"))
}

// --- Section tree ---

struct MdSection {
    title: String,
    level: i32,
    parent: Option<usize>,
    path: String,
    line_start: i64,
    line_end: i64,
    kind: &'static str,
    meta: String,
    /// The section's own heading source line + its line number — prepended to
    /// the first chunk so title words stay in stored text, not just the path.
    heading: Option<(String, i64)>,
    blocks: Vec<Block>,
}

fn build_sections(blocks: &[Block], filename: &str, total_lines: i64) -> Vec<MdSection> {
    let fm = blocks.iter().find(|b| b.kind == BlockKind::FrontMatter);
    let (fm_title, fm_map) = fm.map(parse_front_matter).unwrap_or_default();
    let h1_idx = blocks.iter().position(|b| b.kind == BlockKind::Heading && b.level == 1);
    let (root_title, donated) = root_title(&fm_title, blocks, h1_idx, filename);
    let mut sections = vec![MdSection {
        title: root_title.clone(),
        level: 0,
        parent: None,
        path: root_title,
        line_start: 1,
        line_end: total_lines,
        kind: "section",
        meta: root_meta(&fm_map, filename, blocks, h1_idx),
        heading: donated.map(|idx| (heading_source(&blocks[idx]), blocks[idx].line_start)),
        blocks: Vec::new(),
    }];
    if let Some(f) = fm {
        let path = format!("{} > Front matter", sections[0].path);
        sections.push(MdSection {
            title: "Front matter".to_string(),
            level: 1,
            parent: Some(0),
            path,
            line_start: f.line_start,
            line_end: f.line_end,
            kind: "front_matter",
            meta: fm_meta_json(&fm_map),
            heading: None,
            blocks: vec![f.clone()],
        });
    }
    let mut stack: Vec<usize> = vec![0];
    for (idx, block) in blocks.iter().enumerate() {
        if block.kind == BlockKind::FrontMatter || Some(idx) == donated {
            continue;
        }
        if block.kind == BlockKind::Heading {
            push_heading(&mut sections, &mut stack, block);
        } else {
            let top = stack.last().copied().unwrap_or(0);
            sections[top].blocks.push(block.clone());
        }
    }
    sections
}

/// Front-matter `title`, else the first H1 (which then doesn't open a
/// duplicate section), else the filename stem. Returns the title + the
/// donated H1 block index, if any.
fn root_title(fm_title: &Option<String>, blocks: &[Block], h1_idx: Option<usize>, filename: &str) -> (String, Option<usize>) {
    if let Some(t) = fm_title {
        // Same title in front matter and H1 (`title: ADR 005` + `# ADR 005`)
        // still donates — no `ADR 005 > ADR 005` duplicate section.
        let donated = h1_idx.filter(|idx| heading_title(&blocks[*idx]).trim().eq_ignore_ascii_case(t.trim()));
        return (t.clone(), donated);
    }
    if let Some(idx) = h1_idx {
        let title = heading_title(&blocks[idx]);
        if !title.is_empty() {
            return (title, Some(idx));
        }
    }
    (file_stem(filename), None)
}

fn file_stem(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let stem = match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    };
    let stem = stem.trim();
    if stem.is_empty() { "Untitled document".to_string() } else { stem.to_string() }
}

/// Structured doc metadata for the root node: ADR status/date plus the ADR
/// number from the filename or title (`ADR-005`, `adr_5`, `0005-…` when the
/// name mentions ADR). Powers "ADR 5" / "accepted ADRs" queries via the
/// `/sections` response + the indexed front-matter chunk.
fn root_meta(fm: &std::collections::HashMap<String, String>, filename: &str, blocks: &[Block], h1_idx: Option<usize>) -> String {
    let mut map = serde_json::Map::new();
    for key in ["status", "date"] {
        if let Some(v) = fm.get(key) {
            map.insert(key.to_string(), serde_json::Value::String(v.clone()));
        }
    }
    let title = h1_idx.map(|idx| heading_title(&blocks[idx])).unwrap_or_default();
    if let Some(n) = digits_after_adr(filename).or_else(|| digits_after_adr(&title)) {
        map.insert("adr_number".to_string(), serde_json::Value::String(n));
    }
    serde_json::Value::Object(map).to_string()
}

fn fm_meta_json(fm: &std::collections::HashMap<String, String>) -> String {
    let mut map = serde_json::Map::new();
    for (k, v) in fm {
        map.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    serde_json::Value::Object(map).to_string()
}

fn digits_after_adr(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("adr") {
        let abs = from + pos;
        let before_ok = abs == 0 || !lower.as_bytes()[abs - 1].is_ascii_alphanumeric();
        if before_ok {
            let rest = lower[abs + 3..].trim_start_matches(['-', '_', ' ', '.', ':', '#']);
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(digits.trim_start_matches('0').to_string()).filter(|d| !d.is_empty()).or(Some(digits));
            }
        }
        from = abs + 3;
    }
    None
}

fn parse_front_matter(block: &Block) -> (Option<String>, std::collections::HashMap<String, String>) {
    let mut map = std::collections::HashMap::new();
    let lines: Vec<&str> = block.text.lines().collect();
    for line in lines.iter().skip(1).take(lines.len().saturating_sub(2)) {
        let Some((k, v)) = line.split_once(':') else { continue };
        let key = k.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        map.insert(key, unquote(v.trim()));
    }
    let title = map.get("title").filter(|t| !t.is_empty()).cloned();
    (title, map)
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

fn heading_title(block: &Block) -> String {
    let first = block.text.lines().next().unwrap_or("").trim();
    if block.level >= 1 && block.text.contains('\n') && !first.starts_with('#') {
        // Setext: the title line stands alone above the `===`/`---` underline.
        return first.to_string();
    }
    atx_heading(first).map(|(_, t)| t).unwrap_or_else(|| first.to_string())
}

fn heading_source(block: &Block) -> String {
    block.text.lines().next().unwrap_or("").trim().to_string()
}

fn push_heading(sections: &mut Vec<MdSection>, stack: &mut Vec<usize>, block: &Block) {
    // A level-N heading closes every open section of level ≥ N; a popped
    // section ends on the line before the new heading (the root's level 0
    // never pops, so it always spans to EOF).
    while let Some(&top) = stack.last() {
        if sections[top].level >= block.level {
            sections[top].line_end = block.line_start - 1;
            stack.pop();
        } else {
            break;
        }
    }
    let parent = stack.last().copied().unwrap_or(0);
    let title = heading_title(block);
    let path = format!("{} > {}", sections[parent].path, title);
    sections.push(MdSection {
        title,
        level: block.level,
        parent: Some(parent),
        path,
        line_start: block.line_start,
        line_end: sections[parent].line_end,
        kind: "section",
        meta: "{}".to_string(),
        heading: Some((heading_source(block), block.line_start)),
        blocks: Vec::new(),
    });
    stack.push(sections.len() - 1);
}

fn materialize_md(sections: &[MdSection]) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let metas = sections
        .iter()
        .map(|s| SectionMeta {
            title: s.title.clone(),
            level: s.level,
            parent: s.parent,
            path: s.path.clone(),
            line_start: s.line_start,
            line_end: s.line_end,
            kind: s.kind.to_string(),
            meta: s.meta.clone(),
        })
        .collect();
    let mut chunks = Vec::new();
    for (idx, sec) in sections.iter().enumerate() {
        for (content, line_start, line_end) in pack_section(sec) {
            chunks.push(ChunkMeta {
                content,
                section_idx: Some(idx),
                level: sec.level,
                path: sec.path.clone(),
                line_start,
                line_end,
            });
        }
    }
    (metas, chunks)
}

// --- Packing: whole blocks, per-kind oversize splits ---

fn pack_section(sec: &MdSection) -> Vec<(String, i64, i64)> {
    let mut ordered: Vec<Block> = Vec::with_capacity(sec.blocks.len() + 1);
    if let Some((heading, line)) = &sec.heading {
        ordered.push(Block { kind: BlockKind::Paragraph, level: 0, text: heading.clone(), line_start: *line, line_end: *line, fence: None });
    }
    ordered.extend(sec.blocks.iter().cloned());
    pack_blocks(&ordered)
}

fn pack_blocks(blocks: &[Block]) -> Vec<(String, i64, i64)> {
    let mut out: Vec<(String, i64, i64)> = Vec::new();
    let mut cur: Vec<&Block> = Vec::new();
    let mut cur_len = 0;
    for block in blocks {
        let blen = chars_len(&block.text);
        if blen > MD_CAP {
            flush_blocks(&mut out, &mut cur);
            cur_len = 0;
            for piece in split_block(block) {
                out.push(piece);
            }
            continue;
        }
        if cur_len + blen + 2 > MD_TARGET && !cur.is_empty() {
            flush_blocks(&mut out, &mut cur);
            cur_len = 0;
        }
        cur_len += blen + 2;
        cur.push(block);
    }
    flush_blocks(&mut out, &mut cur);
    merge_short(&mut out);
    out
}

fn flush_blocks(out: &mut Vec<(String, i64, i64)>, cur: &mut Vec<&Block>) {
    if cur.is_empty() {
        return;
    }
    let text = cur.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n\n");
    out.push((text, cur[0].line_start, cur[cur.len() - 1].line_end));
    cur.clear();
}

/// A chunk under `MD_MIN` merges into its section-predecessor (a short intro
/// that stands alone is still kept — it labels the section).
fn merge_short(out: &mut Vec<(String, i64, i64)>) {
    let mut i = 1;
    while i < out.len() {
        if chars_len(&out[i].0) < MD_MIN {
            let (text, _, end) = out.remove(i);
            out[i - 1].0.push_str("\n\n");
            out[i - 1].0.push_str(&text);
            out[i - 1].2 = end;
        } else {
            i += 1;
        }
    }
}

fn split_block(block: &Block) -> Vec<(String, i64, i64)> {
    let texts = match block.kind {
        BlockKind::Code => split_code(block),
        BlockKind::Table => split_table(&block.text),
        BlockKind::List => split_list(&block.text),
        _ => split_prose(&block.text, MD_CAP),
    };
    texts.into_iter().map(|t| (t, block.line_start, block.line_end)).collect()
}

/// Paragraphs/quotes/HTML/front matter: sentence packing with one sentence
/// of overlap across forced splits (added only when it fits the cap).
fn split_prose(text: &str, cap: usize) -> Vec<String> {
    let mut units: Vec<String> = Vec::new();
    for sentence in split_sentences(text) {
        if chars_len(&sentence) > cap {
            units.extend(split_words(&sentence, cap));
        } else {
            units.push(sentence);
        }
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cur_len = 0;
    for unit in units {
        let ulen = chars_len(&unit);
        if cur_len + ulen + 1 > cap && !cur.is_empty() {
            pieces.push(cur.join(" "));
            cur.clear();
            cur_len = 0;
        }
        cur_len += ulen + 1;
        cur.push(unit);
    }
    if !cur.is_empty() {
        pieces.push(cur.join(" "));
    }
    add_overlap(pieces, cap)
}

fn add_overlap(mut pieces: Vec<String>, cap: usize) -> Vec<String> {
    for i in 1..pieces.len() {
        let prev_last = split_sentences(&pieces[i - 1]).pop().unwrap_or_default();
        if prev_last.is_empty() || pieces[i].starts_with(&prev_last) {
            continue;
        }
        if chars_len(&prev_last) + 1 + chars_len(&pieces[i]) <= cap {
            pieces[i] = format!("{} {}", prev_last, pieces[i]);
        }
    }
    pieces
}

fn split_sentences(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'.' || b == b'!' || b == b'?' {
            let boundary = bytes.get(i + 1).map(|c| c.is_ascii_whitespace()).unwrap_or(true);
            if boundary {
                let sentence = text.get(start..=i).unwrap_or("").trim();
                if !sentence.is_empty() {
                    out.push(sentence.to_string());
                }
                start = i + 1;
            }
        }
        i += 1;
    }
    let tail = text.get(start..).unwrap_or("").trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn split_words(text: &str, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0;
    for word in text.split_whitespace() {
        let wlen = chars_len(word);
        if wlen > cap {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            out.extend(char_chunks(word, cap));
            continue;
        }
        if cur_len + wlen + 1 > cap && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        if !cur.is_empty() {
            cur.push(' ');
            cur_len += 1;
        }
        cur.push_str(word);
        cur_len += wlen;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn char_chunks(word: &str, cap: usize) -> Vec<String> {
    word.chars()
        .collect::<Vec<_>>()
        .chunks(cap.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

/// Lists split at item boundaries; continuation lines stay with their item.
fn split_list(text: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for line in text.lines() {
        if is_list_item(line) || items.is_empty() {
            items.push(line.to_string());
        } else {
            items.last_mut().map(|last| {
                last.push('\n');
                last.push_str(line);
            });
        }
    }
    let mut units: Vec<String> = Vec::new();
    for item in items {
        if chars_len(&item) > MD_CAP {
            units.extend(split_words(&item, MD_CAP));
        } else {
            units.push(item);
        }
    }
    pack_units(&units, MD_CAP, "\n")
}

/// Tables split by row groups; the header + delimiter repeat on every piece
/// so each chunk still reads as a table.
fn split_table(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 {
        return split_words(text, MD_CAP);
    }
    let head = format!("{}\n{}", lines[0], lines[1]);
    let head_len = chars_len(&head);
    let mut pieces = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = head_len;
    for row in &lines[2..] {
        let rlen = chars_len(row);
        if rlen + 1 + head_len > MD_CAP {
            // A single cell wider than the cap: word-split it, still headed.
            for frag in split_words(row, MD_CAP.saturating_sub(head_len + 1).max(64)) {
                pieces.push(format!("{}\n{}", head, frag));
            }
            continue;
        }
        if cur_len + rlen + 1 > MD_CAP && !cur.is_empty() {
            pieces.push(format!("{}\n{}", head, cur.join("\n")));
            cur.clear();
            cur_len = head_len;
        }
        cur_len += rlen + 1;
        cur.push(row);
    }
    if !cur.is_empty() {
        pieces.push(format!("{}\n{}", head, cur.join("\n")));
    }
    if pieces.is_empty() {
        pieces.push(head);
    }
    pieces
}

fn pack_units(units: &[String], cap: usize, sep: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cur_len = 0;
    for unit in units {
        let ulen = chars_len(unit);
        if cur_len + ulen + sep.len() > cap && !cur.is_empty() {
            pieces.push(cur.join(sep));
            cur.clear();
            cur_len = 0;
        }
        cur_len += ulen + sep.len();
        cur.push(unit.clone());
    }
    if !cur.is_empty() {
        pieces.push(cur.join(sep));
    }
    pieces
}

/// Code stays whole to `MD_CODE_CAP`; beyond that it splits at blank lines
/// with the opening fence + language and a closing fence repeated per piece.
fn split_code(block: &Block) -> Vec<String> {
    if chars_len(&block.text) <= MD_CODE_CAP {
        return vec![block.text.clone()];
    }
    let inner: Vec<&str> = block.text.lines().collect();
    // Strip the stored fences; they are re-added per piece below.
    let body = if inner.len() >= 2 { &inner[1..inner.len() - 1] } else { &inner[..] };
    let mut groups: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in body {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                groups.push(cur.join("\n"));
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        groups.push(cur.join("\n"));
    }
    let (ch, len) = block.fence.unwrap_or(('`', 3));
    let lang = fence_lang(&block.text);
    let open = format!("{}{}", ch.to_string().repeat(len), lang);
    let close = ch.to_string().repeat(len);
    if groups.is_empty() {
        return vec![block.text.clone()];
    }
    let mut units: Vec<String> = Vec::new();
    for group in groups {
        let budget = MD_CAP.saturating_sub(chars_len(&open) + chars_len(&close) + 2).max(64);
        if chars_len(&group) > budget {
            units.extend(split_words(&group, budget));
        } else {
            units.push(group);
        }
    }
    pack_units(&units, MD_CAP.saturating_sub(chars_len(&open) + chars_len(&close) + 2).max(64), "\n\n")
        .into_iter()
        .map(|body| format!("{}\n{}\n{}", open, body, close))
        .collect()
}

fn fence_lang(code_text: &str) -> String {
    let first = code_text.lines().next().unwrap_or("");
    let trimmed = first.trim_start().trim_start_matches(['`', '~']);
    trimmed.split_whitespace().next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(sections: &[SectionMeta]) -> Vec<&str> {
        sections.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn fence_hash_never_opens_section() {
        let md = "# Doc\n\n```python\n# not a heading\nx = 1\n```\n\n## Real\nbody\n";
        let (secs, chunks) = chunk_markdown(md, "doc.md");
        assert_eq!(titles(&secs), vec!["Doc", "Real"]);
        assert!(chunks.iter().any(|c| c.content.contains("# not a heading")));
        assert_eq!(secs[1].path, "Doc > Real");
        assert_eq!(secs[1].level, 2);
    }

    #[test]
    fn hierarchy_nests_skipped_levels() {
        let md = "# Doc\n\n## Alpha\ntext\n\n### Deep\nmore\n\n## Beta\nend\n";
        let (secs, _) = chunk_markdown(md, "doc.md");
        assert_eq!(titles(&secs), vec!["Doc", "Alpha", "Deep", "Beta"]);
        assert_eq!(secs[2].parent, Some(1));
        assert_eq!(secs[2].path, "Doc > Alpha > Deep");
        // `#` then `###` nests under the `#` (no H2 in between).
        let (secs2, _) = chunk_markdown("# Doc\n\n### Only\nx\n", "d.md");
        assert_eq!(secs2[1].parent, Some(0));
        assert_eq!(secs2[1].path, "Doc > Only");
    }

    #[test]
    fn duplicate_titles_both_kept() {
        let md = "# Doc\n\n## Notes\none\n\n## Notes\ntwo\n";
        let (secs, _) = chunk_markdown(md, "d.md");
        assert_eq!(titles(&secs), vec!["Doc", "Notes", "Notes"]);
        assert_ne!(secs[1].line_start, secs[2].line_start);
    }

    #[test]
    fn root_title_precedence() {
        // Front-matter title beats H1.
        let (secs, _) = chunk_markdown("---\ntitle: FM Title\n---\n\n# H1 Title\nbody\n", "file.md");
        assert_eq!(secs[0].title, "FM Title");
        assert!(secs.iter().any(|s| s.title == "H1 Title"));
        // H1 beats filename (and donates: no duplicate section).
        let (secs, _) = chunk_markdown("# H1 Title\nbody\n", "file.md");
        assert_eq!(titles(&secs), vec!["H1 Title"]);
        // Filename stem as last resort (no extension, no dirs).
        let (secs, _) = chunk_markdown("just prose\nno headings\n", "docs/adr-005-cancel.md");
        assert_eq!(secs[0].title, "adr-005-cancel");
        assert_eq!(secs[0].path, "adr-005-cancel");
    }

    #[test]
    fn front_matter_node_and_meta() {
        let md = "---\ntitle: ADR 005\nstatus: accepted\ndate: 2026-01-02\n---\n\n# ADR 005\nbody\n";
        let (secs, chunks) = chunk_markdown(md, "adr-005.md");
        let fm = secs.iter().find(|s| s.kind == "front_matter").unwrap();
        assert_eq!(fm.parent, Some(0));
        assert!(fm.meta.contains("\"status\":\"accepted\""));
        assert!(secs[0].meta.contains("\"adr_number\":\"5\""));
        assert!(chunks.iter().any(|c| c.content.contains("status: accepted")));
    }

    #[test]
    fn leading_thematic_break_is_not_front_matter() {
        let md = "---\n\nSome intro without colons\n\n---\n\n# Doc\nbody\n";
        let (secs, _) = chunk_markdown(md, "d.md");
        assert!(secs.iter().all(|s| s.kind != "front_matter"));
    }

    #[test]
    fn top_anchor_list_toc_dropped() {
        let md = "# Doc\n\n- [Alpha](#alpha)\n- [Beta](#beta)\n\n## Alpha\ncontent here\n";
        let (secs, chunks) = chunk_markdown(md, "d.md");
        assert!(!chunks.iter().any(|c| c.content.contains("#alpha")));
        assert!(chunks.iter().any(|c| c.content.contains("content here")));
        assert_eq!(titles(&secs), vec!["Doc", "Alpha"]);
    }

    #[test]
    fn table_splits_repeat_header() {
        let mut md = "# Doc\n\n| a | b |\n| --- | --- |\n".to_string();
        for i in 0..60 {
            md.push_str(&format!("| row{:02} value text | more text here |\n", i));
        }
        let (_, chunks) = chunk_markdown(&md, "d.md");
        assert!(chunks.len() > 1);
        // Every table piece repeats the header (a heading-only first chunk
        // carries no rows and is exempt).
        for chunk in chunks.iter().filter(|c| c.content.contains("| row")) {
            assert!(chunk.content.contains("| a | b |"), "missing header in: {}", &chunk.content[..80.min(chunk.content.len())]);
            assert!(chunk.content.contains("| --- | --- |"));
        }
        assert!(chunks.iter().any(|c| c.content.contains("| row")));
    }

    #[test]
    fn code_stays_whole_then_splits_with_fence() {
        let small = format!("# Doc\n\n```rust\n{}\n```\n", "let x = 1;\n".repeat(20));
        let (_, chunks) = chunk_markdown(&small, "d.md");
        assert!(chunks.iter().any(|c| c.content.contains("```rust") && c.content.contains("let x = 1;")));
        let big = format!("# Doc\n\n```python\n{}\n```\n", ("x = 1\n\ny = 2\n\n".repeat(300)));
        let (_, chunks) = chunk_markdown(&big, "d.md");
        assert!(chunks.len() > 1);
        for chunk in chunks.iter().filter(|c| c.content.contains("x = 1")) {
            assert!(chunk.content.starts_with("# Doc\n\n```python") || chunk.content.starts_with("```python"), "{}", &chunk.content[..60.min(chunk.content.len())]);
            assert!(chunk.content.trim_end().ends_with("```"));
            assert!(chars_len(&chunk.content) <= MD_CAP + chars_len("# Doc\n\n"));
        }
    }

    #[test]
    fn line_ranges_track_source() {
        let md = "# Doc\n\nline three here\nline four here\n\n## Next\ntail\n";
        let (secs, chunks) = chunk_markdown(md, "d.md");
        assert_eq!((secs[0].line_start, secs[0].line_end), (1, 8));
        assert_eq!((secs[1].line_start, secs[1].line_end), (6, 8));
        let body = chunks.iter().find(|c| c.content.contains("line three")).unwrap();
        assert_eq!((body.line_start, body.line_end), (1, 4));
    }

    #[test]
    fn links_stripped_for_embed_only() {
        assert_eq!(strip_md_links("see [docs](https://x.test/a) now"), "see docs now");
        assert_eq!(strip_md_links("![alt text](img.png)"), "alt text");
        assert_eq!(strip_md_links("[a](http://x.test/y_(z))"), "a");
        assert_eq!(embed_input("Doc > Sec", "see [docs](https://x.test/a)"), "Doc > Sec\nsee docs");
        assert_eq!(embed_input("", "plain"), "plain");
        // Stored chunk text keeps the URL verbatim.
        let (_, chunks) = chunk_markdown("# Doc\n\nsee [docs](https://x.test/a) now\n", "d.md");
        assert!(chunks.iter().any(|c| c.content.contains("https://x.test/a")));
    }

    #[test]
    fn setext_and_csharp_and_unclosed_fence() {
        let md = "Title\n=====\n\n## C#\nbody\n\n```\ncode to eof\n";
        let (secs, chunks) = chunk_markdown(md, "d.md");
        assert_eq!(titles(&secs), vec!["Title", "C#"]);
        assert!(chunks.iter().any(|c| c.content.contains("code to eof")));
    }

    #[test]
    fn short_trailing_chunk_merges() {
        let para = "Sentence one is here. ".repeat(60);
        let md = format!("# Doc\n\n{}\n\ntiny tail\n", para);
        let (_, chunks) = chunk_markdown(&md, "d.md");
        assert!(!chunks.iter().any(|c| c.content.trim() == "tiny tail"));
        assert!(chunks.iter().any(|c| c.content.contains("tiny tail")));
    }

    #[test]
    fn markdown_filename_detection() {
        assert!(is_markdown_filename("docs/ADR-005.md"));
        assert!(is_markdown_filename("C:\\notes\\x.MARKDOWN"));
        assert!(is_markdown_filename("y.mdx"));
        assert!(!is_markdown_filename("code.rs"));
        assert!(!is_markdown_filename("notes.txt"));
        assert!(!is_markdown_filename("md"));
    }

    #[test]
    fn adr_number_forms() {
        assert_eq!(digits_after_adr("adr-005-cancel.md"), Some("5".to_string()));
        assert_eq!(digits_after_adr("ADR_12_x.md"), Some("12".to_string()));
        assert_eq!(digits_after_adr("ADR 5: Title"), Some("5".to_string()));
        assert_eq!(digits_after_adr("quadrant.md"), None);
        assert_eq!(digits_after_adr("plain.md"), None);
    }
}
