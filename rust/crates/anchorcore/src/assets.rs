//! Image assets (R46.7): every citable image's bytes live in the DB.
//!
//! Owner decision 2026-10-23 (Phase 46): bytes are stored for every source
//! kind, folders included — never a path reference or a later fetch — so an
//! export restores every image after the source is gone. Two origins:
//!
//! - **embed**: an image a document references (`![alt](src "caption")`,
//!   `<img>` in markdown, HTML `<img>`/`<figure>` via `extract.rs`). It sits on
//!   its hosting chunk/section, attribution `embedded`. Office/ODF images are
//!   `embedded:<part>` markers read from the doc package (folder: the file on
//!   disk; Drive: the downloaded bytes, R46.15). Folder sources resolve
//!   relative paths inside the source root (a link leaving the root never
//!   resolves); `data:` URIs resolve for any source. Remote URLs are skipped —
//!   no network on the ingest path; their alt text is already chunk text.
//! - **file**: an image file in a folder source is an item of its own (no text
//!   chunk for raster files; `.svg` keeps its R46.2 text) with one asset,
//!   attributed `referenced` when a synced document links it, else `none`.
//!
//! Identical bytes share one `asset_blobs` row (sha256); an unchanged hash
//! never rewrites the blob. Over `asset_max_bytes` an asset keeps its
//! metadata with `bytes_hash` NULL plus a `system_events` warning — the sync
//! never fails on an image. Re-sync keeps the deterministic `uid` and the
//! people-owned `manual_*` fields.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Provisional per-asset cap (R46.14 replaces it with a measured default).
pub const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Image files a folder source lists. `svg` is also a text kind (R46.2).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];
/// R46.8: Office images smaller than this on either side are decorative
/// (bullets, rules, spacer pixels) and never become assets.
const MIN_EMBEDDED_PX: u32 = 32;
/// R46.8: marker scheme the Office extractors write for package images.
pub const EMBEDDED_SCHEME: &str = "embedded:";
/// Context window around an image in its hosting chunk.
const CONTEXT_CHARS: usize = 240;

pub fn max_bytes(settings: &crate::settings::SettingsService, conn: &Connection) -> u64 {
    settings
        .get("asset_max_bytes", Some(conn))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_BYTES)
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn is_image_path(path: &Path) -> bool {
    IMAGE_EXTS.contains(&ext_of(path).as_str())
}

/// Image files with no text of their own (everything but SVG).
pub fn is_raster_image_path(path: &Path) -> bool {
    is_image_path(path) && ext_of(path) != "svg"
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Deterministic asset uid: re-sync and backup round-trips keep it.
fn asset_uid(item_uid: &str, key: &str, ordinal: usize) -> String {
    sha256_hex(format!("{item_uid}\0{key}\0{ordinal}").as_bytes())[..32].to_string()
}

// ----- image sniffing (no image crate: headers only) -----

#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    pub mime: &'static str,
    pub w: Option<u32>,
    pub h: Option<u32>,
}

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?])))
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?])))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8 | u32::from(*b.get(i + 2)?) << 16)
}

/// MIME + pixel size from the file header; `None` = not an image we keep.
pub fn sniff(b: &[u8]) -> Option<ImageInfo> {
    let info = |mime, w: Option<u32>, h: Option<u32>| Some(ImageInfo { mime, w, h });
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return info("image/png", be32(b, 16), be32(b, 20));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return info("image/gif", le16(b, 6), le16(b, 8));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let (w, h) = jpeg_size(b).unzip();
        return info("image/jpeg", w, h);
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        let (w, h) = webp_size(b).unzip();
        return info("image/webp", w, h);
    }
    if b.starts_with(b"BM") {
        let w = b.get(18..22).and_then(|s| s.try_into().ok()).map(i32::from_le_bytes);
        let h = b.get(22..26).and_then(|s| s.try_into().ok()).map(i32::from_le_bytes);
        return info("image/bmp", w.map(i32::unsigned_abs), h.map(i32::unsigned_abs));
    }
    let head = String::from_utf8_lossy(&b[..b.len().min(2048)]).to_ascii_lowercase();
    if head.contains("<svg") {
        return info("image/svg+xml", None, None);
    }
    None
}

/// Walk JPEG segments to the first SOF marker (baseline/progressive/...).
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 3 < b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0xD0..=0xD9 | 0x01) {
            i += 2;
            continue;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((be16(b, i + 7)?, be16(b, i + 5)?));
        }
        i += 2 + be16(b, i + 2)? as usize;
    }
    None
}

fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    match b.get(12..16)? {
        b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
        b"VP8L" => {
            let bits = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => Some((le24(b, 24)? + 1, le24(b, 27)? + 1)),
        _ => None,
    }
}

