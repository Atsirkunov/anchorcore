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
/// Powers `chunk_text` and `classify_windows` — same loop, one place.
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

/// Fixed-size chunks with overlap — mirrors `backend/app/chunking.py:14` `chunk_text`.
/// Operates on chars (Python str len) to match Python semantics and avoid UTF-8 panics.
pub fn chunk_text(text: &str) -> Vec<String> {
    let size = env_usize("ANCHOR_CHUNK_SIZE", CHUNK_SIZE);
    let overlap = env_usize("ANCHOR_CHUNK_OVERLAP", CHUNK_OVERLAP);
    slide_windows(text, size, size.saturating_sub(overlap).max(1))
}

/// Single heading classifier — was `is_heading` + `heading_level` (+ 3 level
/// helpers), which evaluated the same regexes twice. Returns the section level
/// (`Some`) or `None` for body text. Level rules are unchanged: §-forms carry
/// dots (`§434`→1, `4.2.1`-style→1+dots), article/annex/section words and
/// ALL-CAPS lines are level 1.
fn classify_heading(line: &str) -> Option<i32> {
    static RE_NUM: OnceLock<Regex> = OnceLock::new();
    static RE_CAPS: OnceLock<Regex> = OnceLock::new();
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
}

#[derive(Debug, Clone)]
pub struct ChunkMeta {
    pub content: String,
    pub section_idx: Option<usize>,
    pub level: i32,
    pub path: String,
}

pub fn chunk_document_with_sections(text: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut raw_sections: Vec<RawSection> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for line in lines {
        if let Some(level) = classify_heading(line) {
            let title = line.trim().to_string();
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
    materialize(raw_sections, max_chars)
}

fn split_section(section: &str, max_chars: usize) -> Vec<String> {
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
            out.extend(chunk_text(&para));
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
    let metas: Vec<SectionMeta> = raw_sections
        .iter()
        .map(|r| SectionMeta {
            title: r.title.clone(),
            level: r.level,
            parent: r.parent,
            path: r.path.clone(),
        })
        .collect();
    let mut chunks: Vec<ChunkMeta> = Vec::new();
    for (idx, rs) in raw_sections.iter().enumerate() {
        let text = rs.lines.join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let parts = split_section(&text, max_chars);
        for p in parts {
            chunks.push(ChunkMeta {
                content: p,
                section_idx: Some(idx),
                level: rs.level,
                path: rs.path.clone(),
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
/// innermost open symbol. Stack keys are indent (python) or brace depth.
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

/// Pipeline entry: code items split by symbols, everything else by headings.
pub fn chunk_for_doc_type(
    doc_type: &str,
    filename: &str,
    text: &str,
) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    if doc_type == "code" {
        chunk_code_with_sections(text, filename)
    } else {
        chunk_document_with_sections(text)
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
        let (secs_prose, _) = chunk_for_doc_type("meeting", "n.md", code);
        assert!(secs_prose.iter().all(|s| s.title.is_empty()));
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
}
