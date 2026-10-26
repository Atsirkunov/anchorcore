//! R36.2: rule-level sections for YAML / JSON / JSONC / TOML (supersedes the
//! R33.4 every-key splitters).
//!
//! Keys are chunk-level, rules are section-level:
//!
//! - **Sections** are top-level keys (TOML: tables) — or, when a top-level
//!   value is a list of mappings ("the list is the main content"), one
//!   section per list entry under the key. The rule is deterministic: the
//!   key's direct children are all list entries, there are 2..=500 of them,
//!   and at least half are mappings. Root-level lists (YAML/JSON) and TOML
//!   `[[name]]` runs follow the same rule. A run of consecutive top-level
//!   scalar keys (`version: 3`, `name = "x"`) shares one section so flat
//!   files don't produce one tiny section per line.
//! - **Header promotion**: an entry's title is its identifying scalars
//!   (`id`, `name`, `tenant`, `settle_code`, `item_type`, `*_id`, `*_code`,
//!   `*_type`, … — camelCase and kebab-case normalised), at most three,
//!   values ≤ 32 chars, in source order; without one, the first short scalar
//!   (booleans skipped); without that, `#N`. Keyed sections append their
//!   identifying children in parentheses. Titles become breadcrumbs, and the
//!   breadcrumb heads every chunk's embedding input (`embed_input`). The
//!   promoted pairs also land in the section's `meta` (`{"promoted": …}`).
//! - **Chunks** group a section's keys into 500–800-char runs (a run under
//!   500 may grow to the hard cap rather than stand alone); a key over the
//!   hard cap splits along its own sub-keys, else by lines. Chunks are exact
//!   source line ranges (blank edge lines trimmed) with their own 1-based
//!   `line_start`/`line_end`, so every non-blank line lands in exactly one
//!   chunk, verbatim, in order.
//!
//! Leading lines (comments, `{`, `---`) fold into the first section; a
//! comment run directly above a key belongs to that key; closing braces and
//! trailing comments stay with the unit they follow. Detection only — the
//! bytes are never rewritten.

use std::sync::OnceLock;

use regex::Regex;

use crate::chunking::{ChunkMeta, SectionMeta};

/// Key-group target window (R36.2 DoD).
const TARGET_MIN: usize = 500;
const TARGET_MAX: usize = 800;
/// Past this many entries a list stays one section (chunks still group by
/// entry) — a 50k-row data file must not become 50k sections.
const MAX_ENTRY_SECTIONS: usize = 500;
const PROMOTE_MAX_VALUE: usize = 32;
const PROMOTE_MAX_PAIRS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// A mapping key (`name:`, `"name":`, `name =`, TOML `[table]`).
    Key,
    /// A list entry (`- `, a value inside a JSON array, TOML `[[name]]`).
    Item,
    /// Blank, comment, closer, or value continuation.
    Other,
}

#[derive(Debug, Clone)]
struct Line {
    kind: Kind,
    depth: usize,
    indent: usize,
    blank: bool,
    comment: bool,
    /// Key name (TOML `[[name]]`: the list name; YAML/JSON items: empty).
    name: String,
    /// Inline scalar of a key line.
    scalar: Option<String>,
    /// Item lines: scalar `key: value` pairs written on the entry line itself.
    inline: Vec<(String, String)>,
    /// Item lines: the entry is a mapping (YAML `- k:`, JSON `{`, TOML).
    opens_map: bool,
}

impl Line {
    fn other(indent: usize, blank: bool, comment: bool) -> Self {
        Line { kind: Kind::Other, depth: 0, indent, blank, comment, name: String::new(), scalar: None, inline: Vec::new(), opens_map: false }
    }

    fn key(depth: usize, indent: usize, name: String, scalar: Option<String>) -> Self {
        Line { kind: Kind::Key, depth, indent, blank: false, comment: false, name, scalar, inline: Vec::new(), opens_map: false }
    }

    fn item(depth: usize, indent: usize, inline: Vec<(String, String)>, opens_map: bool) -> Self {
        Line { kind: Kind::Item, depth, indent, blank: false, comment: false, name: String::new(), scalar: None, inline, opens_map }
    }

    fn trivia(&self) -> bool {
        self.blank || self.comment
    }
}

/// A structural line plus the line range it owns (`start..end`, end
/// exclusive). Children tile `line + 1 .. end`.
#[derive(Debug)]
struct Node {
    line: usize,
    start: usize,
    end: usize,
    children: Vec<Node>,
}

/// Formats this splitter owns (by extension).
pub fn handles(ext: &str) -> bool {
    matches!(ext, "yaml" | "yml" | "json" | "jsonc" | "toml")
}