// ----- references in chunk text -----

#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    pub src: String,
    pub alt: String,
    pub caption: String,
    /// Byte range of the reference inside the chunk text.
    pub span: (usize, usize),
}

fn md_image_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"!\[(?P<alt>[^\]\n]*)\]\(\s*(?:<(?P<src2>[^>\n]+)>|(?P<src>[^\s)]+))(?:\s+(?:"(?P<title>[^"\n]*)"|'(?P<title2>[^'\n]*)'))?\s*\)"#)
            .expect("static regex")
    })
}

fn html_img_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?is)<img\b[^>]*>").expect("static regex"))
}

/// `name="value"` / `name='value'` from one tag (case-insensitive name).
pub fn tag_attr(tag: &str, name: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r#"(?is)\b{name}\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)).ok()?;
    let c = re.captures(tag)?;
    c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map(|m| m.as_str().to_string())
}

/// Image references in one chunk, in text order. Fenced code is skipped
/// (a `![x](y)` in a code sample is not an image).
pub fn scan_refs(text: &str) -> Vec<ImageRef> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fence: Option<String> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker: String = trimmed.chars().take_while(|c| *c == '`' || *c == '~').collect();
        if marker.len() >= 3 && (marker.chars().all(|c| c == '`') || marker.chars().all(|c| c == '~')) {
            match &fence {
                Some(open) if marker.starts_with(open.as_str()) => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
        } else if fence.is_none() {
            scan_line(line, offset, &mut out);
        }
        offset += line.len();
    }
    out.sort_by_key(|r| r.span.0);
    out
}

fn scan_line(line: &str, offset: usize, out: &mut Vec<ImageRef>) {
    for c in md_image_re().captures_iter(line) {
        let m = c.get(0).expect("whole match");
        let src = c.name("src2").or_else(|| c.name("src")).map(|s| s.as_str().trim()).unwrap_or("");
        out.push(ImageRef {
            src: src.to_string(),
            alt: c.name("alt").map(|s| s.as_str().trim().to_string()).unwrap_or_default(),
            caption: c.name("title").or_else(|| c.name("title2")).map(|s| s.as_str().trim().to_string()).unwrap_or_default(),
            span: (offset + m.start(), offset + m.end()),
        });
    }
    for m in html_img_re().find_iter(line) {
        let Some(src) = tag_attr(m.as_str(), "src") else { continue };
        out.push(ImageRef {
            src,
            alt: tag_attr(m.as_str(), "alt").unwrap_or_default(),
            caption: tag_attr(m.as_str(), "title").unwrap_or_default(),
            span: (offset + m.start(), offset + m.end()),
        });
    }
}

