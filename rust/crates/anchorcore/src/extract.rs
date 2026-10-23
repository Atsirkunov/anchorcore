//! Document text extraction — one place for every format the connectors can
//! ingest (folder + Drive): notes, source code and config, HTML, PDF, Word,
//! PowerPoint, Excel (modern + legacy), OpenDocument, RTF and email.
//!
//! Pure functions over bytes: `extract_file` for the folder connector,
//! `extract_bytes` for the Drive connector (exports/downloads arrive as bytes
//! with a MIME type, not a path). Unsupported formats return `Err` — folder
//! treats that as "skipped" (kept out of purge, B57), Drive the same.
//!
//! R36.7 structure rules: docx `w:pStyle` Heading 1–6 (plus Title → 1),
//! ODT `text:h` (outline-level, missing → 1) and HTML `h1`–`h6` all emit
//! `#`–`######` markers that the chunker sections on; unknown or missing
//! styles degrade to plain paragraphs (text kept, never dropped). Tables
//! stay tab-separated rows (docx/odt/HTML) for the chunker's row-group
//! splitter. Auto-generated tables of contents are stripped, never indexed:
//! docx `TOC1`–`TOC9` paragraphs, ODT `text:table-of-content` subtrees, and
//! (`strip_toc`) non-heading lines that exactly repeat a heading title or
//! pair one with a TOC page-number artifact (tab number, dot leaders).
//! A page number joined by a plain space ("Chapter 2") is a cross-reference,
//! not a TOC entry, and is kept.

use std::io::{Cursor, Read};
use std::path::Path;

/// Plain-text kinds — decoded, not parsed: notes, docs, source code, config.
const TEXT_EXTS: &[&str] = &[
    // notes & docs
    "txt", "text", "md", "markdown", "mdx", "rst", "adoc", "org", "tex", "bib",
    // data & config
    "csv", "tsv", "json", "jsonc", "log", "yaml", "yml", "toml", "ini", "cfg", "conf", "properties", "xml",
    // source code — the most popular languages (repos are cloned and watched like any folder)
    "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte", "astro", "java", "kt", "kts", "scala",
    "groovy", "gradle", "cs", "fs", "fsx", "vb", "go", "rs", "rb", "php", "pl", "pm", "lua", "r", "jl", "swift",
    "m", "mm", "dart", "ex", "exs", "erl", "hrl", "hs", "clj", "cljs", "cljc", "c", "h", "cpp", "cc", "cxx",
    "hpp", "hh", "hxx", "sh", "bash", "zsh", "fish", "ps1", "psm1", "bat", "cmd", "sql", "css", "scss", "sass",
    "less", "cmake", "mk",
];

/// Extensionless repo files with well-known names — treated as text.
const TEXT_NAMES: &[&str] = &["dockerfile", "makefile", "justfile", "gemfile", "rakefile", ".gitignore", ".dockerignore"];

/// Source-code extensions (subset of `TEXT_EXTS`) — files typed as `code`
/// rather than prose, whatever the content looks like.
const CODE_EXTS: &[&str] = &[
    "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte", "astro", "java", "kt", "kts", "scala",
    "groovy", "gradle", "cs", "fs", "fsx", "vb", "go", "rs", "rb", "php", "pl", "pm", "lua", "r", "jl", "swift",
    "m", "mm", "dart", "ex", "exs", "erl", "hrl", "hs", "clj", "cljs", "cljc", "c", "h", "cpp", "cc", "cxx",
    "hpp", "hh", "hxx", "sh", "bash", "zsh", "fish", "ps1", "psm1", "bat", "cmd", "sql", "css", "scss", "sass",
    "less", "cmake", "mk",
];

/// Extensionless build files that are code, not config (`.gitignore` and
/// `.dockerignore` stay non-code).
const CODE_NAMES: &[&str] = &["dockerfile", "makefile", "justfile", "gemfile", "rakefile", "cmakelists.txt"];

/// Hierarchical config formats — chunked by key/table (R33.4), never
/// entity-extracted. Flat configs (ini/cfg/...) stay prose-chunked.
const CONFIG_EXTS: &[&str] = &["yaml", "yml", "json", "jsonc", "toml"];

/// Extensions that need a real extractor (not plain text).
const FORMAT_EXTS: &[&str] = &[
    "html", "htm", "pdf", "docx", "docm", "pptx", "pptm", "xlsx", "xlsm", "xlsb", "xls", "ods", "odt", "odp", "rtf",
    "eml", "drawio", "dio", "excalidraw", "svg", "vsdx", "vsdm",
];

/// Cap for a single decompressed archive part (docx/pptx/odt content XML).
const MAX_ZIP_ENTRY: u64 = 64 * 1024 * 1024;
/// Spreadsheet caps — keeps a giant export from flooding the index/classifier.
const MAX_SHEET_ROWS: usize = 5000;
const MAX_SHEET_COLS: usize = 64;

pub fn is_supported_ext(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    TEXT_EXTS.contains(&e.as_str()) || FORMAT_EXTS.contains(&e.as_str())
}

pub fn is_supported_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return is_supported_ext(ext);
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| TEXT_NAMES.contains(&n.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// True when a filename (absolute/relative path or bare name) is source
/// code — drives the `code` doc_type fast path in the pipeline.
pub fn is_code_filename(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    if CODE_NAMES.contains(&base.as_str()) {
        return true;
    }
    match base.rsplit('.').next() {
        Some(ext) if ext != base.as_str() => CODE_EXTS.contains(&ext),
        _ => false,
    }
}

pub fn is_code_path(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(is_code_filename).unwrap_or(false)
}

/// True when a filename is a hierarchical config file — drives the
/// key/table chunking family and the pipeline extraction skip (R33.4).
pub fn is_config_filename(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    if diagram_kind(&base).is_some() {
        return false;
    }
    match base.rsplit('.').next() {
        Some(ext) if ext != base.as_str() => CONFIG_EXTS.contains(&ext),
        _ => false,
    }
}

/// Diagram files (R46.2) by full name, so double extensions resolve before
/// the last one does: `x.excalidraw.json` is a scene, not a config, and
/// `x.drawio.svg` / `x.excalidraw.svg` take the SVG path, which reads the
/// embedded scene. The text renders as markdown (`diagram::render`).
pub fn diagram_kind(name: &str) -> Option<&'static str> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    if base.ends_with(".excalidraw") || base.ends_with(".excalidraw.json") {
        Some("excalidraw")
    } else if base.ends_with(".drawio") || base.ends_with(".dio") {
        Some("drawio")
    } else if base.ends_with(".svg") {
        Some("svg")
    } else if base.ends_with(".vsdx") || base.ends_with(".vsdm") {
        Some("vsdx")
    } else if base.starts_with("miro:") {
        // R46.5: Miro boards (external id `miro:<board>`) render through
        // the normal form, so they chunk as markdown like diagram files.
        Some("miro")
    } else {
        None
    }
}

/// Extract text from a file on disk (folder connector).
pub fn extract_file(path: &Path) -> Result<String, String> {
    extract_file_noted(path).0
}

/// `extract_file` plus the PDF catcher's report — `Some` only when a PDF
/// tripped the split-word detector (repaired or not). The pipeline turns
/// improved notes into `system_events` rows.
pub fn extract_file_noted(path: &Path) -> (Result<String, String>, Option<PdfQualityNote>) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let kind = diagram_kind(name).map(str::to_string).unwrap_or_else(|| {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_else(|| name.to_ascii_lowercase())
    });
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return (Err(format!("read {}: {}", path.display(), e)), None),
    };
    if kind == "pdf" {
        match extract_pdf_labeled(&bytes, &path.display().to_string()) {
            Ok((text, note)) => (Ok(text), note),
            Err(e) => (Err(e), None),
        }
    } else {
        (extract_bytes(&kind, &bytes), None)
    }
}

/// Labeled variant: the PDF catcher logs the file it repaired (Drive
/// passes the file name; the kind alone names nothing).
pub fn extract_bytes_labeled(kind: &str, bytes: &[u8], label: &str) -> Result<String, String> {
    let raw = kind.to_ascii_lowercase();
    if raw.trim_start_matches('.') == "pdf" {
        return extract_pdf_labeled(bytes, label).map(|(text, _)| text);
    }
    extract_bytes(kind, bytes)
}

/// Extract text from bytes given a kind (extension, or a well-known file name)
/// — the Drive connector resolves a MIME type to a kind first (`kind_for`).
pub fn extract_bytes(kind: &str, bytes: &[u8]) -> Result<String, String> {
    let raw = kind.to_ascii_lowercase();
    let k = raw.trim_start_matches('.');
    if TEXT_NAMES.contains(&raw.as_str()) || TEXT_EXTS.contains(&k) {
        return decode_text(bytes);
    }
    match k {
        "html" | "htm" => Ok(strip_toc(&strip_html(&html_images_to_md(&decode_text(bytes)?)))),
        "pdf" => extract_pdf(bytes),
        "docx" | "docm" => extract_docx(bytes),
        "pptx" | "pptm" => extract_pptx(bytes),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => extract_sheet(bytes),
        "odt" | "odp" => extract_odf(bytes),
        "rtf" => extract_rtf(bytes),
        "eml" => extract_eml(bytes),
        "drawio" | "dio" => crate::diagram_formats::drawio_to_text(&decode_text(bytes)?),
        "excalidraw" => crate::diagram_formats::excalidraw_to_text(&decode_text(bytes)?),
        "svg" => crate::diagram_formats::svg_to_text(&decode_text(bytes)?),
        "vsdx" | "vsdm" => crate::diagram_formats::vsdx_to_text(bytes),
        other => Err(format!("unsupported file type: {other:?}")),
    }
}

/// MIME type -> extractor kind (the formats `extract_bytes` handles).
const MIME_KINDS: &[(&str, &str)] = &[
    ("application/pdf", "pdf"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docx",
    ),
    ("application/vnd.ms-word.document.macroenabled.12", "docm"),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pptx",
    ),
    (
        "application/vnd.ms-powerpoint.presentation.macroenabled.12",
        "pptm",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsx",
    ),
    ("application/vnd.ms-excel.sheet.macroenabled.12", "xlsm"),
    (
        "application/vnd.ms-excel.sheet.binary.macroenabled.12",
        "xlsb",
    ),
    ("application/vnd.ms-excel", "xls"),
    ("application/vnd.oasis.opendocument.text", "odt"),
    ("application/vnd.oasis.opendocument.spreadsheet", "ods"),
    ("application/vnd.oasis.opendocument.presentation", "odp"),
    ("application/vnd.ms-visio.drawing", "vsdx"),
    ("application/vnd.ms-visio.drawing.main+xml", "vsdx"),
    ("application/rtf", "rtf"),
    ("text/rtf", "rtf"),
    ("message/rfc822", "eml"),
    ("text/markdown", "md"),
    ("text/x-markdown", "md"),
    ("text/csv", "csv"),
    ("text/html", "html"),
    ("application/json", "json"),
    ("text/plain", "txt"),
];

/// Resolve a Drive MIME type (and filename as fallback) to an `extract_bytes`
/// kind. `None` = not ingestible (folders, binaries, unknown types).
pub fn kind_for(mime: &str, name: &str) -> Option<String> {
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Diagram names beat the MIME table: Drive reports `.excalidraw.json`
    // as application/json and `.drawio.svg` as image/svg+xml.
    if let Some(kind) = diagram_kind(name) {
        return Some(kind.to_string());
    }
    if let Some((_, kind)) = MIME_KINDS.iter().find(|(known, _)| *known == m.as_str()) {
        return Some((*kind).to_string());
    }
    if m.starts_with("text/") {
        return Some("txt".to_string());
    }
    // Drive often reports octet-stream - fall back to the filename.
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !ext.is_empty() && ext != name.to_ascii_lowercase() && is_supported_ext(&ext) {
        return Some(ext);
    }
    let base = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    if TEXT_NAMES.contains(&base.as_str()) {
        return Some(base);
    }
    None
}
// ----- text -----

/// Decode bytes as text: UTF-8 (BOM stripped), UTF-16 with BOM, else
/// Windows-1252 when the bytes look textual. NULs usually mean a binary file
/// with a misleading extension — reject instead of ingesting garbage.
fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(&bytes[3..])
            .map(|s| s.to_string())
            .map_err(|e| format!("not utf-8: {e}"));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&bytes[2..], false);
    }
    // UTF-16 without BOM (Windows "Unicode" saves): many NULs alternate bytes.
    if bytes.len() >= 4 && bytes.len() % 2 == 0 {
        let nuls = bytes.iter().filter(|b| **b == 0).count();
        if nuls * 4 >= bytes.len() {
            let le_nuls = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
            return decode_utf16(bytes, le_nuls * 2 >= nuls);
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(s.to_string()),
        Err(e) => {
            if bytes.contains(&0) {
                return Err(format!("binary content (not utf-8): {e}"));
            }
            Ok(decode_cp1252(bytes))
        }
    }
}