/// Pipeline entry for a config file already known to be [`handles`].
pub fn chunk_config(text: &str, ext: &str, hard_cap: usize) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let src: Vec<&str> = text.split('\n').collect();
    let lines = match ext {
        "json" | "jsonc" => classify_json(&src),
        "toml" => classify_toml(&src),
        _ => classify_yaml(&src),
    };
    let doc = Doc { src: &src, lines: &lines, hard_cap };
    let units = build_nodes(&lines, 0, src.len());
    if units.is_empty() {
        return doc.root_only();
    }
    let mut out = Out::default();
    plan_sections(&doc, &units, &mut out);
    (out.sections, out.chunks)
}

struct Doc<'a> {
    src: &'a [&'a str],
    lines: &'a [Line],
    hard_cap: usize,
}

#[derive(Default)]
struct Out {
    sections: Vec<SectionMeta>,
    chunks: Vec<ChunkMeta>,
}

// ---- tree ----

/// Sibling nodes over `lo..hi`: the shallowest structural lines are heads;
/// each owns everything up to the next head. The first head absorbs the
/// leading lines, and a comment run directly above a later head moves to it.
fn build_nodes(lines: &[Line], lo: usize, hi: usize) -> Vec<Node> {
    let structural: Vec<usize> = (lo..hi).filter(|&i| lines[i].kind != Kind::Other).collect();
    let Some(min_depth) = structural.iter().map(|&i| lines[i].depth).min() else {
        return Vec::new();
    };
    let heads: Vec<usize> = structural.into_iter().filter(|&i| lines[i].depth == min_depth).collect();
    let starts: Vec<usize> = heads
        .iter()
        .enumerate()
        .map(|(n, &h)| if n == 0 { lo } else { leading_start(lines, h, heads[n - 1] + 1) })
        .collect();
    heads
        .iter()
        .enumerate()
        .map(|(n, &h)| {
            let end = starts.get(n + 1).copied().unwrap_or(hi);
            Node { line: h, start: starts[n], end, children: build_nodes(lines, h + 1, end) }
        })
        .collect()
}

/// First line of the comment run (blank lines inside it included, blank
/// lines at its top excluded) directly above `head`, never above `floor`.
/// Comments indented deeper than the head stay with the block before it.
fn leading_start(lines: &[Line], head: usize, floor: usize) -> usize {
    let mut j = head;
    while j > floor && lines[j - 1].trivia() && (lines[j - 1].blank || lines[j - 1].indent <= lines[head].indent) {
        j -= 1;
    }
    while j < head && lines[j].blank {
        j += 1;
    }
    j
}

fn is_mapping_entry(lines: &[Line], node: &Node) -> bool {
    let l = &lines[node.line];
    l.kind == Kind::Item && (l.opens_map || node.children.iter().any(|c| lines[c.line].kind == Kind::Key))
}

/// The "list is the main content" rule over a run of sibling entries.
fn is_entry_list(lines: &[Line], entries: &[Node]) -> bool {
    if entries.len() < 2 || entries.len() > MAX_ENTRY_SECTIONS {
        return false;
    }
    if entries.iter().any(|e| lines[e.line].kind != Kind::Item) {
        return false;
    }
    let maps = entries.iter().filter(|e| is_mapping_entry(lines, e)).count();
    maps * 2 >= entries.len()
}

fn is_scalar_key(lines: &[Line], node: &Node) -> bool {
    let l = &lines[node.line];
    l.kind == Kind::Key && node.children.is_empty() && l.scalar.is_some()
}

// ---- sections ----

fn plan_sections(doc: &Doc, units: &[Node], out: &mut Out) {
    let mut i = 0;
    while i < units.len() {
        let line = &doc.lines[units[i].line];
        let run = match line.kind {
            Kind::Item => run_len(units, i, |n| doc.lines[n.line].kind == Kind::Item && doc.lines[n.line].name == line.name),
            _ if is_scalar_key(doc.lines, &units[i]) => run_len(units, i, |n| is_scalar_key(doc.lines, n)),
            _ => 1,
        };
        let group = &units[i..i + run];
        match line.kind {
            Kind::Item => plan_item_run(doc, group, out),
            _ if run > 1 || is_scalar_key(doc.lines, &units[i]) => plan_scalar_run(doc, group, out),
            _ => plan_key(doc, &units[i], out),
        }
        i += run;
    }
}

fn run_len(units: &[Node], from: usize, same: impl Fn(&Node) -> bool) -> usize {
    units[from..].iter().take_while(|n| same(n)).count().max(1)
}

/// Top-level list entries (YAML/JSON root list, TOML `[[name]]` run).
fn plan_item_run(doc: &Doc, run: &[Node], out: &mut Out) {
    let name = doc.lines[run[0].line].name.clone();
    let (start, end) = (run[0].start, run[run.len() - 1].end);
    if !is_entry_list(doc.lines, run) {
        let title = if name.is_empty() { "list".to_string() } else { name };
        let spans = run.iter().map(Span::of).collect();
        push_section(doc, out, Planned { title, parent: None, level: 1, start, end, promoted: Vec::new() }, spans);
        return;
    }
    let parent = (!name.is_empty()).then(|| {
        push_section(doc, out, Planned { title: name, parent: None, level: 1, start, end, promoted: Vec::new() }, Vec::new())
    });
    plan_entries(doc, run, parent, start, out);
}