/// Breadcrumb + the text around the reference (image syntax removed).
fn context_for(chunk_path: &str, text: &str, span: (usize, usize)) -> String {
    let floor = |mut i: usize| {
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let start = floor(span.0.saturating_sub(CONTEXT_CHARS));
    let end = floor((span.1 + CONTEXT_CHARS).min(text.len()));
    let window = format!("{} {}", &text[start..span.0], &text[span.1..end]);
    let window = md_image_re().replace_all(&window, " ");
    let window = html_img_re().replace_all(&window, " ");
    // A window edge can cut a ref in half; its fragments are not context.
    let window = window.split_whitespace().filter(|w| !w.contains(EMBEDDED_SCHEME) && !w.contains("](")).collect::<Vec<_>>().join(" ");
    match (chunk_path.trim().is_empty(), window.is_empty()) {
        (true, _) => window,
        (false, true) => chunk_path.trim().to_string(),
        (false, false) => format!("{} — {window}", chunk_path.trim()),
    }
}

// ----- resolution -----

/// Where a document's relative image paths resolve: the folder source root
/// (canonical) and the document's own absolute path.
#[derive(Debug, Clone)]
pub struct DocPlace {
    pub root: PathBuf,
    pub doc: PathBuf,
}

/// Bytes (or just the size when over the cap) for one reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub bytes: Option<Vec<u8>>,
    pub size: u64,
    pub ref_path: String,
    pub info: ImageInfo,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        if let Some(v) = hex {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve a reference to stored bytes. `None` = not an image we keep
/// (remote URL, missing file, outside the root, not an image).
pub fn resolve(src: &str, place: Option<&DocPlace>, cap: u64) -> Option<Resolved> {
    let src = src.trim();
    if let Some(rest) = src.strip_prefix("data:") {
        return resolve_data_uri(rest, cap);
    }
    if src.contains("://") || src.starts_with("//") || src.starts_with("mailto:") {
        return None;
    }
    resolve_file(src, place?, cap)
}

fn resolve_data_uri(rest: &str, cap: u64) -> Option<Resolved> {
    let (meta, data) = rest.split_once(',')?;
    if !meta.ends_with(";base64") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.split_whitespace().collect::<String>())
        .ok()?;
    let info = sniff(&bytes)?;
    let size = bytes.len() as u64;
    Some(Resolved { bytes: (size <= cap).then_some(bytes), size, ref_path: String::new(), info })
}

fn resolve_file(src: &str, place: &DocPlace, cap: u64) -> Option<Resolved> {
    let path_part = src.split(['?', '#']).next()?;
    let decoded = percent_decode(path_part);
    if decoded.is_empty() {
        return None;
    }
    // `/img/a.png` is root-relative (site style); everything else is
    // relative to the document's own folder.
    let base = if decoded.starts_with('/') { place.root.clone() } else { place.doc.parent()?.to_path_buf() };
    let full = base.join(decoded.trim_start_matches('/')).canonicalize().ok()?;
    // Canonical paths: `..` and symlinks are already resolved, so a link
    // leaving the source root can never sneak back in.
    let rel = full.strip_prefix(&place.root).ok()?.to_string_lossy().to_string();
    let meta = std::fs::metadata(&full).ok().filter(|m| m.is_file())?;
    if !is_image_path(&full) {
        return None;
    }
    let size = meta.len();
    if size > cap {
        let info = ImageInfo { mime: mime_for_ext(&full), w: None, h: None };
        return Some(Resolved { bytes: None, size, ref_path: rel, info });
    }
    let bytes = std::fs::read(&full).ok()?;
    let info = sniff(&bytes)?;
    Some(Resolved { bytes: Some(bytes), size, ref_path: rel, info })
}

/// R46.8: one image part inside the document's own zip package (docx
/// `word/media`, pptx `ppt/media`, ODF `Pictures/`). EMF/WMF and other
/// unsniffable parts, and decorative images under `MIN_EMBEDDED_PX`, are
/// skipped. Folder sources read the package from disk; Drive hands the
/// downloaded bytes through (R46.15); other connectors have no packages.
pub fn resolve_embedded(part: &str, package: &[u8], cap: u64) -> Option<Resolved> {
    use std::io::Read;
    let part = percent_decode(part);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(package)).ok()?;
    let mut entry = zip.by_name(&part).ok()?;
    let size = entry.size();
    if size > cap {
        let info = ImageInfo { mime: mime_for_ext(Path::new(&part)), w: None, h: None };
        return Some(Resolved { bytes: None, size, ref_path: String::new(), info });
    }
    let mut bytes = Vec::new();
    entry.by_ref().take(cap + 1).read_to_end(&mut bytes).ok()?;
    let info = sniff(&bytes)?;
    if info.w.is_some_and(|w| w < MIN_EMBEDDED_PX) || info.h.is_some_and(|h| h < MIN_EMBEDDED_PX) {
        return None;
    }
    Some(Resolved { size: bytes.len() as u64, bytes: Some(bytes), ref_path: String::new(), info })
}

fn mime_for_ext(path: &Path) -> &'static str {
    match ext_of(path).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Read one image file for its own item (cap applies before reading).
pub fn read_file(path: &Path, rel: &str, cap: u64) -> Option<Resolved> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > cap {
        let info = ImageInfo { mime: mime_for_ext(path), w: None, h: None };
        return Some(Resolved { bytes: None, size, ref_path: rel.to_string(), info });
    }
    let bytes = std::fs::read(path).ok()?;
    let info = sniff(&bytes)?;
    Some(Resolved { bytes: Some(bytes), size, ref_path: rel.to_string(), info })
}

// ----- DB writes -----

/// One asset row to upsert (everything sync owns; never the manual fields).
#[derive(Debug, Clone)]
pub struct AssetRow {
    pub uid: String,
    pub item_id: i64,
    pub section_id: Option<i64>,
    pub chunk_id: Option<i64>,
    pub origin: &'static str,
    pub src: String,
    pub alt: String,
    pub caption: String,
    pub context: String,
    pub attribution: &'static str,
    pub resolved: Resolved,
}