fn decode_utf16(bytes: &[u8], le: bool) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("truncated utf-16".to_string());
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16(&units).map_err(|e| format!("invalid utf-16: {e}"))
}

/// Windows-1252 byte to char (items 0x80–0x9F differ from Latin-1).
fn cp1252_byte(b: u8) -> char {
    match b {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => b as char,
    }
}

fn decode_cp1252(bytes: &[u8]) -> String {
    bytes.iter().map(|b| cp1252_byte(*b)).collect()
}

/// Strip tags/scripts/styles, keeping block structure (R36.7): `h1`–`h6`
/// become `#` markers, blocks break lines, table cells separate with tabs
/// (same shape as docx/odt tables, for the chunker's row-group splitter).
pub(crate) fn strip_html(s: &str) -> String {
    use std::sync::OnceLock;
    static RE_SCRIPT: OnceLock<regex::Regex> = OnceLock::new();
    static RE_STYLE: OnceLock<regex::Regex> = OnceLock::new();
    static RE_H: OnceLock<regex::Regex> = OnceLock::new();
    static RE_BLOCK: OnceLock<regex::Regex> = OnceLock::new();
    static RE_CELL_OPEN: OnceLock<regex::Regex> = OnceLock::new();
    static RE_CELL: OnceLock<regex::Regex> = OnceLock::new();
    static RE_TAG: OnceLock<regex::Regex> = OnceLock::new();
    let re_script =
        RE_SCRIPT.get_or_init(|| regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
    let re_style =
        RE_STYLE.get_or_init(|| regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
    let re_h = RE_H.get_or_init(|| regex::Regex::new(r"(?i)<h([1-6])\b[^>]*>").unwrap());
    let re_block = RE_BLOCK.get_or_init(|| {
        regex::Regex::new(
            r"(?i)</?(?:p|div|br|li|[ou]l|tr|h[1-6]|hr|section|article|header|footer|nav|main|aside|figure|blockquote|pre|table|thead|tbody)[^>]*>",
        )
        .unwrap()
    });
    let re_cell_open =
        RE_CELL_OPEN.get_or_init(|| regex::Regex::new(r"(?i)<(?:td|th)\b[^>]*>").unwrap());
    let re_cell = RE_CELL.get_or_init(|| regex::Regex::new(r"(?i)</(?:td|th)[^>]*>").unwrap());
    let re_tag = RE_TAG.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap());
    let mut t = re_script.replace_all(s, " ").to_string();
    t = re_style.replace_all(&t, " ").to_string();
    t = re_h
        .replace_all(&t, |caps: &regex::Captures| {
            let level: usize = caps[1].parse().unwrap_or(1);
            format!("\n{} ", "#".repeat(level.min(6)))
        })
        .to_string();
    t = re_block.replace_all(&t, "\n").to_string();
    t = re_cell_open.replace_all(&t, "").to_string();
    t = re_cell.replace_all(&t, "\t").to_string();
    t = re_tag.replace_all(&t, " ").to_string();
    normalize_lines(&unescape_html(&t))
}

/// R46.7: `<img>` survives tag stripping as markdown image syntax, so the
/// asset pass finds it on its hosting chunk. Inside a `<figure>`, the
/// `<figcaption>` text rides in the caption slot (the figcaption itself
/// stays in the prose). `data:` images keep only their alt text — inlined
/// base64 must never land in chunk text — and src spaces/parens are
/// percent-encoded so the markdown link stays one token.
fn html_images_to_md(html: &str) -> String {
    use std::sync::OnceLock;
    static RE_FIGURE: OnceLock<regex::Regex> = OnceLock::new();
    static RE_FIGCAPTION: OnceLock<regex::Regex> = OnceLock::new();
    let re_figure = RE_FIGURE.get_or_init(|| regex::Regex::new(r"(?is)<figure\b[^>]*>.*?</figure>").unwrap());
    let re_figcaption =
        RE_FIGCAPTION.get_or_init(|| regex::Regex::new(r"(?is)<figcaption\b[^>]*>(.*?)</figcaption>").unwrap());
    let with_figures = re_figure.replace_all(html, |caps: &regex::Captures| {
        let figure = &caps[0];
        let caption = re_figcaption
            .captures(figure)
            .map(|c| strip_html(&c[1]).split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        img_tags_to_md(figure, &caption)
    });
    img_tags_to_md(&with_figures, "")
}

fn img_tags_to_md(html: &str, default_caption: &str) -> String {
    use std::sync::OnceLock;
    static RE_IMG: OnceLock<regex::Regex> = OnceLock::new();
    let re_img = RE_IMG.get_or_init(|| regex::Regex::new(r"(?is)<img\b[^>]*>").unwrap());
    re_img
        .replace_all(html, |caps: &regex::Captures| {
            let tag = &caps[0];
            let clean = |s: String| unescape_html(&s).replace(['[', ']', '"', '\n', '\r'], " ").trim().to_string();
            let alt = clean(crate::assets::tag_attr(tag, "alt").unwrap_or_default());
            let Some(src) = crate::assets::tag_attr(tag, "src").map(|s| unescape_html(&s)) else {
                return format!(" {alt} ");
            };
            if src.trim().starts_with("data:") {
                return format!(" {alt} ");
            }
            let src = src.trim().replace(' ', "%20").replace('(', "%28").replace(')', "%29");
            let title = clean(crate::assets::tag_attr(tag, "title").unwrap_or_else(|| default_caption.to_string()));
            if title.is_empty() {
                format!(" ![{alt}]({src}) ")
            } else {
                format!(" ![{alt}]({src} \"{title}\") ")
            }
        })
        .into_owned()
}

/// Named + numeric HTML entity references (`&amp;`, `&#169;`, `&#x20;`,
/// `&nbsp;`); unknown entities pass through untouched, never dropped.
fn unescape_html(s: &str) -> String {
    use std::sync::OnceLock;
    static RE_ENT: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE_ENT.get_or_init(|| regex::Regex::new(r"&(#[xX]?[0-9a-fA-F]+|[a-zA-Z]+);").unwrap());
    re.replace_all(s, |caps: &regex::Captures| {
        let name = &caps[1];
        if name.eq_ignore_ascii_case("nbsp") {
            return "\u{a0}".to_string();
        }
        resolve_ref(name)
            .map(|c| c.to_string())
            .unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
}

// ----- pdf -----

fn extract_pdf(bytes: &[u8]) -> Result<String, String> {
    extract_pdf_labeled(bytes, "pdf").map(|(text, _)| text)
}

/// Catcher report: the standard PDF text tripped the split-word detector.
/// `alt_splits` is `None` when the repair pass itself failed; `tokens` is
/// the standard text's token count.
pub struct PdfQualityNote {
    pub file: String,
    pub std_splits: usize,
    pub alt_splits: Option<usize>,
    pub tokens: usize,
    pub used_alternate: bool,
}

impl PdfQualityNote {
    /// Event-detail shape; `items.rs::repair_matches` parses the `{file}:`
    /// label half back out for the R36.10 item flag — keep the separator.
    pub fn summary(&self) -> String {
        let alt = self.alt_splits.map(|n| n.to_string()).unwrap_or_else(|| "failed".to_string());
        let decision = if self.used_alternate { "repaired" } else { "kept-standard" };
        format!(
            "{}: split-word suspects {}/{} tokens, repair pass {} ({decision})",
            self.file, self.std_splits, self.tokens, alt
        )
    }
}

/// Adaptive PDF extraction (split-word catcher): `pdf-extract` inserts a
/// space at any glyph gap over 0.1em, and kerned/tracked PDFs trip that
/// mid-word ("Cancellati ons"). Good PDFs are untouched — the standard
/// output is kept unless the detector trips, in which case a flow-layout
/// re-extraction runs and the better-scoring text wins (ties keep standard).
fn extract_pdf_labeled(bytes: &[u8], label: &str) -> Result<(String, Option<PdfQualityNote>), String> {
    extract_pdf_text(bytes, label).map(|(text, note)| (with_pdf_images(text, bytes), note))
}

/// R46.16: the images each page draws, as `![](embedded:pdf:<obj>)` lines
/// closing that page block (image-only pages gain a block of their own).
/// Added after the split-word catcher so markers never score as text. No
/// image objects (vector-only charts) = text unchanged.
fn with_pdf_images(text: String, bytes: &[u8]) -> String {
    let Ok(doc) = lopdf::Document::load_mem(bytes) else { return text };
    let images = crate::pdf_images::page_images(&doc);
    if images.is_empty() {
        return text;
    }
    let marks: std::collections::BTreeMap<u32, Vec<String>> = images
        .into_iter()
        .map(|(page, ids)| (page, ids.into_iter().map(|id| format!("![]({}{})", crate::assets::EMBEDDED_SCHEME, crate::pdf_images::part_for(id))).collect()))
        .collect();
    merge_page_lines(&text, &marks)
}

/// Rebuild marker-delimited page blocks with extra lines appended per page,
/// in page order. Text before the first marker stays in front.
fn merge_page_lines(text: &str, extra: &std::collections::BTreeMap<u32, Vec<String>>) -> String {
    let mut front: Vec<&str> = Vec::new();
    let mut blocks: std::collections::BTreeMap<u32, Vec<String>> = std::collections::BTreeMap::new();
    let mut current: Option<u32> = None;
    for line in text.lines() {
        if let Some(n) = page_marker_number(line) {
            current = Some(n);
            blocks.entry(n).or_default();
            continue;
        }
        match current {
            Some(n) => blocks.entry(n).or_default().push(line.to_string()),
            None => front.push(line),
        }
    }
    for (page, lines) in extra {
        blocks.entry(*page).or_default().extend(lines.iter().cloned());
    }
    let mut out: Vec<String> = front.iter().map(|l| l.to_string()).collect();
    for (page, lines) in blocks {
        out.push(format!("[page {page}]"));
        out.extend(lines);
    }
    out.join("\n")
}

fn extract_pdf_text(bytes: &[u8], label: &str) -> Result<(String, Option<PdfQualityNote>), String> {
    // R36.10: per-page extraction so page markers ride both candidates —
    // the pipeline turns them into breadcrumb page numbers. Markers are
    // detector-neutral (`page` is a known word, digits never count).
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e| format!("pdf extract: {e}"))?;
    let std = join_pdf_pages(&pages);
    let (std_splits, tokens) = split_suspects(&std);
    if !pdf_trips_detector(std_splits, tokens) {
        return Ok((std, None));
    }
    let mut note = PdfQualityNote {
        file: label.to_string(),
        std_splits,
        alt_splits: None,
        tokens,
        used_alternate: false,
    };
    match extract_pdf_flow(bytes) {
        Ok(alt) => {
            let (alt_splits, _) = split_suspects(&alt);
            note.alt_splits = Some(alt_splits);
            note.used_alternate = alt_splits < std_splits;
            log_pdf_note(&note);
            if note.used_alternate {
                Ok((alt, Some(note)))
            } else {
                Ok((std, Some(note)))
            }
        }
        Err(e) => {
            tracing::warn!("pdf split-word catcher ({label}): repair pass failed: {e}");
            Ok((std, Some(note)))
        }
    }
}

/// R36.10 page markers: every non-blank PDF page opens with its own
/// `[page N]` line (1-based file numbering, gaps kept — a skipped blank
/// page still counts). The shape survives B12 cleaning (brackets dodge the
/// page-number-line strip, each marker is unique so the repeat strip keeps
/// it) and never classifies as a heading; the pipeline strips markers from
/// stored chunk content and rolls them into breadcrumb page numbers.
fn join_pdf_pages(pages: &[String]) -> String {
    let mut out = String::new();
    for (idx, page) in pages.iter().enumerate() {
        if page.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[page {}]\n{}", idx + 1, page.trim_end()));
    }
    out
}

/// Parse a page-marker line (`[page 12]`, exact after trimming) to its
/// 1-based page number. Anything else — printed `Page 3` footers included —
/// reads as body text, never a marker.
pub fn page_marker_number(line: &str) -> Option<u32> {
    let inner = line.trim().strip_prefix("[page ")?.strip_suffix(']')?;
    if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    inner.parse::<u32>().ok().filter(|n| *n >= 1)
}

/// True when extracted text carries page markers — the pipeline's gate for
/// breadcrumb page numbers (markers only come from PDF extraction).
pub fn has_page_markers(text: &str) -> bool {
    text.lines().any(|l| page_marker_number(l).is_some())
}

/// Drop page-marker lines, keeping everything else byte-identical (chunk
/// content stores marker-free; the stored item text keeps them as visible
/// page breaks for drill-down reads).
pub fn strip_page_markers(text: &str) -> String {
    text.lines().filter(|l| page_marker_number(l).is_none()).collect::<Vec<_>>().join("\n")
}

/// True for PDF filenames (extension check, like `is_config_filename`) —
/// the item flag's gate: only PDF items are spacing-scored.
pub fn is_pdf_filename(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    match base.rsplit('.').next() {
        Some(ext) if ext != base.as_str() => ext == "pdf",
        _ => false,
    }
}

/// Current-state spacing check for stored item text: the same trip wire the
/// catcher uses at ingest, reused at read time for the item flag.
pub fn pdf_spacing_suspect(text: &str) -> bool {
    let (splits, tokens) = split_suspects(text);
    pdf_trips_detector(splits, tokens)
}

/// Warn only when the repair strictly improved the text — ties are foreign
/// text or typos tripping the detector, not extractable damage.
fn log_pdf_note(note: &PdfQualityNote) {
    if note.used_alternate {
        tracing::warn!("pdf split-word catcher: {}", note.summary());
    } else {
        tracing::debug!("pdf split-word catcher (no improvement): {}", note.summary());
    }
}

/// Short tokens that are real words — never split-word fragments.
const COMMON_SHORT_WORDS: &[&str] = &[
    "a", "i", "am", "an", "as", "at", "be", "by", "do", "go", "he", "if", "in", "is", "it", "me", "my", "no",
    "of", "on", "or", "so", "to", "up", "us", "we", "vs", "ok", "ex", "re", "and", "the", "for", "are",
    "but", "not", "you", "all", "any", "can", "had", "her", "was", "one", "our", "out", "day", "get",
    "has", "him", "his", "how", "man", "new", "now", "old", "see", "two", "way", "who", "did", "its",
    "let", "put", "say", "she", "too", "use", "etc", "per", "via", "pro", "aka", "that", "with", "have",
    "this", "will", "your", "from", "they", "them", "then", "than",
];

/// English frequency set for the split-word catcher: top-30k wordfreq
/// 3.1.1 words (`^[a-z]{2,}$`), Apache-2.0 — see `pdf_words.NOTICE`.
/// Genuine ngram artifacts ("fi" from wi-fi, "ons" from add-ons, "ng")
/// sit inside the set, so fragments alone undercount: the join signal
/// below catches their pairs ("Cancellati"+"ons"), not the pieces.
fn pdf_words() -> &'static std::collections::HashSet<&'static str> {
    static WORDS: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    WORDS.get_or_init(|| include_str!("pdf_words.txt").split_whitespace().collect())
}

/// Domain abbreviations below the frequency cutoff — real tokens, never
/// split fragments. Two-to-four lowercase letters only (the fragment
/// shape); longer tokens never query this set.
const TECH_ALLOW_WORDS: &[&str] = &[
    "cli", "json", "yaml", "csv", "orm", "jwt", "cors", "csrf", "crud", "sdk", "xls", "jsx", "tsx",
    "scss", "ssl", "tls", "udp", "grpc", "tiff", "bmp", "webp", "avif", "mkv", "exe", "dll", "dmg",
    "deb", "rpm", "apk", "epub", "mobi", "faq", "wip", "tbd", "fyi", "asap", "eta", "eod", "pto",
    "ooo", "kpi", "okr", "sla", "sop", "rfp", "poc", "mvp", "arr", "mrr", "cac", "ltv", "yoy", "qoq",
    "ai", "llm", "rag", "nlp", "ocr", "vpn", "lan", "wan", "nat", "iam", "sso", "mfa", "kms",
    "mttr", "mtbf", "qbr", "kyc", "pii", "ttl", "aml", "gdpr", "ccpa", "pci", "dss", "iso", "nist",
    "oecd", "dpa", "sar", "ofac", "itil", "fips", "cc", "bcc", "oss", "bss",
];

fn is_known_word(tok: &str) -> bool {
    COMMON_SHORT_WORDS.contains(&tok) || TECH_ALLOW_WORDS.contains(&tok) || pdf_words().contains(tok)
}

/// Split-word suspects over lowercased tokens: a short unknown fragment
/// beside a lowercase neighbour ("noti fi", "cati ons"), or a pair whose
/// join is a known word while at least one side is not ("Cancellati"+"ons").
/// Two known words never trip ("sun day", "the rapist", "on line").
/// Returns (suspect_count, token_count).
pub fn split_suspects(text: &str) -> (usize, usize) {
    let toks: Vec<String> = text
        .split_whitespace()
        .map(|raw| raw.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let mut splits = 0usize;
    for (i, tok) in toks.iter().enumerate() {
        if is_split_fragment(tok, toks.get(i.wrapping_sub(1)), toks.get(i + 1)) {
            splits += 1;
        }
        if let Some(next) = toks.get(i + 1) {
            if is_split_join(tok, next) {
                splits += 1;
            }
        }
    }
    (splits, toks.len())
}

fn is_pure_alpha(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| c.is_ascii_lowercase())
}

/// Short unknown token (single letters are math variables, never splits)
/// touching a lowercase neighbour on either side.
fn is_split_fragment(tok: &str, prev: Option<&String>, next: Option<&String>) -> bool {
    let n = tok.chars().count();
    if !(2..=4).contains(&n) || !is_pure_alpha(tok) || is_known_word(tok) {
        return false;
    }
    prev.map(|p| p.chars().last().is_some_and(|c| c.is_ascii_lowercase())).unwrap_or(false)
        || next.map(|n| n.chars().next().is_some_and(|c| c.is_ascii_lowercase())).unwrap_or(false)
}

/// Adjacent pure-alpha pair whose join is a known word with at least one
/// unknown side — the rejoin test for ngram-artifact fragments.
fn is_split_join(a: &str, b: &str) -> bool {
    if a.chars().count() < 2 || b.chars().count() < 2 || !is_pure_alpha(a) || !is_pure_alpha(b) {
        return false;
    }
    let joined = format!("{a}{b}");
    if joined.chars().count() < 5 || !is_known_word(&joined) {
        return false;
    }
    !is_known_word(a) || !is_known_word(b)
}

/// Detector trip wire: an absolute floor (short clean texts stay silent)
/// plus a density ceiling (scattered false positives stay silent).
const PDF_TRIP_MIN_SPLITS: usize = 6;
const PDF_TRIP_PER_MILLE: usize = 6;

fn pdf_trips_detector(splits: usize, tokens: usize) -> bool {
    splits >= PDF_TRIP_MIN_SPLITS && splits.saturating_mul(1000) >= tokens.saturating_mul(PDF_TRIP_PER_MILLE)
}

/// Word gap in em for the repair pass: the standard 0.1em splits kerned
/// and tracked words; real word spaces run ~0.25em and up, kerning and
/// tracking jitter stays below ~0.15em.
const PDF_WORD_GAP_EM: f64 = 0.2;
/// Width below which a glyph's advance is untrustworthy: subsetted fonts
/// (Word export) report 0 for positioned glyphs (`i`, `p`, …), so the gap
/// after one measures the full advance (0.24–0.61em) — no threshold can
/// split it. Blind gaps join unless they exceed a whole em (even the
/// widest advance, `w` at 0.944, plus a space clears that).
const PDF_BLIND_WIDTH_EM: f64 = 0.05;
const PDF_BLIND_SPLIT_EM: f64 = 1.0;
/// Row move that starts a new line / new paragraph, in em.
const PDF_LINE_GAP_EM: f64 = 0.5;
const PDF_PARA_GAP_EM: f64 = 1.8;

/// Flow-layout PDF text sink over the same glyph stream as the standard
/// extractor: word breaks need a real word-sized gap (explicit spaces
/// always break), rows break lines, large row gaps break paragraphs.
/// Repair path only — runs when the split-word detector trips.
struct PdfFlowOutput {
    flip: pdf_extract::Transform,
    out: String,
    line: String,
    line_open: bool,
    pending_space: bool,
    last_end: f64,
    last_y: f64,
    last_width: f64,
    pages: usize,
    emit_markers: bool,
    pending_page: Option<u32>,
}

impl PdfFlowOutput {
    fn new() -> Self {
        Self {
            flip: pdf_extract::Transform::identity(),
            out: String::new(),
            line: String::new(),
            line_open: false,
            pending_space: false,
            last_end: 0.0,
            last_y: 0.0,
            last_width: 0.5,
            pages: 0,
            emit_markers: false,
            pending_page: None,
        }
    }

    /// Marker-emitting sink for the repair path: page markers ride the
    /// output so the winner carries page numbers either way. `new()` stays
    /// markerless (direct-sink unit tests pin its bytes).
    fn with_markers() -> Self {
        Self { emit_markers: true, ..Self::new() }
    }

    /// Lazily open the current page: the marker emits on the first real
    /// glyph, so glyph-less pages stay unmarked like the std path's blank
    /// skip (spaces alone never earn a marker — see `push_glyph` only).
    fn emit_pending_marker(&mut self) {
        if !self.emit_markers {
            return;
        }
        if let Some(n) = self.pending_page.take() {
            self.out.push_str(&format!("[page {n}]\n"));
        }
    }

    fn position(&self, trm: &pdf_extract::Transform) -> (f64, f64) {
        let p = trm.post_transform(&self.flip);
        (p.m31, p.m32)
    }

    fn text(mut self) -> String {
        self.flush_line();
        self.out
    }

    fn flush_line(&mut self) {
        if !self.line.trim().is_empty() {
            self.out.push_str(self.line.trim_end());
        }
        self.out.push('\n');
        self.line.clear();
        self.line_open = false;
        self.pending_space = false;
    }

    /// Row move since the last glyph: large gap = paragraph, small gap =
    /// line. Same-row text (columns, repositioned words) stays on the line.
    fn break_for_gap(&mut self, y: f64, size: f64) {
        if !self.line_open {
            return;
        }
        let dy = (y - self.last_y).abs();
        if dy > PDF_PARA_GAP_EM * size {
            self.flush_line();
            self.out.push('\n');
        } else if dy > PDF_LINE_GAP_EM * size {
            self.flush_line();
        }
    }

    fn push_glyph(&mut self, x: f64, y: f64, size: f64, width: f64, advance: f64, ch: &str) {
        self.emit_pending_marker();
        self.break_for_gap(y, size);
        let threshold = if self.last_width < PDF_BLIND_WIDTH_EM {
            PDF_BLIND_SPLIT_EM
        } else {
            PDF_WORD_GAP_EM
        };
        let gap_break = self.line_open && x > self.last_end + threshold * size;
        if self.pending_space || gap_break {
            if self.line_open && !self.line.ends_with(' ') {
                self.line.push(' ');
            }
            self.pending_space = false;
        }
        self.line.push_str(ch);
        self.line_open = true;
        self.last_end = x + advance;
        self.last_y = y;
        self.last_width = width;
    }

    fn push_space(&mut self, x: f64, y: f64, width: f64, advance: f64) {
        if self.line_open {
            self.pending_space = true;
        }
        self.last_end = x + advance;
        self.last_y = y;
        self.last_width = width;
    }
}

impl pdf_extract::OutputDev for PdfFlowOutput {
    fn begin_page(
        &mut self,
        page_num: u32,
        media_box: &pdf_extract::MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), pdf_extract::OutputError> {
        self.flush_line();
        if self.pages > 0 {
            self.out.push('\n');
        }
        self.pending_page = Some(page_num);
        self.flip = pdf_extract::Transform::row_major(1.0, 0.0, 0.0, -1.0, 0.0, media_box.ury - media_box.lly);
        self.pages += 1;
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.flush_line();
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &pdf_extract::Transform,
        width: f64,
        spacing: f64,
        font_size: f64,
        ch: &str,
    ) -> Result<(), pdf_extract::OutputError> {
        let (x, y) = self.position(trm);
        let size = pdf_glyph_size(trm, font_size);
        let advance = pdf_advance(width, spacing, font_size, size);
        if ch.is_empty() {
            self.last_end = x + advance;
            self.last_y = y;
            self.last_width = width;
        } else if ch.trim().is_empty() {
            self.push_space(x, y, width, advance);
        } else {
            self.push_glyph(x, y, size, width, advance, ch);
        }
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
}

/// Device-space glyph size: mirrors `pdf-extract`'s own estimate (linear
/// part of the text matrix applied to the font size), guarded for
/// degenerate matrices.
fn pdf_glyph_size(trm: &pdf_extract::Transform, font_size: f64) -> f64 {
    let x = (trm.m11 + trm.m21) * font_size;
    let y = (trm.m12 + trm.m22) * font_size;
    let size = (x * y).abs().sqrt();
    if size.is_finite() && size > 0.0 {
        size
    } else if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        12.0
    }
}

/// Expected device-space advance of one glyph: nominal width plus the
/// text-state spacing (`Tc` tracking, `Tw` justification) the standard
/// sink ignores — scaled into device space like the glyph size.
fn pdf_advance(width: f64, spacing: f64, font_size: f64, size: f64) -> f64 {
    let scale = if font_size > 0.0 { size / font_size } else { 1.0 };
    (width * font_size + spacing) * scale
}

fn extract_pdf_flow(bytes: &[u8]) -> Result<String, String> {
    let mut doc = pdf_extract::Document::load_mem(bytes).map_err(|e| format!("pdf extract: {e}"))?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|e| format!("pdf extract: {e}"))?;
    }
    let mut out = PdfFlowOutput::with_markers();
    pdf_extract::output_doc(&doc, &mut out).map_err(|e| format!("pdf extract: {e}"))?;
    Ok(normalize_lines(&dehyphenate_pdf(&out.text())))
}

/// Join typeset line-break hyphens ("notifica-\ntions" → "notifications"):
/// a line ending in `-`/soft-hyphen continues on a lowercase/digit line.
/// Hyphenated compounds ("well-\nknown") join too — typeset breaks
/// dominate in justified PDFs, and the joined form still embeds better
/// than the split pair.
fn dehyphenate_pdf(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut carry: Option<String> = None;
    for (idx, raw) in lines.iter().enumerate() {
        let cur = match carry.take() {
            Some(head) => format!("{head}{}", raw.trim()),
            None => raw.to_string(),
        };
        let next_low = lines.get(idx + 1).is_some_and(|n| starts_lower_alnum(n.trim_start()));
        if next_low {
            if let Some(head) = strip_break_hyphen(&cur) {
                carry = Some(head);
                continue;
            }
        }
        out.push(cur);
    }
    if let Some(head) = carry {
        out.push(format!("{head}-"));
    }
    out.join("\n")
}

/// Trailing break hyphen: `-`/soft-hyphen after an alphanumeric, never a
/// lone bullet or a double-hyphen dash.
fn strip_break_hyphen(line: &str) -> Option<String> {
    let t = line.trim_end();
    let head = t.strip_suffix('-').or_else(|| t.strip_suffix('\u{ad}'))?;
    if head.is_empty() || !head.chars().last().is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    if head.ends_with('-') || head.ends_with('\u{ad}') {
        return None;
    }
    Some(head.to_string())
}

fn starts_lower_alnum(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_digit() || (c.is_alphabetic() && c.is_lowercase()))
}