/// One section per entry; the first entry also owns `head_start..` (the
/// list's key line and the comments above it).
fn plan_entries(doc: &Doc, entries: &[Node], parent: Option<usize>, head_start: usize, out: &mut Out) {
    let level = if parent.is_some() { 2 } else { 1 };
    for (n, entry) in entries.iter().enumerate() {
        let start = if n == 0 { head_start } else { entry.start };
        let promoted = entry_pairs(doc.lines, entry);
        let title = entry_title(doc.lines, entry, &promoted, n + 1);
        let mut spans = node_spans(entry);
        if let Some(first) = spans.first_mut() {
            first.start = start;
        }
        push_section(doc, out, Planned { title, parent, level, start, end: entry.end, promoted }, spans);
    }
}

fn plan_scalar_run(doc: &Doc, run: &[Node], out: &mut Out) {
    let names: Vec<&str> = run.iter().map(|n| doc.lines[n.line].name.as_str()).collect();
    let (start, end) = (run[0].start, run[run.len() - 1].end);
    let spans = run.iter().map(Span::of).collect();
    push_section(doc, out, Planned { title: names.join(", "), parent: None, level: 1, start, end, promoted: Vec::new() }, spans);
}

/// A top-level key: its own section, or a parent over per-entry sections
/// when its value is an entry list.
fn plan_key(doc: &Doc, node: &Node, out: &mut Out) {
    let key = doc.lines[node.line].name.clone();
    if is_entry_list(doc.lines, &node.children) {
        let parent = push_section(doc, out, Planned { title: key, parent: None, level: 1, start: node.start, end: node.end, promoted: Vec::new() }, Vec::new());
        plan_entries(doc, &node.children, Some(parent), node.start, out);
        return;
    }
    let promoted = identifying_pairs(child_pairs(doc.lines, node));
    let title = if promoted.is_empty() { key } else { format!("{key} ({})", format_pairs(&promoted)) };
    let mut spans = node_spans(node);
    if let Some(first) = spans.first_mut() {
        first.start = node.start;
    }
    push_section(doc, out, Planned { title, parent: None, level: 1, start: node.start, end: node.end, promoted }, spans);
}

struct Planned {
    title: String,
    parent: Option<usize>,
    level: i32,
    start: usize,
    end: usize,
    promoted: Vec<(String, String)>,
}

fn push_section(doc: &Doc, out: &mut Out, p: Planned, spans: Vec<Span>) -> usize {
    let title = truncate_title(&p.title);
    let path = match p.parent.map(|i| out.sections[i].path.clone()) {
        Some(pp) if !pp.is_empty() => format!("{pp} > {title}"),
        _ => title.clone(),
    };
    let (line_start, line_end) = doc.trimmed(p.start, p.end).map(|(s, e)| (s as i64 + 1, e as i64)).unwrap_or((p.start as i64 + 1, p.start as i64 + 1));
    let idx = out.sections.len();
    out.sections.push(SectionMeta {
        title,
        level: p.level,
        parent: p.parent,
        path: path.clone(),
        line_start,
        line_end,
        kind: "section".to_string(),
        meta: promoted_meta(&p.promoted),
    });
    for (s, e) in pack(doc, spans) {
        let Some((s, e)) = doc.trimmed(s, e) else { continue };
        out.chunks.push(ChunkMeta {
            content: doc.src[s..e].join("\n"),
            section_idx: Some(idx),
            level: p.level,
            path: path.clone(),
            line_start: s as i64 + 1,
            line_end: e as i64,
        });
    }
    idx
}

fn promoted_meta(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return "{}".to_string();
    }
    let map: serde_json::Map<String, serde_json::Value> =
        pairs.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
    serde_json::json!({ "promoted": map }).to_string()
}

fn truncate_title(title: &str) -> String {
    let t = title.trim();
    if t.chars().count() <= 80 {
        return t.to_string();
    }
    format!("{}...", t.chars().take(77).collect::<String>())
}

impl Doc<'_> {
    /// `s..e` without blank edge lines; `None` when nothing is left.
    fn trimmed(&self, mut s: usize, mut e: usize) -> Option<(usize, usize)> {
        while s < e && self.src[s].trim().is_empty() {
            s += 1;
        }
        while e > s && self.src[e - 1].trim().is_empty() {
            e -= 1;
        }
        (s < e).then_some((s, e))
    }

    fn chars(&self, s: usize, e: usize) -> usize {
        self.src[s..e].iter().map(|l| l.chars().count() + 1).sum()
    }

    /// No structure found (a bare scalar, comments only): one root section.
    fn root_only(&self) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
        let mut out = Out::default();
        let span = Span { start: 0, end: self.src.len(), node: None };
        push_section(self, &mut out, Planned { title: String::new(), parent: None, level: 0, start: 0, end: self.src.len(), promoted: Vec::new() }, vec![span]);
        (out.sections, out.chunks)
    }
}