/// Upsert one asset. The blob is written only when the hash is new to this
/// row (an unchanged image never rewrites its bytes); returns true when the
/// asset is over the cap and newly so (the caller warns once, not per sync).
pub fn upsert(conn: &Connection, row: &AssetRow) -> Result<bool, String> {
    let r = &row.resolved;
    let hash = r.bytes.as_deref().map(sha256_hex);
    let existing: Option<(i64, Option<String>)> = conn
        .query_row("SELECT id, bytes_hash FROM assets WHERE uid = ?1", [&row.uid], |x| Ok((x.get(0)?, x.get(1)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    if let (Some(h), Some(bytes)) = (&hash, &r.bytes) {
        if existing.as_ref().and_then(|e| e.1.as_ref()) != Some(h) {
            conn.execute(
                "INSERT OR IGNORE INTO asset_blobs (hash, bytes, size) VALUES (?1, ?2, ?3)",
                params![h, bytes, r.size as i64],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let newly_over = hash.is_none() && existing.as_ref().is_none_or(|e| e.1.is_some());
    match existing {
        Some((id, _)) => conn.execute(
            "UPDATE assets SET section_id=?2, chunk_id=?3, src=?4, ref_path=?5, mime=?6, w=?7, h=?8, bytes_hash=?9, size=?10, \
             alt=?11, caption=?12, context=?13, \
             attribution = CASE WHEN attribution IN ('manual','ignored') THEN attribution ELSE ?14 END, \
             updated_at=datetime('now') WHERE id=?1",
            params![id, row.section_id, row.chunk_id, row.src, r.ref_path, r.info.mime, r.info.w, r.info.h, hash, r.size as i64, row.alt, row.caption, row.context, row.attribution],
        ),
        None => conn.execute(
            "INSERT INTO assets (uid, item_id, section_id, chunk_id, origin, src, ref_path, mime, w, h, bytes_hash, size, alt, caption, context, attribution) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![row.uid, row.item_id, row.section_id, row.chunk_id, row.origin, row.src, r.ref_path, r.info.mime, r.info.w, r.info.h, hash, r.size as i64, row.alt, row.caption, row.context, row.attribution],
        ),
    }
    .map_err(|e| e.to_string())?;
    Ok(newly_over)
}

fn warn_over_cap(conn: &Connection, source_id: i64, what: &str, size: u64, cap: u64) {
    tracing::warn!("asset over size cap ({what}): {size} bytes > {cap}");
    let _ = conn.execute(
        "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'asset over size cap',?2)",
        params![source_id, serde_json::json!({"asset": what, "size": size, "cap": cap}).to_string()],
    );
}

/// The hosting chunks of one item, in order: (chunk id, section id, path, content).
type HostChunk = (i64, Option<i64>, String, String);

fn host_chunks(conn: &Connection, item_id: i64) -> Result<Vec<HostChunk>, String> {
    let mut stmt = conn
        .prepare("SELECT id, section_id, path, content FROM chunks WHERE item_id=?1 AND entity_id IS NULL AND kind='document' ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Embedded assets of one document, rebuilt after its chunks are stored:
/// upsert every resolvable reference (stable uid per src + ordinal), drop
/// the item's embed rows that no longer appear. File reads happen before
/// the write transaction opens (single-writer rule). `package` is the
/// document's Office package when it is held in memory (Drive, R46.15);
/// otherwise `embedded:` markers read it from `place.doc` (folder).
pub fn store_embedded(conn: &Connection, source_id: i64, item_id: i64, place: Option<&DocPlace>, package: Option<&[u8]>, cap: u64) -> Result<usize, String> {
    let item_uid: String = conn
        .query_row("SELECT COALESCE(uid, '') FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    let mut ordinals: HashMap<String, usize> = HashMap::new();
    let mut packages = Packages { given: package, place, disk: None, pdf: None };
    let pages = pdf_ref_pages(conn, item_id);
    let in_text = ref_counts(conn, item_id);
    for (chunk_id, section_id, path, content) in host_chunks(conn, item_id)? {
        let refs = scan_refs(&content);
        // R46.16: an image on a chunk with no words of its own (a bare
        // slide) has no context to be found by — unplaced.
        let chunk_words = has_words(&content, &refs);
        for r in refs {
            let n = ordinals.entry(r.src.clone()).or_default();
            // Prose chunks overlap: a ref past its count in the document text
            // is the next chunk re-reading the same bytes, not a new image.
            if in_text.get(&r.src).is_some_and(|max| *n >= *max) {
                continue;
            }
            *n += 1;
            // PDF chunks pack several pages: page + words come from the page.
            let page = pages.get(&(r.src.clone(), *n));
            let words = page.map_or(chunk_words, |p| p.1);
            let crumb = page.map_or_else(|| path.clone(), |p| format!("p. {}", p.0));
            let resolved = match r.src.strip_prefix(EMBEDDED_SCHEME) {
                Some(part) => packages.resolve(part, cap),
                None => resolve(&r.src, place, cap),
            };
            let Some(resolved) = resolved else { continue };
            rows.push(AssetRow {
                uid: asset_uid(&item_uid, &r.src, *n),
                item_id,
                section_id,
                chunk_id: Some(chunk_id),
                origin: "embed",
                src: if r.src.starts_with("data:") { "data:".to_string() } else { r.src.clone() },
                alt: r.alt.clone(),
                caption: r.caption.clone(),
                context: context_for(&crumb, &content, r.span),
                attribution: if words { "embedded" } else { "none" },
                resolved,
            });
        }
    }
    conn.execute("BEGIN IMMEDIATE", []).map_err(|e| e.to_string())?;
    let res = write_embedded(conn, source_id, item_id, &rows, cap);
    conn.execute(if res.is_ok() { "COMMIT" } else { "ROLLBACK" }, []).map_err(|e| e.to_string())?;
    res?;
    // Embed assets just moved to the new sections: people's captions on
    // them follow (R46.12/R46.16).
    crate::captions::refresh_for_item(conn, item_id)?;
    Ok(rows.len())
}

/// How often each ref occurs in the item text: the true count that
/// overlapping chunk windows must not exceed. Empty when the item keeps no
/// text (the cap then does not apply).
fn ref_counts(conn: &Connection, item_id: i64) -> HashMap<String, usize> {
    let text: String = conn.query_row("SELECT text FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_default();
    let mut out = HashMap::new();
    for r in scan_refs(&text) {
        *out.entry(r.src).or_default() += 1;
    }
    out
}

/// R46.16: (src, ordinal) → (page, page has words) for the `embedded:pdf:`
/// refs of a PDF item, read from the item text (which keeps `[page N]`
/// markers; stored chunks do not). Empty for every other item.
fn pdf_ref_pages(conn: &Connection, item_id: i64) -> HashMap<(String, usize), (u32, bool)> {
    let needle = format!("{EMBEDDED_SCHEME}{}", crate::pdf_images::PART_PREFIX);
    let text: Option<String> = conn
        .query_row("SELECT text FROM ingested_items WHERE id=?1 AND instr(text, ?2) > 0", params![item_id, needle], |r| r.get(0))
        .ok();
    let mut out = HashMap::new();
    let Some(text) = text else { return out };
    let mut ordinals: HashMap<String, usize> = HashMap::new();
    let mut page = 0u32;
    let mut block = String::new();
    let mut pending: Vec<(String, usize)> = Vec::new();
    let flush = |page: u32, block: &str, pending: &mut Vec<(String, usize)>, out: &mut HashMap<(String, usize), (u32, bool)>| {
        let words = has_words(block, &scan_refs(block));
        for key in pending.drain(..) {
            out.insert(key, (page, words));
        }
    };
    for line in text.lines() {
        if let Some(n) = crate::extract::page_marker_number(line) {
            flush(page, &block, &mut pending, &mut out);
            (page, block) = (n, String::new());
            continue;
        }
        for r in scan_refs(line) {
            let n = ordinals.entry(r.src.clone()).or_default();
            *n += 1;
            pending.push((r.src, *n));
        }
        block.push_str(line);
        block.push('\n');
    }
    flush(page, &block, &mut pending, &mut out);
    out
}

/// What sync derives for an embed asset (`embedded` or `none`, R46.16), so
/// a person clearing their fields hands back the same split. The uid is
/// deterministic (item uid + src + ordinal), which recovers the ordinal.
pub fn embed_sync_attribution(conn: &Connection, asset_id: i64) -> Result<&'static str, String> {
    let (uid, item_id, src, host, item_uid): (String, i64, String, String, String) = conn
        .query_row(
            "SELECT a.uid, a.item_id, a.src, COALESCE(c.content, ''), COALESCE(i.uid, '') FROM assets a \
             JOIN ingested_items i ON i.id = a.item_id LEFT JOIN chunks c ON c.id = a.chunk_id WHERE a.id=?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|e| e.to_string())?;
    let page_words = pdf_ref_pages(conn, item_id)
        .into_iter()
        .find(|((s, n), _)| *s == src && asset_uid(&item_uid, s, *n) == uid)
        .map(|(_, (_, words))| words);
    let words = page_words.unwrap_or_else(|| has_words(&host, &scan_refs(&host)));
    Ok(if words { "embedded" } else { "none" })
}

/// True when a chunk carries text besides its image references.
pub fn has_words(content: &str, refs: &[ImageRef]) -> bool {
    let mut rest = String::with_capacity(content.len());
    let mut at = 0;
    for r in refs {
        rest.push_str(&content[at..r.span.0]);
        at = r.span.1;
    }
    rest.push_str(&content[at..]);
    rest.chars().any(char::is_alphanumeric)
}

/// The document's own container, read once for all its images: an Office
/// zip (R46.8) or a PDF (R46.16), held in memory (Drive, R46.15) or read
/// from `place.doc` (folder).
struct Packages<'a> {
    given: Option<&'a [u8]>,
    place: Option<&'a DocPlace>,
    disk: Option<Option<Vec<u8>>>,
    pdf: Option<Option<lopdf::Document>>,
}

impl Packages<'_> {
    fn bytes(&mut self) -> Option<&[u8]> {
        let place = self.place;
        match self.given {
            Some(b) => Some(b),
            None => self.disk.get_or_insert_with(|| place.and_then(|p| std::fs::read(&p.doc).ok())).as_deref(),
        }
    }

    fn resolve(&mut self, part: &str, cap: u64) -> Option<Resolved> {
        if !part.starts_with(crate::pdf_images::PART_PREFIX) {
            return self.bytes().and_then(|b| resolve_embedded(part, b, cap));
        }
        if self.pdf.is_none() {
            let doc = self.bytes().and_then(|b| lopdf::Document::load_mem(b).ok());
            self.pdf = Some(doc);
        }
        let img = crate::pdf_images::decode(self.pdf.as_ref()?.as_ref()?, part, cap)?;
        if img.w < MIN_EMBEDDED_PX || img.h < MIN_EMBEDDED_PX {
            return None;
        }
        let info = ImageInfo { mime: img.mime, w: Some(img.w), h: Some(img.h) };
        Some(Resolved { bytes: img.bytes, size: img.size, ref_path: String::new(), info })
    }
}

fn write_embedded(conn: &Connection, source_id: i64, item_id: i64, rows: &[AssetRow], cap: u64) -> Result<(), String> {
    let keep: HashSet<&str> = rows.iter().map(|r| r.uid.as_str()).collect();
    let mut stmt = conn
        .prepare("SELECT id, uid FROM assets WHERE item_id=?1 AND origin='embed'")
        .map_err(|e| e.to_string())?;
    let stale: Vec<i64> = stmt
        .query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|(_, uid)| !keep.contains(uid.as_str()))
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        conn.execute("DELETE FROM assets WHERE id=?1", [id]).map_err(|e| e.to_string())?;
    }
    for row in rows {
        if upsert(conn, row)? {
            warn_over_cap(conn, source_id, &row.src, row.resolved.size, cap);
        }
    }
    Ok(())
}

/// The asset of an image file item (`origin='file'`). Attribution starts at
/// `none`; `refresh_file_attribution` sets `referenced` after the sync walks
/// every document.
pub fn store_file(conn: &Connection, source_id: i64, item_id: i64, resolved: Resolved, cap: u64) -> Result<(), String> {
    let item_uid: String = conn
        .query_row("SELECT COALESCE(uid, '') FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let src = resolved.ref_path.clone();
    let row = AssetRow {
        uid: asset_uid(&item_uid, "\0file", 1),
        item_id,
        section_id: None,
        chunk_id: None,
        origin: "file",
        src: src.clone(),
        alt: String::new(),
        caption: String::new(),
        context: String::new(),
        attribution: "none",
        resolved,
    };
    if upsert(conn, &row)? {
        warn_over_cap(conn, source_id, &src, row.resolved.size, cap);
    }
    Ok(())
}

/// File assets of one source: `referenced` when any embed asset of the same
/// source resolved to that file, else `none`. People's `manual`/`ignored`
/// attributions are never touched.
pub fn refresh_file_attribution(conn: &Connection, source_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE assets SET attribution = CASE WHEN EXISTS (
             SELECT 1 FROM assets e JOIN ingested_items ei ON ei.id = e.item_id
             WHERE e.origin = 'embed' AND ei.source_id = ?1 AND e.ref_path <> '' AND e.ref_path = fi.external_id
         ) THEN 'referenced' ELSE 'none' END
         FROM ingested_items fi
         WHERE fi.id = assets.item_id AND fi.source_id = ?1 AND assets.origin = 'file'
           AND assets.attribution IN ('referenced', 'none')",
        [source_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ----- read side (R46.9) -----

/// Where an asset's bytes are served: never the filesystem, always the DB.
pub const MEDIA_ROUTE: &str = "/media";

/// An image reference as agents and the UI see it (search hits, agent
/// citations, item detail). `caption` prefers a person's caption.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct AssetRef {
    pub asset_id: String,
    pub alt: String,
    pub caption: String,
    pub mime: String,
    pub w: Option<i64>,
    pub h: Option<i64>,
    /// `GET` path for the bytes; `None` when they are not stored (over cap).
    pub url: Option<String>,
}

const REF_COLUMNS: &str = "uid, alt, CASE WHEN manual_caption <> '' THEN manual_caption ELSE caption END, mime, w, h, bytes_hash IS NOT NULL";

fn ref_row(r: &rusqlite::Row) -> rusqlite::Result<AssetRef> {
    let uid: String = r.get(0)?;
    let stored: bool = r.get(6)?;
    Ok(AssetRef {
        url: stored.then(|| format!("{MEDIA_ROUTE}/{uid}")),
        asset_id: uid,
        alt: r.get(1)?,
        caption: r.get(2)?,
        mime: r.get(3)?,
        w: r.get(4)?,
        h: r.get(5)?,
    })
}

/// Images shown with a hit: those on its chunk, plus those elsewhere in
/// its returned section (merged parent hits span several chunks), plus
/// the image a person's caption chunk there describes (R46.12).
/// People-`ignored` assets never surface.
pub fn refs_for_hit(conn: &Connection, chunk_id: i64, section_id: Option<i64>) -> Vec<AssetRef> {
    let sql = format!(
        "SELECT {REF_COLUMNS} FROM assets WHERE attribution <> 'ignored' \
         AND (chunk_id = ?1 OR (?2 IS NOT NULL AND section_id = ?2) \
           OR 'caption:' || uid IN (SELECT c.uid FROM chunks c WHERE c.kind = 'caption' AND (c.id = ?1 OR (?2 IS NOT NULL AND c.section_id = ?2)))) \
         ORDER BY id"
    );
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    stmt.query_map(params![chunk_id, section_id], ref_row)
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Every asset of an item (item drill-down), embeds and its own file.
pub fn refs_for_item(conn: &Connection, item_id: i64) -> Vec<Value> {
    let sql = format!("SELECT {REF_COLUMNS}, origin, attribution, src FROM assets WHERE item_id = ?1 ORDER BY id");
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    stmt.query_map([item_id], |r| {
        let mut v = serde_json::to_value(ref_row(r)?).unwrap_or_default();
        v["origin"] = serde_json::json!(r.get::<_, String>(7)?);
        v["attribution"] = serde_json::json!(r.get::<_, String>(8)?);
        v["src"] = serde_json::json!(r.get::<_, String>(9)?);
        Ok(v)
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// One asset for `GET /media/:uid`: (mime, source label, bytes if stored, (w, h)).
pub type Media = (String, String, Option<Vec<u8>>, (Option<i64>, Option<i64>));

/// The label slot is the gate label: the source label, or
/// `common::ITEM_SENSITIVE_MARK` when the item carries the override (R35.4).
pub fn load_media(conn: &Connection, uid: &str) -> Result<Option<Media>, String> {
    conn.query_row(
        "SELECT a.mime, CASE WHEN i.sensitive_override = 1 AND COALESCE(s.label, '') <> 'sensitive' THEN 'item_sensitive' ELSE s.label END, \
         b.bytes, a.w, a.h FROM assets a \
         JOIN ingested_items i ON i.id = a.item_id JOIN sources s ON s.id = i.source_id \
         LEFT JOIN asset_blobs b ON b.hash = a.bytes_hash WHERE a.uid = ?1",
        [uid],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, (r.get(3)?, r.get(4)?))),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Documents whose chunks can carry image references: markdown (not the
/// rendered diagram kinds), HTML, and Office/ODF packages (R46.8).
pub fn hosts_images(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let html = base.ends_with(".html") || base.ends_with(".htm");
    // R46.8: Office/ODF documents carry `embedded:` markers; R46.16: PDFs too.
    let office = [".docx", ".docm", ".pptx", ".pptm", ".odt", ".odp", ".pdf"].iter().any(|e| base.ends_with(e));
    html || office || (crate::markdown::is_markdown_filename(&base) && crate::extract::diagram_kind(&base).is_none())
}

/// 1×1 transparent PNG (shared by the pipeline and backup tests).
#[cfg(test)]
pub(crate) const TEST_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_reads_headers() {
        assert_eq!(sniff(TEST_PNG), Some(ImageInfo { mime: "image/png", w: Some(1), h: Some(1) }));
        let gif = b"GIF89a\x40\x01\xF0\x00rest";
        assert_eq!(sniff(gif), Some(ImageInfo { mime: "image/gif", w: Some(320), h: Some(240) }));
        // JPEG: SOI, APP0 (len 4), SOF0 with h=480 w=640.
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80];
        assert_eq!(sniff(&jpg), Some(ImageInfo { mime: "image/jpeg", w: Some(640), h: Some(480) }));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[0; 8]);
        webp.extend_from_slice(&[0x7F, 0x02, 0x00, 0xDF, 0x01, 0x00]); // 640×480 (minus one encoded)
        assert_eq!(sniff(&webp), Some(ImageInfo { mime: "image/webp", w: Some(640), h: Some(480) }));
        assert_eq!(sniff(b"<?xml?><svg width='1'/>").map(|i| i.mime), Some("image/svg+xml"));
        assert_eq!(sniff(b"%PDF-1.7"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn scan_refs_markdown_html_and_fences() {
        let quoted = scan_refs("![a](x.png \"Bob's screen\") ![b](y.png 'say \"hi\"')");
        assert_eq!(quoted.iter().map(|r| r.caption.as_str()).collect::<Vec<_>>(), vec!["Bob's screen", "say \"hi\""]);
        let text = "Intro ![Login screen](img/login.png \"The login\") and <img src='a b.png' alt=\"Chart\">\n```\n![not](code.png)\n```\n![x](<with space.png>)\n";
        let refs = scan_refs(text);
        let got: Vec<(&str, &str, &str)> = refs.iter().map(|r| (r.src.as_str(), r.alt.as_str(), r.caption.as_str())).collect();
        assert_eq!(got, vec![("img/login.png", "Login screen", "The login"), ("a b.png", "Chart", ""), ("with space.png", "x", "")]);
        assert_eq!(&text[refs[0].span.0..refs[0].span.1], "![Login screen](img/login.png \"The login\")");
    }

    #[test]
    fn context_strips_image_syntax_and_keeps_breadcrumb() {
        let text = "Before the picture. ![a](b.png) After it, more words.";
        let r = &scan_refs(text)[0];
        assert_eq!(context_for("Guide > Login", text, r.span), "Guide > Login — Before the picture. After it, more words.");
        assert_eq!(context_for("", text, r.span), "Before the picture. After it, more words.");
    }

    #[test]
    fn resolve_files_stay_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().canonicalize().unwrap();
        let root = outer.join("root");
        std::fs::create_dir_all(root.join("docs/img")).unwrap();
        std::fs::write(root.join("docs/img/a b.png"), TEST_PNG).unwrap();
        std::fs::write(root.join("logo.png"), TEST_PNG).unwrap();
        std::fs::write(outer.join("secret.png"), TEST_PNG).unwrap();
        std::fs::write(root.join("docs/notes.txt"), b"not an image").unwrap();
        let place = DocPlace { root: root.clone(), doc: root.join("docs/guide.md") };
        let rel = |src: &str| resolve(src, Some(&place), DEFAULT_MAX_BYTES).map(|r| r.ref_path);
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(rel("img/a%20b.png"), Some(format!("docs{sep}img{sep}a b.png")));
        assert_eq!(rel("./img/a b.png?raw=1#x"), Some(format!("docs{sep}img{sep}a b.png")));
        assert_eq!(rel("/logo.png"), Some("logo.png".to_string()));
        assert_eq!(rel("../logo.png"), Some("logo.png".to_string()));
        assert_eq!(rel("../../secret.png"), None, "outside the root");
        assert_eq!(rel("notes.txt"), None, "not an image");
        assert_eq!(rel("missing.png"), None);
        assert_eq!(rel("https://example.com/a.png"), None, "no network");
        assert!(resolve("img/a b.png", None, DEFAULT_MAX_BYTES).is_none(), "no root → no files");
        let over = resolve("/logo.png", Some(&place), 10).expect("metadata kept");
        assert_eq!((over.bytes, over.size), (None, TEST_PNG.len() as u64));
    }

    #[test]
    fn resolve_data_uri_any_source() {
        let uri = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(TEST_PNG));
        let r = resolve(&uri, None, DEFAULT_MAX_BYTES).expect("data uri");
        assert_eq!(r.bytes.as_deref(), Some(TEST_PNG));
        assert_eq!(r.info.mime, "image/png");
        assert!(resolve("data:text/plain,hello", None, DEFAULT_MAX_BYTES).is_none());
        assert!(resolve("data:image/png;base64,!!!", None, DEFAULT_MAX_BYTES).is_none());
    }

    #[test]
    fn hosts_images_is_markdown_and_html_only() {
        for yes in ["a.md", "docs/b.MARKDOWN", "c.html", "d.htm", "e.docx", "f.PPTX", "g.odt", "h.pdf"] {
            assert!(hosts_images(yes), "{yes}");
        }
        for no in ["a.drawio", "b.excalidraw.json", "c.svg", "d.txt", "e.py", "gdrive:abc"] {
            assert!(!hosts_images(no), "{no}");
        }
    }
}