/// Resolve an XML general reference name (`amp`, `#169`, `#x20`) to a char.
pub(crate) fn resolve_ref(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            char::from_u32(code)
        }
    }
}

// ----- zip-backed formats (docx/pptx/odf) -----

pub(crate) fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))
}

pub(crate) fn read_zip_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
) -> Result<String, String> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| format!("missing {name}: {e}"))?;
    if entry.size() > MAX_ZIP_ENTRY {
        return Err(format!("{name} too large ({} bytes)", entry.size()));
    }
    let mut buf = Vec::new();
    entry
        .by_ref()
        .take(MAX_ZIP_ENTRY + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read {name}: {e}"))?;
    if buf.len() as u64 > MAX_ZIP_ENTRY {
        return Err(format!("{name} too large"));
    }
    String::from_utf8(buf).map_err(|e| format!("{name} is not utf-8: {e}"))
}

pub(crate) fn zip_names(archive: &zip::ZipArchive<Cursor<&[u8]>>, prefix: &str, suffix: &str) -> Vec<String> {
    archive
        .file_names()
        .filter_map(|n| n.ok())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .map(|n| n.to_string())
        .collect()
}

/// R46.8: package relationships of one part — `rId` → part path inside the
/// zip, resolved against `base` (`../media/x.png` from `ppt/slides` is
/// `ppt/media/x.png`). External (linked) targets are dropped: no network.
fn package_rels(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, rels: &str, base: &str) -> std::collections::HashMap<String, String> {
    use quick_xml::events::Event;
    let mut out = std::collections::HashMap::new();
    let Ok(xml) = read_zip_entry(archive, rels) else { return out };
    let mut reader = quick_xml::Reader::from_str(&xml);
    while let Ok(ev) = reader.read_event() {
        match ev {
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == "Relationship" => {
                let external = xml_attr(&e, "TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("external"));
                if let (Some(id), Some(target), false) = (xml_attr(&e, "Id"), xml_attr(&e, "Target"), external) {
                    out.insert(id, join_part(base, &target));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Resolve a relationship target against the folder of its part (`..` pops).
fn join_part(base: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn extract_docx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = open_zip(bytes)?;
    let xml = read_zip_entry(&mut archive, "word/document.xml")?;
    let rels = package_rels(&mut archive, "word/_rels/document.xml.rels", "word");
    let text = xml_text_media(&xml, Profile::Word, rels).map_err(|e| format!("docx: {e}"))?;
    Ok(strip_toc(&normalize_lines(&text)))
}

fn extract_pptx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = open_zip(bytes)?;
    let mut slides = zip_names(&archive, "ppt/slides/", ".xml");
    slides.sort_by_key(|n| slide_number(n).unwrap_or(u32::MAX));
    let mut out = String::new();
    for (i, name) in slides.iter().enumerate() {
        let xml = read_zip_entry(&mut archive, name)?;
        let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
        let rels = package_rels(&mut archive, &format!("{dir}/_rels/{file}.rels"), dir);
        let text = xml_text_media(&xml, Profile::Slides, rels).map_err(|e| format!("{name}: {e}"))?;
        let text = normalize_lines(&text);
        if text.trim().is_empty() {
            continue;
        }
        // R46.8: a marker heading, so each slide is a section (breadcrumbs,
        // and slide images attach to their slide).
        out.push_str(&format!("# Slide {}\n", i + 1));
        out.push_str(&text);
        out.push_str("\n\n");
    }
    if out.is_empty() {
        return Err("pptx: no slide text".to_string());
    }
    Ok(out.trim_end().to_string())
}

fn slide_number(name: &str) -> Option<u32> {
    let base = name.rsplit('/').next()?;
    let n = base.strip_prefix("slide")?.strip_suffix(".xml")?;
    n.parse().ok()
}

fn extract_odf(bytes: &[u8]) -> Result<String, String> {
    let mut archive = open_zip(bytes)?;
    let xml = read_zip_entry(&mut archive, "content.xml")?;
    let text = xml_text_media(&xml, Profile::Odf, std::collections::HashMap::new()).map_err(|e| format!("odf: {e}"))?;
    Ok(strip_toc(&normalize_lines(&text)))
}

#[derive(Clone, Copy)]
enum Profile {
    Word,
    Slides,
    Odf,
}

impl Profile {
    fn text_elem(&self, name: &str) -> bool {
        match self {
            Profile::Word => name == "w:t",
            Profile::Slides => name == "a:t",
            Profile::Odf => false,
        }
    }

    fn para_end(&self, name: &str) -> bool {
        match self {
            Profile::Word => name == "w:p" || name == "w:tr",
            Profile::Slides => name == "a:p",
            Profile::Odf => {
                name == "text:p"
                    || name == "text:h"
                    || name == "table:table-row"
                    || name == "draw:page"
            }
        }
    }

    fn cell_end(&self, name: &str) -> bool {
        match self {
            Profile::Word => name == "w:tc",
            Profile::Slides => false,
            Profile::Odf => name == "table:table-cell",
        }
    }

    fn line_break(&self, name: &str) -> bool {
        match self {
            Profile::Word => name == "w:br" || name == "w:cr",
            Profile::Slides => name == "a:br",
            Profile::Odf => name == "text:line-break",
        }
    }

    fn tab(&self, name: &str) -> bool {
        match self {
            Profile::Word => name == "w:tab",
            Profile::Slides => false,
            Profile::Odf => name == "text:tab",
        }
    }
}

/// What a tracked paragraph is: body text, a heading (`#` level), or a
/// field-generated TOC entry (stripped, not even a blank line is left).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParaKind {
    Body,
    Heading(u8),
    Toc,
    /// R46.8: Word `Caption` style — rides into the preceding image marker.
    Caption,
}

/// Walker state for `xml_text` — paragraph tracking drives R36.7 structure:
/// docx `w:pStyle` headings become `#` markers, TOC entries are stripped.
struct WalkState {
    out: String,
    in_text: bool,
    in_body: bool,
    para_start: Option<usize>,
    para_kind: ParaKind,
    toc_depth: u32,
    /// R46.8: `rId` → zip part for this XML part's images.
    rels: std::collections::HashMap<String, String>,
    /// Alt text of the drawing being read (`wp:docPr`/`p:cNvPr` descr).
    pending_alt: Option<String>,
    /// Byte offset of the last image marker's closing `)` (caption target).
    last_marker: Option<usize>,
}

impl WalkState {
    fn new(profile: Profile) -> Self {
        WalkState {
            out: String::new(),
            in_text: matches!(profile, Profile::Odf),
            in_body: !matches!(profile, Profile::Odf),
            para_start: None,
            para_kind: ParaKind::Body,
            toc_depth: 0,
            rels: std::collections::HashMap::new(),
            pending_alt: None,
            last_marker: None,
        }
    }

    /// R46.8: images become `![alt](embedded:<zip part>)` markers at their
    /// position, so the asset pass attaches each to its paragraph/slide
    /// chunk and reads the bytes from the package (`assets::resolve`).
    fn track_media(&mut self, e: &quick_xml::events::BytesStart, n: &str) {
        let attr = |k: &str| xml_attr(e, k).map(|v| unescape_html(&v));
        match n {
            "wp:docPr" | "p:cNvPr" => self.pending_alt = attr("descr").filter(|d| !d.trim().is_empty()).or_else(|| attr("title")),
            "a:blip" => {
                let part = attr("r:embed").and_then(|id| self.rels.get(&id).cloned());
                self.push_marker(part);
            }
            "v:imagedata" => {
                self.pending_alt = attr("o:title");
                let part = attr("r:id").and_then(|id| self.rels.get(&id).cloned());
                self.push_marker(part);
            }
            "draw:image" => {
                let href = attr("xlink:href").filter(|h| !h.contains("://"));
                self.push_marker(href.map(|h| h.trim_start_matches("./").to_string()));
            }
            _ => {}
        }
    }

    fn push_marker(&mut self, part: Option<String>) {
        let alt = self.pending_alt.take().unwrap_or_default();
        let Some(part) = part else { return };
        if !self.in_body || self.toc_depth > 0 {
            return;
        }
        let alt = alt.replace(|c: char| "[]\n\r".contains(c), " ");
        let part = part.replace(' ', "%20").replace('(', "%28").replace(')', "%29");
        self.out.push_str(&format!(" ![{}](embedded:{part}) ", alt.trim()));
        self.last_marker = Some(self.out.len() - 2);
    }

    fn wants_text(&self) -> bool {
        self.in_text && self.in_body && self.toc_depth == 0
    }

    fn on_start(&mut self, e: &quick_xml::events::BytesStart, profile: Profile) {
        let name = e.name();
        let n: &str = name.as_ref();
        if n == "office:body" {
            self.in_body = true;
        }
        if n == "text:table-of-content" {
            self.toc_depth += 1;
        }
        if self.toc_depth == 0 {
            self.track_para_start(e, n, profile);
            self.track_media(e, n);
        }
        if profile.text_elem(n) {
            self.in_text = true;
        }
    }

    fn track_para_start(&mut self, e: &quick_xml::events::BytesStart, n: &str, profile: Profile) {
        match profile {
            Profile::Word if n == "w:p" => {
                self.para_start = Some(self.out.len());
                self.para_kind = ParaKind::Body;
            }
            Profile::Word if n == "w:pStyle" => {
                if self.para_start.is_some() {
                    if let Some(val) = xml_attr(e, "w:val") {
                        self.para_kind = docx_para_kind(&val);
                    }
                }
            }
            Profile::Odf if n == "text:h" => {
                self.para_start = Some(self.out.len());
                self.para_kind = ParaKind::Heading(odf_heading_level(e));
            }
            _ => {}
        }
    }

    fn on_end(&mut self, e: &quick_xml::events::BytesEnd, profile: Profile) {
        let name = e.name();
        let n: &str = name.as_ref();
        if n == "text:table-of-content" {
            self.toc_depth = self.toc_depth.saturating_sub(1);
            return;
        }
        if self.toc_depth > 0 {
            return;
        }
        if profile.text_elem(n) {
            self.in_text = false;
        }
        if n == "office:body" {
            self.in_body = false;
        }
        if is_para_close(n, profile) {
            if self.flush_para() {
                self.out.push('\n');
            }
        } else if profile.cell_end(n) {
            while self.out.ends_with('\n') {
                self.out.pop();
            }
            self.out.push('\t');
        } else if profile.para_end(n) {
            self.out.push('\n');
        }
    }

    fn on_empty(&mut self, e: &quick_xml::events::BytesStart, profile: Profile) {
        let name = e.name();
        let n: &str = name.as_ref();
        if self.toc_depth > 0 {
            return;
        }
        self.track_media(e, n);
        if matches!(profile, Profile::Word) && n == "w:pStyle" && self.para_start.is_some() {
            if let Some(val) = xml_attr(e, "w:val") {
                self.para_kind = docx_para_kind(&val);
            }
        }
        if profile.tab(n) {
            self.out.push('\t');
        } else if profile.line_break(n) || profile.para_end(n) {
            self.out.push('\n');
        }
    }

    /// Close a tracked paragraph: strip TOC entries, prefix headings with
    /// `#` markers. Returns whether the paragraph break is still wanted.
    fn flush_para(&mut self) -> bool {
        let Some(start) = self.para_start.take() else {
            return true;
        };
        let kind = std::mem::replace(&mut self.para_kind, ParaKind::Body);
        match kind {
            ParaKind::Body => {
                // Text after the image paragraph means no caption follows.
                if self.last_marker.is_some_and(|m| m < start) && !self.out[start..].trim().is_empty() {
                    self.last_marker = None;
                }
                true
            }
            ParaKind::Caption => {
                self.attach_caption(start);
                true
            }
            ParaKind::Toc => {
                self.out.truncate(start);
                false
            }
            ParaKind::Heading(level) => {
                if !self.out[start..].trim().is_empty() {
                    self.out
                        .insert_str(start, &format!("{} ", "#".repeat(level as usize)));
                }
                true
            }
        }
    }
}

impl WalkState {
    /// R46.8: a `Caption` paragraph right after an image marker becomes the
    /// marker's caption slot; the caption text also stays in the prose.
    fn attach_caption(&mut self, start: usize) {
        let caption = self.out[start..].split_whitespace().collect::<Vec<_>>().join(" ").replace('\u{22}', "'");
        if let Some(pos) = self.last_marker.take().filter(|m| *m < start && !caption.is_empty()) {
            self.out.insert_str(pos, &format!(" \"{caption}\""));
        }
    }
}

/// Tracked paragraph closes: docx `w:p`, ODT `text:h`. (`w:tr` still ends
/// a row via the generic `para_end` branch — rows are never headings.)
fn is_para_close(n: &str, profile: Profile) -> bool {
    matches!(
        (profile, n),
        (Profile::Word, "w:p") | (Profile::Odf, "text:h")
    )
}

/// Raw attribute value (`w:val`, `text:outline-level`, ...) — style ids are
/// plain ASCII, so no entity decoding is needed.
fn xml_attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    for attr in e.attributes() {
        let attr = attr.ok()?;
        if attr.key.into_inner() == key {
            return Some(attr.value.into_owned());
        }
    }
    None
}

/// Map a docx paragraph style id to structure: `Heading1`–`Heading9`
/// (clamped to six `#` levels) and `Title` become headings, `TOC1`–`TOC9`
/// (field-generated TOC entries) strip. Anything else is a body paragraph —
/// unknown styles degrade to plain text, never to dropped text.
fn docx_para_kind(style: &str) -> ParaKind {
    if style.eq_ignore_ascii_case("title") {
        return ParaKind::Heading(1);
    }
    let lower: String = style.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = lower.to_ascii_lowercase();
    if let Some(n) = style_num(&lower, "heading") {
        return ParaKind::Heading(n.min(6));
    }
    if style_num(&lower, "toc").is_some() {
        return ParaKind::Toc;
    }
    if lower == "caption" {
        return ParaKind::Caption;
    }
    ParaKind::Body
}

/// Trailing `1`–`9` after a style prefix (`heading2`, `toc1`) — anything
/// else (letters, `0`, multi-digit past 9) is not a numbered style.
fn style_num(style: &str, prefix: &str) -> Option<u8> {
    style
        .strip_prefix(prefix)?
        .parse::<u8>()
        .ok()
        .filter(|n| (1..=9).contains(n))
}

/// ODT heading level from `text:outline-level` — a `text:h` without one is
/// still a heading (level 1), never body text.
fn odf_heading_level(e: &quick_xml::events::BytesStart) -> u8 {
    xml_attr(e, "text:outline-level")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|n| (1..=9).contains(n))
        .unwrap_or(1)
        .min(6)
}

/// Walk XML and pull text out of one of the three document shapes.
fn xml_text_media(xml: &str, profile: Profile, rels: std::collections::HashMap<String, String>) -> Result<String, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut st = WalkState::new(profile);
    st.rels = rels;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => st.on_start(&e, profile),
            Ok(Event::End(e)) => st.on_end(&e, profile),
            Ok(Event::Text(t)) => {
                if st.wants_text() {
                    let raw = t.into_inner();
                    let s = quick_xml::escape::unescape(&raw)
                        .map_err(|e| format!("xml entity: {e}"))?;
                    st.out.push_str(&s);
                }
            }
            Ok(Event::CData(t)) => {
                if st.wants_text() {
                    st.out.push_str(&t.into_inner());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if st.wants_text() {
                    if let Some(c) = resolve_ref(&r.into_inner()) {
                        st.out.push(c);
                    }
                }
            }
            Ok(Event::Empty(e)) => st.on_empty(&e, profile),
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("xml: {e}")),
            _ => {}
        }
    }
    Ok(st.out)
}