// ---- header promotion ----

/// An entry's scalar pairs: the entry line's inline pairs, then its direct
/// scalar children, in source order.
fn entry_pairs(lines: &[Line], entry: &Node) -> Vec<(String, String)> {
    let mut pairs = lines[entry.line].inline.clone();
    pairs.extend(child_pairs(lines, entry));
    identifying_pairs(pairs)
}

fn child_pairs(lines: &[Line], node: &Node) -> Vec<(String, String)> {
    node.children
        .iter()
        .filter_map(|c| {
            let l = &lines[c.line];
            (l.kind == Kind::Key).then(|| l.scalar.clone().map(|v| (l.name.clone(), v))).flatten()
        })
        .collect()
}

fn identifying_pairs(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    pairs
        .into_iter()
        .filter(|(k, v)| is_identifying_key(k) && promotable_value(v))
        .take(PROMOTE_MAX_PAIRS)
        .collect()
}

fn entry_title(lines: &[Line], entry: &Node, promoted: &[(String, String)], ordinal: usize) -> String {
    if !promoted.is_empty() {
        return format_pairs(promoted);
    }
    let mut pairs = lines[entry.line].inline.clone();
    pairs.extend(child_pairs(lines, entry));
    pairs
        .iter()
        .find(|(_, v)| promotable_value(v) && !is_bool_word(v))
        .map(|(k, v)| format!("{k}: {v}"))
        .unwrap_or_else(|| format!("#{ordinal}"))
}

fn format_pairs(pairs: &[(String, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
}

/// `tenantId` / `settle-code` / `Item_Type` → `tenant_id` / `settle_code` /
/// `item_type`.
fn normalize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    let mut prev_lower = false;
    for c in key.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(match c {
            '-' | ' ' | '.' => '_',
            _ => c.to_ascii_lowercase(),
        });
    }
    out
}

fn is_identifying_key(key: &str) -> bool {
    const EXACT: &[&str] = &["id", "name", "key", "code", "slug", "title", "type", "kind", "tenant", "rule", "route"];
    const SUFFIX: &[&str] = &["_id", "_code", "_type", "_name", "_key", "_slug", "_tenant"];
    let k = normalize_key(key);
    EXACT.contains(&k.as_str()) || SUFFIX.iter().any(|s| k.ends_with(s)) || k.starts_with("tenant")
}

fn promotable_value(v: &str) -> bool {
    let v = v.trim();
    let len = v.chars().count();
    (1..=PROMOTE_MAX_VALUE).contains(&len)
        && !v.starts_with(['{', '[', '|', '>', '&', '*', '!'])
        && !matches!(v, "null" | "~")
}

fn is_bool_word(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off")
}

/// The value after `:` / `=`: a quoted string's inner text, else the bare
/// token up to a trailing comment or comma. `None` for empty values.
fn scalar_value(rest: &str) -> Option<String> {
    let r = rest.trim();
    let quote = r.chars().next().filter(|c| *c == '"' || *c == '\'');
    let value = match quote {
        Some(q) => quoted_inner(&r[1..], q)?,
        None => {
            let cut = r.find(" #").unwrap_or(r.len());
            r[..cut].trim().trim_end_matches(',').trim().to_string()
        }
    };
    (!value.is_empty()).then_some(value)
}

fn quoted_inner(after_open: &str, q: char) -> Option<String> {
    let mut out = String::new();
    let mut escaped = false;
    for c in after_open.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' && q == '"' {
            escaped = true;
        } else if c == q {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

// ---- chunk packing ----

/// A contiguous line range to pack, with the node it came from (so an
/// oversized unit can split along its sub-keys).
struct Span<'n> {
    start: usize,
    end: usize,
    node: Option<&'n Node>,
}

impl<'n> Span<'n> {
    fn of(node: &'n Node) -> Self {
        Span { start: node.start, end: node.end, node: Some(node) }
    }
}

/// A node's packing units: its children (the header line folds into the
/// first), or the node itself when it has none.
fn node_spans(node: &Node) -> Vec<Span<'_>> {
    if node.children.is_empty() {
        return vec![Span::of(node)];
    }
    let mut spans: Vec<Span> = node.children.iter().map(Span::of).collect();
    spans[0].start = node.start;
    spans
}

