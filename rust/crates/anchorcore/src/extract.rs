//! Document text extraction — one place for every format the connectors can
//! ingest (folder + Drive): notes, source code and config, HTML, PDF, Word,
//! PowerPoint, Excel (modern + legacy), OpenDocument, RTF and email.
//!
//! Pure functions over bytes: `extract_file` for the folder connector,
//! `extract_bytes` for the Drive connector (exports/downloads arrive as bytes
//! with a MIME type, not a path). Unsupported formats return `Err` — folder
//! treats that as "skipped" (kept out of purge, B57), Drive the same.

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
    "eml",
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
    match base.rsplit('.').next() {
        Some(ext) if ext != base.as_str() => CONFIG_EXTS.contains(&ext),
        _ => false,
    }
}

/// Extract text from a file on disk (folder connector).
pub fn extract_file(path: &Path) -> Result<String, String> {
    let kind = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase());
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    extract_bytes(&kind, &bytes)
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
        "html" | "htm" => Ok(strip_html(&decode_text(bytes)?)),
        "pdf" => extract_pdf(bytes),
        "docx" | "docm" => extract_docx(bytes),
        "pptx" | "pptm" => extract_pptx(bytes),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => extract_sheet(bytes),
        "odt" | "odp" => extract_odf(bytes),
        "rtf" => extract_rtf(bytes),
        "eml" => extract_eml(bytes),
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
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        _ => b as char,
    }
}

fn decode_cp1252(bytes: &[u8]) -> String {
    bytes.iter().map(|b| cp1252_byte(*b)).collect()
}

/// Strip tags/scripts/styles and collapse whitespace (same rules the folder
/// connector always used for `.html`).
fn strip_html(s: &str) -> String {
    use std::sync::OnceLock;
    static RE_SCRIPT: OnceLock<regex::Regex> = OnceLock::new();
    static RE_STYLE: OnceLock<regex::Regex> = OnceLock::new();
    static RE_TAG: OnceLock<regex::Regex> = OnceLock::new();
    static RE_WS: OnceLock<regex::Regex> = OnceLock::new();
    let re_script =
        RE_SCRIPT.get_or_init(|| regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
    let re_style =
        RE_STYLE.get_or_init(|| regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
    let re_tag = RE_TAG.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap());
    let re_ws = RE_WS.get_or_init(|| regex::Regex::new(r"\s+").unwrap());
    let mut t = re_script.replace_all(s, " ").to_string();
    t = re_style.replace_all(&t, " ").to_string();
    t = re_tag.replace_all(&t, " ").to_string();
    re_ws.replace_all(&t, " ").trim().to_string()
}

// ----- pdf -----

fn extract_pdf(bytes: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| format!("pdf extract: {e}"))
}

/// Resolve an XML general reference name (`amp`, `#169`, `#x20`) to a char.
fn resolve_ref(name: &str) -> Option<char> {
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

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))
}

fn read_zip_entry(
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

fn zip_names(archive: &zip::ZipArchive<Cursor<&[u8]>>, prefix: &str, suffix: &str) -> Vec<String> {
    archive
        .file_names()
        .filter_map(|n| n.ok())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .map(|n| n.to_string())
        .collect()
}

fn extract_docx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = open_zip(bytes)?;
    let xml = read_zip_entry(&mut archive, "word/document.xml")?;
    let text = xml_text(&xml, Profile::Word).map_err(|e| format!("docx: {e}"))?;
    Ok(normalize_lines(&text))
}

fn extract_pptx(bytes: &[u8]) -> Result<String, String> {
    let mut archive = open_zip(bytes)?;
    let mut slides = zip_names(&archive, "ppt/slides/", ".xml");
    slides.sort_by_key(|n| slide_number(n).unwrap_or(u32::MAX));
    let mut out = String::new();
    for (i, name) in slides.iter().enumerate() {
        let xml = read_zip_entry(&mut archive, name)?;
        let text = xml_text(&xml, Profile::Slides).map_err(|e| format!("{name}: {e}"))?;
        let text = normalize_lines(&text);
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("Slide {}\n", i + 1));
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
    let text = xml_text(&xml, Profile::Odf).map_err(|e| format!("odf: {e}"))?;
    Ok(normalize_lines(&text))
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

/// Walk XML and pull text out of one of the three document shapes.
fn xml_text(xml: &str, profile: Profile) -> Result<String, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = matches!(profile, Profile::Odf);
    let mut in_body = !matches!(profile, Profile::Odf);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let n: &str = name.as_ref();
                if n == "office:body" {
                    in_body = true;
                }
                if profile.text_elem(n) {
                    in_text = true;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let n: &str = name.as_ref();
                if profile.text_elem(n) {
                    in_text = false;
                }
                if n == "office:body" {
                    in_body = false;
                }
                if profile.cell_end(n) {
                    while out.ends_with('\n') {
                        out.pop();
                    }
                    out.push('\t');
                } else if profile.para_end(n) {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) => {
                if in_text && in_body {
                    let raw = t.into_inner();
                    let s = quick_xml::escape::unescape(&raw)
                        .map_err(|e| format!("xml entity: {e}"))?;
                    out.push_str(&s);
                }
            }
            Ok(Event::CData(t)) => {
                if in_text && in_body {
                    out.push_str(&t.into_inner());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if in_text && in_body {
                    if let Some(c) = resolve_ref(&r.into_inner()) {
                        out.push(c);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let n: &str = name.as_ref();
                if profile.tab(n) {
                    out.push('\t');
                } else if profile.line_break(n) || profile.para_end(n) {
                    out.push('\n');
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("xml: {e}")),
            _ => {}
        }
    }
    Ok(out)
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
        let two = text.find("Slide 2\nTwo").expect("slide 2");
        let ten = text.find("Slide 3\nTen").expect("slide 3");
        assert!(text.contains("Slide 1\nOne"));
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
    fn corrupt_inputs_error_instead_of_panicking() {
        assert!(extract_bytes("docx", b"not a zip").is_err());
        assert!(extract_bytes("pptx", b"not a zip").is_err());
        assert!(extract_bytes("xlsx", b"not a zip").is_err());
        assert!(extract_bytes("pdf", b"not a pdf").is_err());
        assert!(extract_bytes("ods", b"nope").is_err());
        assert!(extract_bytes("weird", b"x").is_err());
    }
}