// ----- spreadsheets (xlsx/xlsm/xlsb/xls/ods via calamine) -----

fn extract_sheet(bytes: &[u8]) -> Result<String, String> {
    use calamine::Reader;
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| format!("spreadsheet: {e}"))?;
    let names = workbook.sheet_names();
    let mut out = String::new();
    for name in names {
        let range = match workbook.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut sheet = String::new();
        let mut rows_written = 0usize;
        let mut truncated = false;
        'rows: for row in range.rows() {
            if rows_written >= MAX_SHEET_ROWS {
                truncated = true;
                break 'rows;
            }
            let mut cells: Vec<String> = Vec::new();
            for cell in row.iter().take(MAX_SHEET_COLS) {
                cells.push(cell_text(cell));
            }
            while cells.last().map(|c| c.is_empty()).unwrap_or(false) {
                cells.pop();
            }
            if cells.is_empty() {
                continue;
            }
            sheet.push_str(&cells.join(" | "));
            sheet.push('\n');
            rows_written += 1;
        }
        if sheet.is_empty() {
            continue;
        }
        out.push_str(&format!("## Sheet: {name}\n"));
        out.push_str(&sheet);
        if truncated {
            out.push_str(&format!("(first {MAX_SHEET_ROWS} rows shown)\n"));
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        return Err("spreadsheet has no text".to_string());
    }
    Ok(out.trim_end().to_string())
}