/// Greedy key grouping: extend while the run stays ≤ `TARGET_MAX`, or while
/// it is still under `TARGET_MIN` and stays ≤ the hard cap. A unit past the
/// hard cap stands alone, split by `split_oversized`.
fn pack(doc: &Doc, spans: Vec<Span>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, usize, usize)> = None;
    for span in spans {
        let len = doc.chars(span.start, span.end);
        if len > doc.hard_cap {
            out.extend(cur.take().map(|(s, e, _)| (s, e)));
            out.extend(split_oversized(doc, span));
            continue;
        }
        cur = match cur {
            Some((s, _, n)) if n + len <= TARGET_MAX || (n < TARGET_MIN && n + len <= doc.hard_cap) => Some((s, span.end, n + len)),
            Some((s, e, _)) => {
                out.push((s, e));
                Some((span.start, span.end, len))
            }
            None => Some((span.start, span.end, len)),
        };
    }
    out.extend(cur.map(|(s, e, _)| (s, e)));
    out
}

fn split_oversized(doc: &Doc, span: Span) -> Vec<(usize, usize)> {
    match span.node.filter(|n| !n.children.is_empty()) {
        Some(node) => {
            let mut spans = node_spans(node);
            spans[0].start = span.start;
            if let Some(last) = spans.last_mut() {
                last.end = span.end;
            }
            pack(doc, spans)
        }
        None => pack_lines(doc, span.start, span.end),
    }
}

/// Leaf fallback: consecutive lines up to `TARGET_MAX` (one line longer
/// than that stands alone — content is never cut mid-line).
fn pack_lines(doc: &Doc, start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut s, mut n) = (start, 0usize);
    for i in start..end {
        let len = doc.src[i].chars().count() + 1;
        if n > 0 && n + len > TARGET_MAX {
            out.push((s, i));
            s = i;
            n = 0;
        }
        n += len;
    }
    if s < end {
        out.push((s, end));
    }
    out
}

// ---- YAML ----

fn yaml_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^(?:"([^"\n]+)"|'([^'\n]+)'|([A-Za-z0-9_][A-Za-z0-9_.\-/]*))\s*:(?:\s+(.*)|$)"#).unwrap()
    })
}

/// `key: rest` — the colon must be followed by space/EOL, so `http://x` and
/// `12:30` never match.
fn yaml_pair(probe: &str) -> Option<(String, Option<String>, bool)> {
    let cap = yaml_key_re().captures(probe)?;
    let key = cap.get(1).or(cap.get(2)).or(cap.get(3))?.as_str().to_string();
    let rest = cap.get(4).map(|m| m.as_str()).unwrap_or("");
    let block = rest.trim_start().starts_with(['|', '>']);
    let scalar = if block { None } else { scalar_value(rest) };
    Some((key, scalar, block))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// YAML lines. Sequence entries nest one level inside whatever introduced
/// them (`depth = indent + 1`), so `key:\n- a` (dash at the key's indent)
/// still reads as the key's list. Block scalars (`|`, `>`) are content.
fn classify_yaml(src: &[&str]) -> Vec<Line> {
    let mut out = Vec::with_capacity(src.len());
    let mut block: Option<usize> = None;
    for raw in src {
        let indent = indent_of(raw);
        let probe = raw.trim_start_matches([' ', '\t']);
        let blank = probe.trim().is_empty();
        if let Some(b) = block {
            if blank || indent > b {
                out.push(Line::other(indent, blank, false));
                continue;
            }
            block = None;
        }
        let (line, opens_block) = yaml_line(probe, indent, blank);
        if opens_block {
            block = Some(indent);
        }
        out.push(line);
    }
    out
}

fn yaml_line(probe: &str, indent: usize, blank: bool) -> (Line, bool) {
    if blank {
        return (Line::other(indent, true, false), false);
    }
    if probe.starts_with('#') || probe.starts_with("---") || probe.starts_with("...") {
        return (Line::other(indent, false, true), false);
    }
    if probe == "-" || probe.starts_with("- ") || probe.starts_with("-\t") {
        let rest = probe[1..].trim_start();
        return match yaml_pair(rest) {
            Some((k, v, block)) => (Line::item(indent + 1, indent, v.map(|v| vec![(k, v)]).unwrap_or_default(), true), block),
            None => (Line::item(indent + 1, indent, Vec::new(), false), false),
        };
    }
    match yaml_pair(probe) {
        Some((k, v, block)) => (Line::key(indent, indent, k, v), block),
        None => (Line::other(indent, false, false), false),
    }
}

// ---- JSON / JSONC ----

fn json_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"^"((?:[^"\\]|\\.)*)"\s*:(.*)$"#).unwrap())
}

fn json_pairs_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""((?:[^"\\]|\\.)*)"\s*:\s*("(?:[^"\\]|\\.)*"|[^,{}\[\]\s]+)"#).unwrap())
}

/// JSON nesting state carried across lines (strings never span lines;
/// JSONC block comments may).
#[derive(Default)]
struct JsonState {
    stack: Vec<char>,
    in_block: bool,
}

/// JSON lines by real nesting depth (string- and comment-aware), so keys
/// and array entries are found by structure, not by indentation.
fn classify_json(src: &[&str]) -> Vec<Line> {
    let mut st = JsonState::default();
    src.iter()
        .map(|raw| {
            let in_comment = st.in_block;
            let probe = strip_json_comments(raw, &mut st);
            let at = Snapshot { depth: st.stack.len(), container: st.stack.last().copied() };
            let line = json_line(&probe, raw, in_comment, &at);
            advance_json_stack(&probe, &mut st.stack);
            line
        })
        .collect()
}

/// Depth + innermost container at the START of a line.
struct Snapshot {
    depth: usize,
    container: Option<char>,
}

fn json_line(probe: &str, raw: &str, in_comment: bool, at: &Snapshot) -> Line {
    let indent = indent_of(raw);
    let t = probe.trim();
    if t.is_empty() {
        return Line::other(indent, raw.trim().is_empty(), !raw.trim().is_empty() || in_comment);
    }
    if let Some(cap) = json_key_re().captures(t) {
        let key = cap[1].replace("\\\"", "\"").replace("\\\\", "\\");
        if !key.is_empty() {
            return Line::key(at.depth, indent, key, scalar_value(&cap[2]));
        }
    }
    if at.container == Some('[') && !t.starts_with(']') {
        let inline = json_pairs_re()
            .captures_iter(t)
            .filter_map(|c| scalar_value(&c[2]).map(|v| (c[1].to_string(), v)))
            .collect();
        return Line::item(at.depth, indent, inline, t.starts_with('{'));
    }
    Line::other(indent, false, false)
}

/// The line with JSONC comments (`//`, `/* */`) removed, string-aware;
/// block-comment state carries to the next line.
fn strip_json_comments(raw: &str, st: &mut JsonState) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_str = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if st.in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                st.in_block = false;
            }
            continue;
        }
        if in_str {
            out.push(c);
            escaped = !escaped && c == '\\';
            in_str = escaped || c != '"';
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => break,
            ('/', Some('*')) => {
                chars.next();
                st.in_block = true;
            }
            _ => {
                in_str = c == '"';
                out.push(c);
            }
        }
    }
    out
}