fn cell_text(cell: &calamine::Data) -> String {
    use calamine::Data;
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => trim_float(*f),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(d) => d.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

fn trim_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        let s = format!("{f}");
        s
    }
}

// ----- rtf -----

fn extract_rtf(bytes: &[u8]) -> Result<String, String> {
    let s = decode_text(bytes)?;
    let text = rtf_to_text(&s);
    if text.trim().is_empty() {
        return Err("rtf: no text".to_string());
    }
    Ok(text)
}

/// RTF destinations whose group contents are metadata/pictures, not text.
const RTF_SKIP_DESTS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "generator",
    "listtable",
    "rsidtbl",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "xmlnstbl",
    "private",
    "blipuid",
];

/// Minimal RTF reader: control words, `\uN` escapes, `\'hh` (cp1252) bytes,
/// paragraph breaks, and skipped destination groups (`{\*\...}`, font/color
/// tables, styles, embedded pictures).
fn rtf_to_text(rtf: &str) -> String {
    let b = rtf.as_bytes();
    let mut out = String::new();
    let mut stack: Vec<bool> = Vec::new(); // per group: skipped?
    let mut skip = false;
    let mut uc = 1usize; // `\uc<N>` fallback char count
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => {
                stack.push(skip);
                let mut dest = false;
                let next = i + 1;
                let mut after = next;
                if next < b.len() && b[next] == b'\\' && next + 1 < b.len() && b[next + 1] == b'*' {
                    dest = true;
                    after = skip_control_word(b, next + 2);
                } else if next < b.len() && b[next] == b'\\' {
                    let (word, _param, word_end) = read_control(b, next + 1);
                    if RTF_SKIP_DESTS.contains(&word.as_str()) {
                        dest = true;
                        after = word_end;
                    }
                }
                if dest {
                    skip = true;
                    i = after;
                } else {
                    i = next;
                }
            }
            b'}' => {
                skip = stack.pop().unwrap_or(false);
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    break;
                }
                match b[i] {
                    b'\'' | b'\\' | b'{' | b'}' => {
                        if b[i] == b'\'' {
                            if i + 2 < b.len() {
                                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                                if let Ok(v) = u8::from_str_radix(hex, 16) {
                                    if !skip {
                                        out.push(cp1252_byte(v));
                                    }
                                }
                                i += 3;
                            } else {
                                i = b.len();
                            }
                        } else {
                            if !skip {
                                out.push(b[i] as char);
                            }
                            i += 1;
                        }
                    }
                    b'~' => {
                        if !skip {
                            out.push('\u{a0}');
                        }
                        i += 1;
                    }
                    b'-' | b'_' => i += 1,
                    b'*' => {
                        skip = true;
                        i += 1;
                        i = skip_control_word(b, i);
                    }
                    b'u' => {
                        i += 1;
                        let (_, param, next) = read_control(b, i);
                        i = next;
                        let n = param.unwrap_or(-1);
                        // RTF stores `\uN` as a signed 16-bit value.
                        let code = if n < 0 { n + 65536 } else { n };
                        if let Some(c) = char::from_u32(code as u32) {
                            if !skip {
                                out.push(c);
                            }
                        }
                        // skip the `\uc<N>` fallback characters
                        let mut remaining = uc;
                        while remaining > 0 && i < b.len() {
                            if b[i] == b'\\'
                                && i + 1 < b.len()
                                && (b[i + 1] == b'\''
                                    || b[i + 1] == b'\\'
                                    || b[i + 1] == b'{'
                                    || b[i + 1] == b'}')
                            {
                                i += if b[i + 1] == b'\'' { 4 } else { 2 };
                            } else if b[i] == b'{' || b[i] == b'}' {
                                break;
                            } else {
                                i += 1;
                            }
                            remaining -= 1;
                        }
                    }
                    _ => {
                        let (word, param, next) = read_control(b, i);
                        i = next;
                        if word == "uc" {
                            if let Some(n) = param {
                                if n >= 0 {
                                    uc = n as usize;
                                }
                            }
                        } else if !skip {
                            match word.as_str() {
                                "par" | "line" | "sect" | "page" => out.push('\n'),
                                "tab" => out.push('\t'),
                                _ => {}
                            }
                        }
                    }
                }
            }
            c => {
                if !skip {
                    out.push(cp1252_byte(c));
                }
                i += 1;
            }
        }
    }
    normalize_lines(&out)
}

/// Read a control word at `i` (letters) plus an optional numeric parameter.
fn read_control(b: &[u8], mut i: usize) -> (String, Option<i32>, usize) {
    let start = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let word = String::from_utf8_lossy(&b[start..i]).to_string();
    let mut param = None;
    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
        let num_start = i;
        while i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
            i += 1;
        }
        param = std::str::from_utf8(&b[num_start..i])
            .ok()
            .and_then(|t| t.parse::<i32>().ok());
    }
    if i < b.len() && b[i] == b' ' {
        i += 1;
    }
    (word, param, i)
}

fn skip_control_word(b: &[u8], i: usize) -> usize {
    let j = if i < b.len() && b[i] == b'\\' {
        i + 1
    } else {
        i
    };
    read_control(b, j).2
}

// ----- email -----

fn extract_eml(bytes: &[u8]) -> Result<String, String> {
    use mailparse::MailHeaderMap;
    let mail = mailparse::parse_mail(bytes).map_err(|e| format!("eml: {e}"))?;
    let mut out = String::new();
    for key in ["From", "To", "Cc", "Date", "Subject"] {
        if let Some(v) = mail.headers.get_first_value(key) {
            out.push_str(&format!("{key}: {v}\n"));
        }
    }
    if let Some(body) = part_text(&mail, 0) {
        out.push('\n');
        out.push_str(&body);
    }
    if out.trim().is_empty() {
        return Err("eml: no text content".to_string());
    }
    Ok(normalize_lines(out.trim_end()))
}

/// Prefer text/plain, then text/html (tag-stripped), then any text part.
fn part_text(mail: &mailparse::ParsedMail<'_>, depth: usize) -> Option<String> {
    if depth > 20 {
        return None;
    }
    if mail.subparts.is_empty() {
        if !mail.ctype.mimetype.starts_with("text/") {
            return None;
        }
        let body = mail.get_body().ok()?;
        return Some(if mail.ctype.mimetype == "text/html" {
            strip_html(&body)
        } else {
            body
        });
    }
    for kind in ["text/plain", "text/html"] {
        for part in &mail.subparts {
            if part.ctype.mimetype == kind {
                if let Some(t) = part_text(part, depth + 1) {
                    return Some(t);
                }
            }
        }
    }
    for part in &mail.subparts {
        if let Some(t) = part_text(part, depth + 1) {
            return Some(t);
        }
    }
    None
}

// ----- table-of-contents stripping (R36.7) -----

/// Strip unstyled/manual tables of contents from parser-produced text: a
/// non-heading line drops when it exactly repeats a `#`-marker heading title
/// (running header, title-page repeat) or pairs one with a TOC page-number
/// artifact (tab number, dot leaders). Field-marked TOCs (docx `TOC1`–`9`
/// styles, ODT `text:table-of-content`) never reach this pass — the XML
/// walker strips them. Plain-text/markdown passthrough never calls it.
fn strip_toc(text: &str) -> String {
    use std::collections::HashSet;
    let titles: HashSet<&str> = text
        .lines()
        .filter_map(|l| crate::chunking::marker_heading(l).map(|(_, t)| t))
        .collect();
    if titles.is_empty() {
        return text.to_string();
    }
    text.lines()
        .filter(|l| !is_toc_line(l, &titles))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_toc_line(line: &str, titles: &std::collections::HashSet<&str>) -> bool {
    if crate::chunking::marker_heading(line).is_some() {
        return false;
    }
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if titles.contains(t) {
        return true;
    }
    toc_entry_core(t)
        .map(|core| titles.contains(core))
        .unwrap_or(false)
}

/// Core text of a TOC page-number entry: `Title\t12` or `Title .... 12`.
/// A number joined by a plain space is NOT an entry (cross-reference).
fn toc_entry_core(line: &str) -> Option<&str> {
    if let Some((core, num)) = line.rsplit_once('\t') {
        if is_page_number(num.trim()) && !core.trim().is_empty() {
            return Some(core.trim());
        }
    }
    let cut = line.find("..").or_else(|| line.find('\u{2026}'))?;
    let core = line[..cut].trim();
    if core.is_empty() {
        return None;
    }
    let tail: String = line[cut..]
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.' && *c != '\u{2026}')
        .collect();
    if is_page_number(&tail) {
        Some(core)
    } else {
        None
    }
}

fn is_page_number(s: &str) -> bool {
    !s.is_empty() && (s.chars().all(|c| c.is_ascii_digit()) || s.chars().all(is_roman_numeral))
}

fn is_roman_numeral(c: char) -> bool {
    // `str::contains`, not a `matches!` alternative list: lizard 1.24
    // mis-scopes the `|`-separated char literals and swallows the rest of
    // the file into this function's span (same tokenizer family as R36.7).
    "ivxlcdm".contains(c.to_ascii_lowercase())
}

// ----- shared text shaping -----

/// Trim each line, collapse runs of spaces, drop leading/trailing blank lines
/// and collapse repeated blank lines.
fn normalize_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_blank = true;
    for raw in s.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            if !last_blank {
                out.push('\n');
                last_blank = true;
            }
            continue;
        }
        out.push_str(&collapse_spaces(line));
        out.push('\n');
        last_blank = false;
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