/// Advance the nesting stack over a comment-free probe (strings skipped).
fn advance_json_stack(probe: &str, stack: &mut Vec<char>) {
    let mut in_str = false;
    let mut escaped = false;
    for c in probe.chars() {
        if in_str {
            escaped = !escaped && c == '\\';
            in_str = escaped || c != '"';
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => stack.push(c),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }
}

// ---- TOML ----

fn toml_res() -> (&'static Regex, &'static Regex) {
    static TABLE: OnceLock<Regex> = OnceLock::new();
    static KEY: OnceLock<Regex> = OnceLock::new();
    (
        TABLE.get_or_init(|| Regex::new(r"^\s*(\[{1,2})\s*([^\]#]+?)\s*\]{1,2}\s*(?:#.*)?$").unwrap()),
        KEY.get_or_init(|| Regex::new(r"^\s*([^#\[=\s][^=]*?)\s*=(.*)$").unwrap()),
    )
}

/// TOML context while classifying: the open `[[name]]` run, whether a
/// sub-table of that entry is open, and multi-line value state.
#[derive(Default)]
struct TomlState {
    list: Option<String>,
    in_table: bool,
    nested: bool,
    ml_string: bool,
    array_depth: i32,
}

/// TOML lines. Tables and `[[name]]` entries are depth 0 (top-level
/// units); `[[name]]` entries are items grouped by name, and their
/// `[name.sub]` / `[[name.sub]]` tables are keys inside the entry.
fn classify_toml(src: &[&str]) -> Vec<Line> {
    let mut st = TomlState::default();
    src.iter().map(|raw| toml_line(raw, &mut st)).collect()
}

fn toml_line(raw: &str, st: &mut TomlState) -> Line {
    let indent = indent_of(raw);
    let t = raw.trim();
    if st.ml_string || st.array_depth > 0 {
        toml_continue(t, st);
        return Line::other(indent, t.is_empty(), false);
    }
    if t.is_empty() || t.starts_with('#') {
        return Line::other(indent, t.is_empty(), !t.is_empty());
    }
    let (re_table, re_key) = toml_res();
    if let Some(cap) = re_table.captures(raw) {
        return toml_table(cap[2].trim(), cap[1].len() == 2, indent, st);
    }
    let Some(cap) = re_key.captures(raw) else {
        return Line::other(indent, false, false);
    };
    let key = unquote_toml_key(cap[1].trim());
    let rest = cap[2].to_string();
    toml_open_value(&rest, st);
    let depth = usize::from(st.in_table || st.list.is_some()) + usize::from(st.nested);
    let scalar = if st.ml_string || st.array_depth > 0 { None } else { scalar_value(&rest) };
    Line::key(depth, indent, key, scalar)
}

fn toml_table(name: &str, array: bool, indent: usize, st: &mut TomlState) -> Line {
    let inside_entry = st.list.as_deref().is_some_and(|l| name.starts_with(&format!("{l}.")));
    if inside_entry {
        st.nested = true;
        return Line::key(1, indent, name.to_string(), None);
    }
    st.nested = false;
    if array {
        st.list = Some(name.to_string());
        st.in_table = false;
        let mut line = Line::item(0, indent, Vec::new(), true);
        line.name = name.to_string();
        return line;
    }
    st.list = None;
    st.in_table = true;
    Line::key(0, indent, name.to_string(), None)
}

/// A value that opens a multi-line string or array keeps the following
/// lines as content until it closes.
fn toml_open_value(rest: &str, st: &mut TomlState) {
    let r = rest.trim();
    if (r.starts_with("\"\"\"") || r.starts_with("'''")) && (r.len() < 6 || !(r[3..].contains("\"\"\"") || r[3..].contains("'''"))) {
        st.ml_string = true;
        return;
    }
    if r.starts_with('[') {
        st.array_depth = bracket_balance(r);
    }
}

fn toml_continue(t: &str, st: &mut TomlState) {
    if st.ml_string {
        st.ml_string = !(t.contains("\"\"\"") || t.contains("'''"));
        return;
    }
    st.array_depth += bracket_balance(t);
}

fn bracket_balance(s: &str) -> i32 {
    let mut in_str: Option<char> = None;
    let mut n = 0;
    for c in s.chars() {
        match (in_str, c) {
            (Some(q), _) if c == q => in_str = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => in_str = Some(c),
            (None, '#') => break,
            (None, '[') => n += 1,
            (None, ']') => n -= 1,
            _ => {}
        }
    }
    n.max(0)
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

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 1600;

    fn run(text: &str, ext: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
        chunk_config(text, ext, CAP)
    }

    fn paths(secs: &[SectionMeta]) -> Vec<&str> {
        secs.iter().map(|s| s.path.as_str()).collect()
    }

    fn nonblank(text: &str) -> Vec<&str> {
        text.split('\n').filter(|l| !l.trim().is_empty()).collect()
    }

    /// Every non-blank line exactly once, verbatim, in order.
    fn assert_lossless(text: &str, chunks: &[ChunkMeta]) {
        let joined: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
        let joined = joined.join("\n");
        assert_eq!(nonblank(&joined), nonblank(text));
    }

    #[test]
    fn json_rule_array_promotes_ids_and_keeps_lines() {
        let json = "{\n  \"version\": 2,\n  \"routes\": [\n    {\n      \"itemType\": \"payout\",\n      \"settleCode\": \"STL-0042\",\n      \"target\": \"ledger-eu-1\"\n    },\n    {\n      \"itemType\": \"refund\",\n      \"settleCode\": \"STL-0107\",\n      \"target\": \"ledger-eu-2\"\n    }\n  ]\n}\n";
        let (secs, chunks) = run(json, "json");
        assert_eq!(paths(&secs), vec!["version", "routes", "routes > itemType: payout, settleCode: STL-0042", "routes > itemType: refund, settleCode: STL-0107"]);
        let refund = chunks.iter().find(|c| c.content.contains("STL-0107")).unwrap();
        assert!(refund.content.starts_with("    {"), "{refund:?}");
        assert_eq!(refund.path, "routes > itemType: refund, settleCode: STL-0107");
        assert_lossless(json, &chunks);
    }

    #[test]
    fn json_root_array_entries_are_top_level_sections() {
        let json = "[\n  { \"id\": \"r1\", \"enabled\": true },\n  { \"id\": \"r2\", \"enabled\": false }\n]\n";
        let (secs, chunks) = run(json, "json");
        assert_eq!(paths(&secs), vec!["id: r1", "id: r2"]);
        assert!(secs.iter().all(|s| s.parent.is_none() && s.level == 1));
        assert_lossless(json, &chunks);
    }

    #[test]
    fn toml_array_of_tables_is_an_entry_list() {
        let toml = "title = \"routing\"\n\n[defaults]\ncurrency = \"EUR\"\n\n[[routes]]\nitem_type = \"payout\"\nsettle_code = \"STL-0042\"\n\n[routes.match]\ncountry = \"DE\"\n\n[[routes]]\nitem_type = \"refund\"\nsettle_code = \"STL-0107\"\ntags = [\n  \"eu\",\n  \"retail\",\n]\n";
        let (secs, chunks) = run(toml, "toml");
        assert_eq!(
            paths(&secs),
            vec!["title", "defaults", "routes", "routes > item_type: payout, settle_code: STL-0042", "routes > item_type: refund, settle_code: STL-0107"]
        );
        let payout = chunks.iter().find(|c| c.content.contains("STL-0042")).unwrap();
        assert!(payout.content.contains("[routes.match]\ncountry = \"DE\""), "sub-table stays in its entry: {payout:?}");
        let refund = chunks.iter().find(|c| c.content.contains("STL-0107")).unwrap();
        assert!(refund.content.contains("  \"retail\","), "multi-line array is content");
        assert_lossless(toml, &chunks);
    }

    #[test]
    fn yaml_root_list_and_fallback_titles() {
        // no identifying key → first short non-boolean scalar → `#N`
        let yaml = "- enabled: true\n  port: 8080\n- enabled: false\n  hosts:\n    - a\n    - b\n";
        let (secs, _) = run(yaml, "yaml");
        assert_eq!(paths(&secs), vec!["port: 8080", "#2"]);
    }

    #[test]
    fn yaml_scalar_lists_and_single_entries_stay_one_section() {
        let yaml = "hosts:\n  - a.example.com\n  - b.example.com\nonly:\n  - name: solo\n    port: 1\n";
        let (secs, _) = run(yaml, "yml");
        assert_eq!(paths(&secs), vec!["hosts", "only"]);
    }

    #[test]
    fn keyed_section_promotes_identifying_children() {
        let yaml = "acme_payouts:\n  tenant: acme-eu\n  settle-code: STL-0042\n  enabled: true\n  tenant_uuid: 9b1f6c2e-0000-4000-8000-00000000a001\n";
        let (secs, _) = run(yaml, "yaml");
        assert_eq!(paths(&secs), vec!["acme_payouts (tenant: acme-eu, settle-code: STL-0042)"]);
        assert!(secs[0].meta.contains("\"tenant\":\"acme-eu\""), "{}", secs[0].meta);
    }

    #[test]
    fn identifying_key_rules() {
        for k in ["id", "tenantId", "settle-code", "Item_Type", "rule_name", "tenant", "routeKey"] {
            assert!(is_identifying_key(k), "{k}");
        }
        for k in ["paid", "valid", "enabled", "amount_max", "description"] {
            assert!(!is_identifying_key(k), "{k}");
        }
        assert_eq!(normalize_key("settleCode"), "settle_code");
        assert!(!promotable_value("9b1f6c2e-0000-4000-8000-00000000a001"), "long opaque ids stay in content");
        assert!(!promotable_value("{ a: 1 }") && !promotable_value("null"));
        assert_eq!(scalar_value(" \"STL-0042\", # note"), Some("STL-0042".to_string()));
        assert_eq!(scalar_value(" ledger-eu-1 # primary"), Some("ledger-eu-1".to_string()));
        assert_eq!(scalar_value(" 3,"), Some("3".to_string()));
        assert_eq!(scalar_value("   "), None);
    }

    #[test]
    fn keys_group_into_target_window() {
        let mut yaml = String::from("settings:\n");
        for i in 0..60 {
            yaml.push_str(&format!("  option_{i:02}: value number {i:02} for the synthetic fixture\n"));
        }
        let (secs, chunks) = run(&yaml, "yaml");
        assert_eq!(secs.len(), 1);
        assert!(chunks.len() > 2);
        for c in &chunks[..chunks.len() - 1] {
            let n = c.content.chars().count();
            assert!((TARGET_MIN - 60..=TARGET_MAX).contains(&n), "chunk of {n} chars");
        }
        assert!(chunks.windows(2).all(|w| w[0].line_end < w[1].line_start), "chunk line ranges are disjoint and ordered");
        assert_eq!(chunks[0].line_start, 1);
        assert_lossless(&yaml, &chunks);
    }

    #[test]
    fn oversized_key_splits_along_sub_keys_then_lines() {
        let mut yaml = String::from("big:\n  nested:\n");
        for i in 0..80 {
            yaml.push_str(&format!("    leaf_{i:02}: some fairly long synthetic value {i:02}\n"));
        }
        yaml.push_str("  note: |\n");
        for i in 0..5 {
            yaml.push_str(&format!("    block line {i}: not a key\n"));
        }
        yaml.push_str("small: 1\n");
        let (secs, chunks) = run(&yaml, "yaml");
        assert_eq!(paths(&secs), vec!["big", "small"], "block-scalar lines never open keys");
        assert!(chunks.iter().all(|c| c.content.chars().count() <= CAP));
        assert!(chunks.iter().filter(|c| c.path == "big").count() > 2);
        assert_lossless(&yaml, &chunks);
    }

    #[test]
    fn comment_above_a_key_travels_with_it() {
        let yaml = "a:\n  x: 1\n  # deeper note stays with a\n# about b\nb:\n  y: 2\n";
        let (_, chunks) = run(yaml, "yaml");
        assert_eq!(chunks[0].content, "a:\n  x: 1\n  # deeper note stays with a");
        assert_eq!(chunks[1].content, "# about b\nb:\n  y: 2");
        assert_eq!((chunks[1].line_start, chunks[1].line_end), (4, 6));
    }

    #[test]
    fn structureless_text_is_one_root_section() {
        let (secs, chunks) = run("# only a comment\n", "yaml");
        assert_eq!(secs.len(), 1);
        assert_eq!(secs[0].title, "");
        assert_eq!(chunks.len(), 1);
    }
}