fn collapse_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_space = false;
    for c in line.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    // R36.10 marker tests sit ahead of `text_decodes_*`: lizard 1.24
    // mis-scopes that test into a bogus EOF span (see AGENTS.md), so tests
    // after it inflate the artifact instead of measuring themselves.
    #[test]
    fn pdf_page_marker_parse_and_strip() {
        assert_eq!(page_marker_number("[page 1]"), Some(1));
        assert_eq!(page_marker_number("  [page 12]  "), Some(12));
        for not in ["Page 3", "3 of 10", "[page x]", "[page 0]", "[page]", "x [page 2]", "[page 2] x", "42", ""] {
            assert_eq!(page_marker_number(not), None, "{not:?} must not parse as a marker");
        }
        assert!(!is_pdf_filename("report.pdfx") && !is_pdf_filename("pdf") && !is_pdf_filename("notes.md"));
        assert!(is_pdf_filename("report.pdf") && is_pdf_filename("DIR/R.PDF") && is_pdf_filename("a\\b.pdf"));
        let marked = "[page 1]\nFirst words\n[page 2]\nSecond words";
        assert!(has_page_markers(marked));
        assert!(!has_page_markers("First words\nSecond words"));
        assert_eq!(strip_page_markers(marked), "First words\nSecond words");
    }

    #[test]
    fn pdf_markers_do_not_trip_detector() {
        // Marker tokens (`page` + digits) must stay calibration-neutral:
        // clean prose wrapped in markers scores what bare prose scores.
        let bare = "Cancellations and notifications for processing of subscriptions and installations.";
        let marked = "[page 1]\nCancellations and notifications for processing\n[page 2]\nof subscriptions and installations.";
        let (bare_splits, _) = split_suspects(bare);
        let (splits, tokens) = split_suspects(marked);
        assert_eq!(splits, bare_splits, "markers must add no suspects");
        assert!(!pdf_trips_detector(splits, tokens));
        assert!(!pdf_spacing_suspect(marked));
        assert!(pdf_spacing_suspect("Cancellati ons and Correcti ons with noti fi cati ons for processi ng of subscripti ons"));
    }

    #[test]
    fn pdf_std_path_numbers_pages_and_skips_blank() {
        let pdf = pdf_bytes(&[
            "BT /F1 12 Tf 72 720 Td (First page words.) Tj ET",
            "BT /F1 12 Tf 72 720 Td (Second page words.) Tj ET",
        ]);
        let (text, note) = extract_pdf_labeled(&pdf, "two.pdf").unwrap();
        assert!(note.is_none(), "clean text must not trip");
        let one = text.find("[page 1]").expect("page 1 marker");
        let two = text.find("[page 2]").expect("page 2 marker");
        assert!(one < two, "{text:?}");
        assert!(text.contains("First page words."), "{text:?}");
        assert!(text.contains("Second page words."), "{text:?}");
        // A blank middle page leaves a numbering gap, never a marker.
        let pdf = pdf_bytes(&[
            "BT /F1 12 Tf 72 720 Td (Alpha words here.) Tj ET",
            "BT /F1 12 Tf ET",
            "BT /F1 12 Tf 72 720 Td (Gamma words here.) Tj ET",
        ]);
        let (text, _) = extract_pdf_labeled(&pdf, "gap.pdf").unwrap();
        assert!(text.contains("[page 1]"), "{text:?}");
        assert!(!text.contains("[page 2]"), "{text:?}");
        assert!(text.contains("[page 3]"), "{text:?}");
    }

    #[test]
    fn pdf_adaptive_clean_gains_only_page_markers() {
        // R36.10: clean PDFs keep their words — the only addition is the
        // page-marker line the pipeline rolls into breadcrumb page numbers.
        let pdf = pdf_bytes(&["BT /F1 12 Tf 72 720 Td (Cancellations and notifications.) Tj 0 -14 Td (Second line here.) Tj ET"]);
        let std = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        let (text, note) = extract_pdf_labeled(&pdf, "clean.pdf").unwrap();
        assert!(note.is_none());
        assert_eq!(strip_page_markers(&text), std.trim_end());
        assert!(text.starts_with("[page 1]\n"), "{text:?}");
    }

    #[test]
    fn pdf_flow_opens_first_page_with_marker() {
        let pdf = pdf_bytes(&["BT /F1 12 Tf 72 720 Td (First page words.) Tj ET"]);
        let text = extract_pdf_flow(&pdf).unwrap();
        assert!(text.starts_with("[page 1]\n"), "{text:?}");
        assert!(text.contains("First page words."), "{text:?}");
    }

    #[test]
    fn text_decodes_utf8_utf16_and_cp1252() {
        assert_eq!(decode_text(b"plain ascii").unwrap(), "plain ascii");
        assert_eq!(decode_text("héllo".as_bytes()).unwrap(), "héllo");
        // UTF-8 BOM
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("hi".as_bytes());
        assert_eq!(decode_text(&bom).unwrap(), "hi");
        // UTF-16 LE BOM
        let utf16: Vec<u8> = "héllo"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let mut utf16_bom = vec![0xFF, 0xFE];
        utf16_bom.extend_from_slice(&utf16);
        assert_eq!(decode_text(&utf16_bom).unwrap(), "héllo");
        // UTF-16 without BOM (LE) is detected by the NUL pattern
        assert_eq!(decode_text(&utf16).unwrap(), "héllo");
        // cp1252 fallback (0x93/0x94 = curly quotes)
        assert_eq!(decode_text(&[0x93, b'h', b'i', 0x94]).unwrap(), "“hi”");
        // binary with a text extension is rejected
        assert!(decode_text(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").is_err());
    }

    #[test]
    fn html_strips_tags_and_scripts() {
        let html = "<html><head><style>p{color:red}</style><script>alert(1)</script></head><body><h1>Plan</h1><p>Hello   world</p></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Plan"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("color"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn docx_reads_paragraphs_and_entities() {
        let doc = zip_bytes(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Quarterly plan</w:t></w:r></w:p>
<w:p><w:r><w:t>Owner: Sarah &amp; team</w:t></w:r><w:r><w:tab/><w:t>Due Q3</w:t></w:r></w:p>
</w:body></w:document>"#,
        )]);
        let text = extract_bytes("docx", &doc).unwrap();
        assert!(text.contains("Quarterly plan"));
        assert!(text.contains("Owner: Sarah & team"));
        assert!(text.contains('\t'));
        assert!(text.contains("Due Q3"));
    }

    #[test]
    fn pptx_reads_slides_in_numeric_order() {
        let ppt = zip_bytes(&[
            (
                "ppt/slides/slide10.xml",
                r#"<p:sld xmlns:a="x"><a:p><a:r><a:t>Ten</a:t></a:r></a:p></p:sld>"#,
            ),
            (
                "ppt/slides/slide2.xml",
                r#"<p:sld xmlns:a="x"><a:p><a:r><a:t>Two</a:t></a:r></a:p></p:sld>"#,
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:a="x"><a:p><a:r><a:t>One</a:t></a:r></a:p></p:sld>"#,
            ),
        ]);
        let text = extract_bytes("pptx", &ppt).unwrap();
        let two = text.find("# Slide 2\nTwo").expect("slide 2");
        let ten = text.find("# Slide 3\nTen").expect("slide 3");
        assert!(text.contains("# Slide 1\nOne"));
        assert!(two < ten, "slide2 before slide10: {text}");
    }

    #[test]
    fn odt_reads_paragraphs_and_tables() {
        let odt = zip_bytes(&[(
            "content.xml",
            r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:table="tb"><office:body><office:text>
<text:h>Heading</text:h><text:p>First <text:span>para</text:span>.</text:p>
<table:table><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table>
</office:text></office:body></office:document-content>"#,
        )]);
        let text = extract_bytes("odt", &odt).unwrap();
        assert!(text.contains("Heading"));
        assert!(text.contains("First para."));
        assert!(text.contains("A\tB"), "table cell separator: {text}");
    }

    #[test]
    fn xlsx_reads_sheets_with_shared_strings_and_numbers() {
        let xlsx = zip_bytes(&[
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#,
            ),
            (
                "xl/sharedStrings.xml",
                r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>Item</t></si><si><t>Cost</t></si></sst>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2"><v>42</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        let text = extract_bytes("xlsx", &xlsx).unwrap();
        assert!(text.contains("Sheet: Budget"), "{text}");
        assert!(text.contains("Item | Cost"));
        assert!(text.contains("Item | 42"));
    }

    #[test]
    fn rtf_strips_control_words_and_decodes_escapes() {
        let rtf = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil Calibri;}}{\*\generator Riched20;}{\colortbl;\red0\green0\blue0;}
Hello \b bold\b0  and \'93quoted\'94.\par Second \u8364? paragraph.\par}";
        let text = extract_bytes("rtf", &rtf.as_bytes()).unwrap();
        assert!(text.contains("Hello bold and “quoted”."), "{text}");
        assert!(text.contains("Second € paragraph."), "{text}");
        assert!(!text.contains("Calibri"));
        assert!(!text.contains("generator"));
        assert!(!text.contains('\u{0}'));
    }

    #[test]
    fn eml_reads_headers_and_prefers_plain_body() {
        let eml = concat!(
            "From: Alice <alice@example.com>\r\n",
            "To: Bob <bob@example.com>\r\n",
            "Subject: =?utf-8?q?Budget_=E2=82=AC?=\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"B\"\r\n\r\n",
            "--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
            "Budget approved. Owner: Bob.\r\n",
            "--B\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
            "<html><body><p>Budget approved. Owner: Bob.</p></body></html>\r\n",
            "--B--\r\n",
        );
        let text = extract_bytes("eml", eml.as_bytes()).unwrap();
        assert!(text.contains("Subject: Budget €"), "{text}");
        assert!(text.contains("Budget approved. Owner: Bob."));
        assert!(!text.contains("<p>"));
    }

    #[test]
    fn kind_for_maps_mime_and_falls_back_to_extension() {
        assert_eq!(kind_for("application/pdf", "x").as_deref(), Some("pdf"));
        assert_eq!(
            kind_for(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "plan"
            )
            .as_deref(),
            Some("docx")
        );
        assert_eq!(
            kind_for("application/octet-stream", "notes.docx").as_deref(),
            Some("docx")
        );
        assert_eq!(
            kind_for("text/plain; charset=utf-8", "x").as_deref(),
            Some("txt")
        );
        assert_eq!(kind_for("application/vnd.google-apps.folder", "x"), None);
        assert_eq!(kind_for("image/png", "x"), None);
        assert_eq!(kind_for("application/octet-stream", "Dockerfile").as_deref(), Some("dockerfile"));
        assert_eq!(kind_for("application/octet-stream", ".gitignore").as_deref(), Some(".gitignore"));
    }

    #[test]
    fn code_and_named_files_are_plain_text() {
        for ext in ["rs", "py", "ts", "tsx", "go", "java", "kt", "cs", "cpp", "h", "sh", "ps1", "sql", "css", "vue"] {
            assert!(is_supported_ext(ext), "{ext} should be supported");
            assert_eq!(extract_bytes(ext, b"fn main() { /* code */ }").unwrap(), "fn main() { /* code */ }");
        }
        assert!(is_supported_path(Path::new("src/main.rs")));
        assert!(is_supported_path(Path::new("Dockerfile")));
        assert!(is_supported_path(Path::new("Makefile")));
        assert!(is_supported_path(Path::new(".gitignore")));
        assert!(!is_supported_path(Path::new("logo.png")));
        assert_eq!(extract_bytes("dockerfile", b"FROM scratch\n").unwrap(), "FROM scratch\n");
        assert_eq!(extract_bytes(".gitignore", b"node_modules/\n").unwrap(), "node_modules/\n");
    }

    #[test]
    fn supported_matrix() {
        for ok in ["md", "DOCX", ".pdf", "xls", "eml", "rtf", "tsv", "jsonc", "yml", "tex"] {
            assert!(is_supported_ext(ok), "{ok} should be supported");
        }
        for no in ["doc", "ppt", "exe", "zip", ""] {
            assert!(!is_supported_ext(no), "{no} should not be supported");
        }
        assert!(is_supported_path(Path::new("a/b/report.PDF")));
        assert!(!is_supported_path(Path::new("a/b/archive.zip")));
    }

    #[test]
    fn code_filename_matrix() {
        for yes in ["main.rs", "src/lib.rs", "C:\\repo\\app\\index.ts", "/home/u/repo/a.py", "Dockerfile", "Makefile", "CMakeLists.txt", "deploy.sh", "schema.sql", "Jira:IGNORED.rs"] {
            assert!(is_code_filename(yes), "{yes} should be code");
        }
        for no in ["notes.md", "report.pdf", "data.json", "config.yaml", ".gitignore", ".dockerignore", "Jira:KEY-1", "gdrive:abc123", "logo.png", "README"] {
            assert!(!is_code_filename(no), "{no} should not be code");
        }
        assert!(is_code_path(Path::new("a/b/main.go")));
        assert!(!is_code_path(Path::new("a/b/notes.md")));
    }

    #[test]
    fn office_images_become_embedded_markers() {
        // R46.8: docx drawing → marker at its paragraph, alt from docPr,
        // a following Caption paragraph rides into the caption slot (and
        // stays prose); external (linked) images never resolve.
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="image" Target="media/image1.png"/><Relationship Id="rId6" Type="image" Target="https://x.test/a.png" TargetMode="External"/></Relationships>"#;
        let doc = r#"<w:document xmlns:w="w" xmlns:wp="wp" xmlns:a="a" xmlns:r="r"><w:body>
<w:p><w:r><w:t>Quarterly results.</w:t></w:r></w:p>
<w:p><w:r><w:drawing><wp:inline><wp:docPr id="1" name="Picture 1" descr="Revenue [chart] &amp; trend"/><a:graphic><a:blip r:embed="rId5"/></a:graphic></wp:inline></w:drawing></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Caption"/></w:pPr><w:r><w:t>Figure 1: Revenue "by" quarter</w:t></w:r></w:p>
<w:p><w:r><w:drawing><wp:docPr id="2" descr="linked"/><a:blip r:embed="rId6"/></w:drawing></w:r></w:p>
</w:body></w:document>"#;
        let docx = zip_bytes(&[("word/document.xml", doc), ("word/_rels/document.xml.rels", rels)]);
        let text = extract_bytes("docx", &docx).unwrap();
        assert!(text.contains("![Revenue chart & trend](embedded:word/media/image1.png \"Figure 1: Revenue 'by' quarter\")"), "{text}");
        assert!(text.contains("Figure 1: Revenue \"by\" quarter"), "caption stays prose: {text}");
        assert!(!text.contains("x.test") && !text.contains("linked]"), "{text}");
        let refs = crate::assets::scan_refs(&text);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].caption, "Figure 1: Revenue 'by' quarter");

        let slide_rels = r#"<Relationships><Relationship Id="rId2" Target="../media/image7.jpeg"/></Relationships>"#;
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Architecture</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="4" name="Picture 3" descr="Service map"/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic></p:spTree></p:cSld></p:sld>"#;
        let pptx = zip_bytes(&[("ppt/slides/slide1.xml", slide), ("ppt/slides/_rels/slide1.xml.rels", slide_rels)]);
        let text = extract_bytes("pptx", &pptx).unwrap();
        assert!(text.starts_with("# Slide 1\n"), "{text}");
        assert!(text.contains("![Service map](embedded:ppt/media/image7.jpeg)"), "{text}");

        let content = r#"<office:document-content xmlns:office="o" xmlns:text="t" xmlns:draw="d" xmlns:xlink="x"><office:body><office:text><text:p>Floor plan<draw:frame><draw:image xlink:href="Pictures/plan.png"/></draw:frame></text:p></office:text></office:body></office:document-content>"#;
        let text = extract_bytes("odt", &zip_bytes(&[("content.xml", content)])).unwrap();
        assert!(text.contains("![](embedded:Pictures/plan.png)"), "{text}");
        assert_eq!(join_part("ppt/slides", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(join_part("word", "/word/media/b.png"), "word/media/b.png");
    }

    #[test]
    fn html_images_become_markdown_refs() {
        let html = r#"<p>Intro</p><img src="img/a b.png" alt="Sign [in]"><figure><img src='chart.png'><figcaption>Q3 <b>revenue</b></figcaption></figure><img src="data:image/png;base64,AAAA" alt="inline"><img alt="no src">"#;
        let text = extract_bytes("html", html.as_bytes()).unwrap();
        assert!(text.contains("![Sign in](img/a%20b.png)"), "{text}");
        assert!(text.contains("![](chart.png \"Q3 revenue\")"), "{text}");
        assert!(text.contains("Q3 revenue"), "figcaption stays prose: {text}");
        assert!(!text.contains("base64"), "{text}");
        assert!(text.contains("inline") && text.contains("no src"), "{text}");
        let refs = crate::assets::scan_refs(&text);
        let srcs: Vec<&str> = refs.iter().map(|r| r.src.as_str()).collect();
        assert_eq!(srcs, vec!["img/a%20b.png", "chart.png"]);
        assert_eq!(refs[1].caption, "Q3 revenue");
    }

    #[test]
    fn diagram_kind_dispatch() {
        // R46.2: full-name match so double extensions beat the last one.
        for (name, kind) in [
            ("flows/order.drawio", Some("drawio")),
            ("Order.DIO", Some("drawio")),
            ("board.excalidraw", Some("excalidraw")),
            ("C:\\x\\board.excalidraw.json", Some("excalidraw")),
            ("arch.drawio.svg", Some("svg")),
            ("sketch.excalidraw.svg", Some("svg")),
            ("logo.svg", Some("svg")),
            ("flows/Process.VSDX", Some("vsdx")),
            ("macro.vsdm", Some("vsdx")),
            ("miro:uXjVOabc=", Some("miro")),
            ("data.json", None),
            ("notes.md", None),
        ] {
            assert_eq!(diagram_kind(name), kind, "{name}");
        }
        assert!(!is_config_filename("board.excalidraw.json"));
        assert!(is_config_filename("board.json"));
        assert!(crate::markdown::is_markdown_filename("flows/order.drawio"));
        assert!(crate::markdown::is_markdown_filename("miro:uXjVOabc="), "R46.5 boards chunk as markdown");
        assert!(is_supported_path(Path::new("a/process.vsdx")));
        assert_eq!(kind_for("application/vnd.ms-visio.drawing", "process").as_deref(), Some("vsdx"));
        assert!(is_supported_path(Path::new("a/order.drawio")));
        assert!(is_supported_path(Path::new("a/logo.svg")));
        assert_eq!(kind_for("application/json", "board.excalidraw.json").as_deref(), Some("excalidraw"));
        assert_eq!(kind_for("image/svg+xml", "arch.drawio.svg").as_deref(), Some("svg"));
        assert_eq!(kind_for("application/vnd.jgraph.mxfile", "order.drawio").as_deref(), Some("drawio"));
        let scene = br#"{"type":"excalidraw","elements":[{"id":"a","type":"text","text":"Kickoff","x":0,"y":0}]}"#;
        assert_eq!(extract_bytes("excalidraw", scene).unwrap(), "- Kickoff\n");
        assert!(extract_bytes("drawio", b"\x00\x01binary").is_err());
    }

    #[test]
    fn config_filename_matrix() {
        // R33.4: hierarchical configs are their own family — never code.
        for yes in ["config.yaml", "values.yml", "C:\\repo\\tsconfig.json", "a/b/settings.jsonc", "Cargo.toml"] {
            assert!(is_config_filename(yes), "{yes} should be config");
            assert!(!is_code_filename(yes), "{yes} should not be code");
        }
        for no in ["notes.md", "main.rs", "app.ini", "setup.cfg", "data.xml", "Makefile", "README"] {
            assert!(!is_config_filename(no), "{no} should not be config");
        }
    }

    #[test]
    fn docx_heading_styles_emit_markers() {
        // R36.7: Heading styles → `#` markers; Title → `#`; unknown/missing
        // styles degrade to plain paragraphs (kept, never dropped).
        let doc = zip_bytes(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Annual Report</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Overview</w:t></w:r></w:p>
<w:p><w:r><w:t>Body para one.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"></w:pStyle></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:t>Quoted text.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading7"/></w:pPr><w:r><w:t>Deep</w:t></w:r></w:p>
</w:body></w:document>"#,
        )]);
        let text = extract_bytes("docx", &doc).unwrap();
        assert_eq!(
            text,
            "# Annual Report\n# Overview\nBody para one.\n## Details\nQuoted text.\n###### Deep",
            "{text}"
        );
    }

    #[test]
    fn docx_toc_styles_stripped_without_trace() {
        let doc = zip_bytes(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Overview</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:t>Overview</w:t></w:r><w:r><w:tab/><w:t>3</w:t></w:r></w:p>
<w:p><w:r><w:t>After toc.</w:t></w:r></w:p>
</w:body></w:document>"#,
        )]);
        let text = extract_bytes("docx", &doc).unwrap();
        assert_eq!(text, "# Overview\nAfter toc.", "{text}");
    }

    #[test]
    fn docx_manual_toc_repetition_stripped_cross_ref_kept() {
        // Unstyled TOC entries (tab number, dot leaders) strip; a number
        // joined by a plain space is a cross-reference and stays.
        let doc = zip_bytes(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Overview</w:t></w:r><w:r><w:tab/><w:t>3</w:t></w:r></w:p>
<w:p><w:r><w:t>Details .... 5</w:t></w:r></w:p>
<w:p><w:r><w:t>See Chapter 2 for background</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Overview</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter</w:t></w:r></w:p>
<w:p><w:r><w:t>Chapter 2</w:t></w:r></w:p>
</w:body></w:document>"#,
        )]);
        let text = extract_bytes("docx", &doc).unwrap();
        assert_eq!(
            text, "See Chapter 2 for background\n# Overview\n## Details\n# Chapter\nChapter 2",
            "{text}"
        );
    }

    #[test]
    fn odt_headings_and_toc_subtree() {
        // `text:h` outline-level → markers (missing level → 1);
        // `text:table-of-content` subtree stripped.
        let odt = zip_bytes(&[(
            "content.xml",
            "<office:document-content xmlns:office=\"o\" xmlns:text=\"t\"><office:body><office:text>\n\
             <text:table-of-content><text:p>Overview\t3</text:p></text:table-of-content>\n\
             <text:h text:outline-level=\"1\">Overview</text:h>\n\
             <text:p>Body.</text:p>\n\
             <text:h text:outline-level=\"3\">Deep dive</text:h>\n\
             <text:h>No level</text:h>\n\
             </office:text></office:body></office:document-content>",
        )]);
        let text = extract_bytes("odt", &odt).unwrap();
        assert_eq!(
            text, "# Overview\n\nBody.\n\n### Deep dive\n\n# No level",
            "{text}"
        );
    }

    #[test]
    fn html_headings_tables_and_entities() {
        let html = "<html><head><title>Doc</title><style>p{x}</style><script>bad()</script></head>\
             <body><h1>Plan &amp; Budget</h1><p>First para.</p><h3>Deep</h3>\
             <table><tr><th>Name</th><th>Cost</th></tr><tr><td>Item</td><td>42</td></tr></table>\
             <p>Tail.</p></body></html>";
        let text = extract_bytes("html", html.as_bytes()).unwrap();
        assert_eq!(
            text,
            "Doc\n# Plan & Budget\n\nFirst para.\n\n### Deep\n\nName\tCost\n\nItem\t42\n\nTail.",
            "{text}"
        );
        assert!(!text.contains("bad()"));
        assert!(!text.contains("p{x}"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn docx_markers_section_end_to_end() {
        // docx bytes → `#` markers → heading sections with clean titles.
        let doc = zip_bytes(&[(
            "word/document.xml",
            r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Overview</w:t></w:r></w:p>
<w:p><w:r><w:t>Body one.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
<w:p><w:r><w:t>Body two.</w:t></w:r></w:p>
</w:body></w:document>"#,
        )]);
        let text = extract_bytes("docx", &doc).unwrap();
        let (secs, chunks) = crate::chunking::chunk_document_with_sections(&text);
        let titles: Vec<&str> = secs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Overview", "Details"]);
        assert_eq!(secs[1].parent, Some(0));
        assert_eq!(secs[1].path, "Overview > Details");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.starts_with("# Overview\n"));
    }

    #[test]
    fn corrupt_inputs_error_instead_of_panicking() {
        assert!(extract_bytes("docx", b"not a zip").is_err());
        assert!(extract_bytes("pptx", b"not a zip").is_err());
        assert!(extract_bytes("xlsx", b"not a zip").is_err());
        assert!(extract_bytes("pdf", b"not a pdf").is_err());
        assert!(extract_bytes("ods", b"nope").is_err());
        assert!(extract_bytes("weird", b"x").is_err());
    }

    /// Minimal single-font (Helvetica) PDF with one content stream per page —
    /// byte offsets are computed, so the xref table is always valid.
    fn pdf_bytes(page_streams: &[&str]) -> Vec<u8> {
        let n_pages = page_streams.len();
        let page_first = 3;
        let content_first = page_first + n_pages;
        let font_id = content_first + n_pages;
        let mut parts: Vec<(usize, Vec<u8>)> = Vec::new();
        let kids: Vec<String> = (0..n_pages).map(|i| format!("{} 0 R", page_first + i)).collect();
        parts.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
        parts.push((
            2,
            format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), n_pages).into_bytes(),
        ));
        for i in 0..n_pages {
            parts.push((
                page_first + i,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                     /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {} 0 R >>",
                    content_first + i,
                )
                .into_bytes(),
            ));
        }
        for (i, stream) in page_streams.iter().enumerate() {
            parts.push((
                content_first + i,
                format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()).into_bytes(),
            ));
        }
        parts.push((font_id, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
        parts.sort_by_key(|(n, _)| *n);
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize; font_id + 1];
        for (n, body) in &parts {
            offsets[*n] = out.len();
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", font_id + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", font_id + 1)
                .as_bytes(),
        );
        out
    }

    #[test]
    fn pdf_fixture_extracts_clean_line() {
        let pdf = pdf_bytes(&["BT /F1 12 Tf 72 720 Td (Cancellations and notifications.) Tj ET"]);
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(text.contains("Cancellations and notifications."), "{text:?}");
    }

    #[test]
    fn pdf_standard_splits_kerned_tj_words() {
        // Repro: TJ kerning offsets between string fragments read as word
        // gaps ("Cancellati ons") — the reported extraction damage.
        let pdf = pdf_bytes(&[
            "BT /F1 12 Tf 72 720 Td [(Cancellati) -150 (ons and noti) -150 (fi) -150 (cati) -150 (ons.)] TJ ET",
        ]);
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(text.contains("Cancellati ons"), "{text:?}");
        assert!(text.contains("noti fi cati ons"), "{text:?}");
    }

    #[test]
    fn split_scorer_flags_shredded_text() {
        let shredded = "Cancellati ons and Correcti ons with noti fi cati ons \
            for processi ng of subscripti ons and installati on guides";
        let (splits, tokens) = split_suspects(shredded);
        assert!(splits >= 6, "splits={splits} tokens={tokens}");
        assert!(pdf_trips_detector(splits, tokens));
    }

    #[test]
    fn split_scorer_silent_on_clean_prose() {
        for clean in [
            "Cancellations and notifications for processing of subscriptions.",
            "The db migration ran via the CLI with no errors, etc.",
            "Section 12.3 covers Article 4, pages 10 of 42 - see - 12 - below.",
            "State-of-the-art results: e.g. well-known DON'T stop # Heading One",
        ] {
            let (splits, tokens) = split_suspects(clean);
            assert!(!pdf_trips_detector(splits, tokens), "{clean:?} splits={splits} tokens={tokens}");
        }
    }

    fn md_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir).unwrap();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                md_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }

    /// Three shredded lines (~9 suspects in standard output): kerned TJ
    /// fragments across several join shapes (plurals, gerunds, middles).
    fn shredded_pdf() -> Vec<u8> {
        pdf_bytes(&[
            "BT /F1 12 Tf 72 720 Td [(Cancellati) -150 (ons and noti) -150 (fi) -150 (cati) -150 (ons.)] TJ 0 -14 Td [(Correcti) -150 (ons for processi) -150 (ng of subscripti) -150 (ons.)] TJ 0 -14 Td [(Installati) -150 (on guides and noti) -150 (fi) -150 (cati) -150 (ons.)] TJ ET",
        ])
    }

    #[test]
    fn pdf_adaptive_repairs_shredded_text() {
        let pdf = shredded_pdf();
        let std = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        let (std_splits, _) = split_suspects(&std);
        assert!(std_splits >= 6, "fixture must trip: {std:?}");
        let (text, note) = extract_pdf_labeled(&pdf, "shredded.pdf").unwrap();
        let note = note.expect("tripped detector reports a note");
        assert!(note.used_alternate, "{}", note.summary());
        assert!(note.alt_splits.unwrap() < std_splits, "{}", note.summary());
        assert!(text.contains("Cancellations"), "{text:?}");
        assert!(text.contains("notifications"), "{text:?}");
        assert!(text.contains("Corrections"), "{text:?}");
        assert!(text.contains("processing"), "{text:?}");
        assert!(text.contains("subscriptions"), "{text:?}");
        assert!(text.contains("Installation"), "{text:?}");
        assert!(!text.contains("Cancellati ons"), "{text:?}");
    }

    #[test]
    fn pdf_flow_keeps_tj_word_spaces() {
        // A real word-sized TJ gap (0.278em) still breaks in the repair pass.
        let pdf = pdf_bytes(&["BT /F1 12 Tf 72 720 Td [(Hello) -278 (world)] TJ ET"]);
        let text = extract_pdf_flow(&pdf).unwrap();
        assert!(text.contains("Hello world"), "{text:?}");
    }

    #[test]
    fn pdf_flow_joins_tracked_fragments() {
        // `Tc` tracking shifts every advance; the standard sink ignores the
        // spacing term and splits, the flow sink accounts for it.
        let pdf = pdf_bytes(&["BT /F1 12 Tf 2 Tc 72 720 Td [(Tra) (cked)] TJ ET"]);
        let std = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(std.contains("Tra cked"), "{std:?}");
        let text = extract_pdf_flow(&pdf).unwrap();
        assert!(text.contains("Tracked"), "{text:?}");
    }

    #[test]
    fn pdf_flow_breaks_lines_paragraphs_and_pages() {
        let pdf = pdf_bytes(&[
            "BT /F1 12 Tf 72 720 Td (First line) Tj 0 -14 Td (Second line) Tj 0 -40 Td (New paragraph) Tj ET",
            "BT /F1 12 Tf 72 720 Td (Page two) Tj ET",
        ]);
        let text = extract_pdf_flow(&pdf).unwrap();
        assert!(text.contains("First line\nSecond line"), "{text:?}");
        assert!(text.contains("Second line\n\nNew paragraph"), "{text:?}");
        // R36.10: the page break now carries its marker line.
        assert!(text.contains("New paragraph\n\n[page 2]\nPage two"), "{text:?}");
    }

    #[test]
    fn pdf_flow_dehyphenates_line_breaks() {
        let pdf = pdf_bytes(&["BT /F1 12 Tf 72 720 Td (notifica-) Tj 0 -14 Td (tions here) Tj ET"]);
        let text = extract_pdf_flow(&pdf).unwrap();
        assert!(text.contains("notifications here"), "{text:?}");
    }

    #[test]
    fn dehyphenate_joins_breaks_keeps_bullets() {
        assert_eq!(dehyphenate_pdf("notifica-\ntions"), "notifications");
        assert_eq!(dehyphenate_pdf("a-\nb-\nc"), "abc");
        assert_eq!(dehyphenate_pdf("well-\nKnown"), "well-\nKnown");
        assert_eq!(dehyphenate_pdf("-\nfoo"), "-\nfoo");
        assert_eq!(dehyphenate_pdf("word --\nnext"), "word --\nnext");
        assert_eq!(dehyphenate_pdf("tail-\n"), "tail-\n");
        assert_eq!(dehyphenate_pdf("no hyphen\nhere"), "no hyphen\nhere");
    }

    #[test]
    fn pdf_quality_note_summary_names_decision() {
        let repaired = PdfQualityNote {
            file: "a.pdf".to_string(),
            std_splits: 9,
            alt_splits: Some(1),
            tokens: 100,
            used_alternate: true,
        };
        assert!(repaired.summary().contains("repaired"), "{}", repaired.summary());
        let kept = PdfQualityNote {
            file: "b.pdf".to_string(),
            std_splits: 9,
            alt_splits: Some(9),
            tokens: 100,
            used_alternate: false,
        };
        assert!(kept.summary().contains("kept-standard"), "{}", kept.summary());
    }

    /// Drive the flow sink directly with synthetic glyphs
    /// (`x`, `width_em`, `font_size`, `char` on one row): zero-width font
    /// entries can't be produced through a Helvetica fixture.
    fn flow_glyph_text(calls: &[(f64, f64, f64, &str)]) -> String {
        use pdf_extract::OutputDev;
        let mut out = PdfFlowOutput::new();
        out.begin_page(
            1,
            &pdf_extract::MediaBox { llx: 0.0, lly: 0.0, urx: 612.0, ury: 792.0 },
            None,
        )
        .unwrap();
        for (x, w, fs, ch) in calls {
            let trm = pdf_extract::Transform::row_major(1.0, 0.0, 0.0, 1.0, *x, 400.0);
            out.output_character(&trm, *w, 0.0, *fs, ch).unwrap();
        }
        out.end_page().unwrap();
        out.text()
    }

    #[test]
    fn pdf_flow_joins_zero_width_gaps() {
        // Subsetted font reports width 0 for `p`/`i`: the measured gaps
        // (0.61em/0.24em) are phantom advances, not word spaces.
        let text = flow_glyph_text(&[
            (72.0, 0.73, 40.0, "C"),
            (101.2, 0.60, 40.0, "o"),
            (125.2, 0.376, 40.0, "r"),
            (140.9, 0.0, 40.0, "p"),
            (165.4, 0.60, 40.0, "o"),
        ]);
        assert_eq!(text.trim(), "Corpo");
        let text = flow_glyph_text(&[(337.7, 0.0, 40.0, "i"), (347.3, 0.60, 40.0, "o")]);
        assert_eq!(text.trim(), "io");
    }

    #[test]
    fn pdf_flow_splits_only_huge_blind_gaps() {
        // Past one em even a blind gap must hold a real space.
        let text = flow_glyph_text(&[(0.0, 0.0, 40.0, "i"), (45.0, 0.60, 40.0, "o")]);
        assert_eq!(text.trim(), "i o");
    }

    #[test]
    fn pdf_flow_normal_gap_threshold_unchanged() {
        // Sanity either side of 0.2em with honest widths.
        let text = flow_glyph_text(&[(0.0, 0.5, 40.0, "a"), (30.0, 0.5, 40.0, "b")]);
        assert_eq!(text.trim(), "a b");
        let text = flow_glyph_text(&[(0.0, 0.5, 40.0, "a"), (26.0, 0.5, 40.0, "b")]);
        assert_eq!(text.trim(), "ab");
    }

    #[test]
    fn pdf_detector_silent_on_repo_corpora() {
        // Calibrated 2026-10-13: 0 trips over 832 clean files (sample/,
        // sample-ai/ plus the local business-scale fixture set); the worst
        // clean file scores 3 suspects against a trip floor of 6.
        let mut files = Vec::new();
        for corpus in ["../../../sample", "../../../sample-ai"] {
            md_files(std::path::Path::new(corpus), &mut files);
        }
        assert!(!files.is_empty());
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            let (splits, tokens) = split_suspects(&text);
            assert!(
                !pdf_trips_detector(splits, tokens),
                "{} trips: splits={splits} tokens={tokens}",
                path.display()
            );
        }
    }
}
