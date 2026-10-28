//! Whole-item fetch — full file bytes in one call.
//!
//! Search hits and citations carry bounded context (`content_truncated`
//! says when expansion drops siblings); `GET /items/:id` is the drill-down
//! that returns the complete ingested text.
//!
//! R35.4: items carry the Sensitive override (whole document, people-driven,
//! escalate-only). `PATCH /items/:id {"sensitive_override": true}` sets it,
//! `null` clears it back to inheriting the source label; `false` is reserved
//! (400), and the removed `is_pii` key is a 400 naming the new field.
//!
//! R45.13: `PATCH /items/:id {"is_glossary": bool}` marks a file as a
//! glossary — its rows (CSV/TSV, Markdown tables, `CODE - x`, `Term: x`)
//! become active vocabulary right away; unmarking removes them.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::health::AppState;
use crate::read_gate::ReadRefusal;

#[derive(Deserialize)]
pub struct UpdateItemPayload {
    /// R35.4: tri-state write — true sets the override, null clears it
    /// (inherit), false is a 400 (0 is reserved). Absent = no change.
    /// Stock `Option<Option<bool>>` reads explicit null as absent, so a
    /// custom deserializer keeps present-null distinct from missing.
    #[serde(default, deserialize_with = "de_tri_state")]
    pub sensitive_override: Option<Option<bool>>,
    /// Removed in R35.4: any present `is_pii` key is a 400 naming the new
    /// field (presence, not value, is the error — even explicit null).
    #[serde(default, deserialize_with = "de_present_flag")]
    pub is_pii: bool,
    #[serde(default)]
    pub is_glossary: Option<bool>,
}

/// Presence detector for removed keys: missing reads false (default), any
/// present value — including null — reads true.
fn de_present_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(d).map(|_| true)
}

/// Tri-state write: this runs only when the key is present, so null reads
/// `Some(None)` (clear) instead of collapsing to absent.
fn de_tri_state<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<bool>>, D::Error> {
    Option::<bool>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub path_prefix: Option<String>,
    pub source_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// R36.15: only items of `public`-labelled sources (the share-safe scope).
    pub public_only: Option<bool>,
}

/// R36.15: `GET /items/:id` read options.
#[derive(Deserialize)]
pub struct GetQuery {
    /// Line window `a-b` (1-based, inclusive), `a` (one line) or `a-` (to
    /// the end) over the stored item text.
    pub lines: Option<String>,
    pub public_only: Option<bool>,
}

/// R36.15: the accepted `path_prefix` forms, named in every rejection.
const PREFIX_FORM: &str = "Expected a relative item path as GET /items lists it (e.g. \"docs/\" or \"cfg/routing\") or an absolute path under a folder source's root (e.g. \"/data/corpus/cfg\" or \"C:/corpus/cfg\"); no wildcards, \"..\" segments, leading \"./\" or \"~\", or URL schemes";

/// R36.15: why a `path_prefix` can never match a stored item path, if it
/// can't. Prefixes are literal (LIKE-escaped), so these inputs used to
/// return an empty list that read as "nothing there".
fn prefix_defect(p: &str) -> Option<&'static str> {
    let n = p.replace('\\', "/");
    if p.chars().any(|c| c.is_control()) {
        return Some("it contains control characters");
    }
    if p.contains('*') || p.contains('?') {
        return Some("wildcards are not supported (prefixes match literally)");
    }
    if n.contains("://") {
        return Some("URLs are not item paths");
    }
    if n.split('/').any(|seg| seg == "..") {
        return Some("\"..\" segments never occur in stored paths");
    }
    if n.starts_with("./") || n.starts_with('~') {
        return Some("stored paths have no leading \"./\" or \"~\" (use the relative path, or the absolute path)");
    }
    let b = n.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() && b.get(2) != Some(&b'/') {
        return Some("drive-relative paths (\"C:x\") are ambiguous");
    }
    None
}

/// R36.15: shared by `GET /items` and `POST /qa/search` — a malformed
/// prefix is a 400 naming the expected form, never an empty result. Empty
/// or whitespace-only prefixes stay "unfiltered".
pub fn validate_path_prefix(prefix: Option<&str>) -> Result<(), String> {
    let Some(p) = prefix else { return Ok(()) };
    if p.trim().is_empty() {
        return Ok(());
    }
    match prefix_defect(p) {
        Some(why) => Err(format!("path_prefix {p:?} is malformed: {why}. {PREFIX_FORM}.")),
        None => Ok(()),
    }
}

/// R36.15: a parsed `?lines=` window (1-based, inclusive; `end: None` = EOF).
#[derive(Debug, PartialEq)]
pub struct LineWindow {
    pub start: usize,
    pub end: Option<usize>,
}

const LINES_FORM: &str = "lines must be \"a-b\" (1-based, inclusive, a <= b), \"a\" (one line) or \"a-\" (line a to the end)";

fn parse_line_no(s: &str) -> Option<usize> {
    s.trim().parse::<usize>().ok().filter(|n| *n >= 1)
}

pub fn parse_lines(raw: &str) -> Result<LineWindow, String> {
    let bad = || format!("{LINES_FORM} — got {raw:?}");
    let Some((a, b)) = raw.split_once('-') else {
        let n = parse_line_no(raw).ok_or_else(bad)?;
        return Ok(LineWindow { start: n, end: Some(n) });
    };
    let start = parse_line_no(a).ok_or_else(bad)?;
    if b.trim().is_empty() {
        return Ok(LineWindow { start, end: None });
    }
    let end = parse_line_no(b).ok_or_else(bad)?;
    if end < start {
        return Err(bad());
    }
    Ok(LineWindow { start, end: Some(end) })
}

/// Apply a window to the item text: `(content, start, end, total)`. A start
/// past the last line is a 400 naming the line count; an end past it clamps
/// (`sed -n` semantics), and the response reports the served end.
pub fn window_text(text: &str, w: &LineWindow) -> Result<(String, usize, usize, usize), String> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if w.start > total {
        return Err(format!("lines start {} is past the end: the item has {total} line(s)", w.start));
    }
    let end = w.end.unwrap_or(total).min(total);
    Ok((lines[w.start - 1..end].join("\n"), w.start, end, total))
}

fn item_json(id: i64, source_id: i64, title: String, path: String, text: String, sensitive_override: Option<bool>, pdf_quality: Option<String>) -> Value {
    // `content` aliases `text`: every content-carrying payload exposes
    // `content` (search hits, citations), so item reads do too (1.0.24
    // retest #8). `text` stays the canonical key for backward compat.
    // `pdf_quality` (R36.10) is null for every non-PDF and clean PDF.
    // `sensitive_override` (R35.4) mirrors the column: true or null.
    json!({"id": id, "source_id": source_id, "title": title, "path": path, "text": text.clone(), "content": text, "sensitive_override": sensitive_override, "pdf_quality": pdf_quality})
}

/// R36.10 item-level PDF quality flag: `suspect` when the stored text trips
/// the spacing detector right now (current state wins — a repair that left
/// damage still reads damaged), `repaired` when a split-word repair event
/// matches this file and the text is clean, else null. Derived at read
/// time — no migration, retroactive to rows synced before this change.
fn pdf_quality_for(
    conn: &rusqlite::Connection,
    source_id: i64,
    title: &str,
    external_id: &str,
    text: &str,
) -> Option<String> {
    if !item_is_pdf(title, external_id) {
        return None;
    }
    if crate::extract::pdf_spacing_suspect(text) {
        return Some("suspect".to_string());
    }
    if repair_hit(&folder_roots(conn), &repair_details_by_source(conn, &[source_id]), source_id, external_id) {
        return Some("repaired".to_string());
    }
    None
}

fn item_is_pdf(title: &str, external_id: &str) -> bool {
    crate::extract::is_pdf_filename(title) || crate::extract::is_pdf_filename(external_id)
}

/// Repair-event details (`'pdf split-word repair'` rows) grouped by source —
/// one query for the detail view or a whole listing page.
fn repair_details_by_source(
    conn: &rusqlite::Connection,
    source_ids: &[i64],
) -> std::collections::HashMap<i64, Vec<String>> {
    let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    if source_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT source_id, detail FROM system_events WHERE message = 'pdf split-word repair' AND source_id IN ({})",
        crate::common::placeholders(source_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(source_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
    }) {
        for (sid, detail) in rows.flatten() {
            out.entry(sid).or_default().push(detail);
        }
    }
    out
}

/// True when a repair event names this item's file: the event label is the
/// absolute extraction path, matched exactly against the owning folder root
/// joined with the external id (slash-normalized, so Windows `\` labels
/// match; a moved root degrades to no match, never to a wrong file's flag).
/// `root` must already be canonicalized (see `repair_hit`) — the label is.
fn repair_matches(detail: &str, root: &str, external_id: &str) -> bool {
    let label = match detail.split_once(": split-word suspects ") {
        Some((label, _)) => label,
        None => return false,
    };
    slash_norm(label) == slash_norm(&format!("{root}/{external_id}"))
}

fn repair_hit(
    roots: &std::collections::HashMap<i64, String>,
    events: &std::collections::HashMap<i64, Vec<String>>,
    source_id: i64,
    external_id: &str,
) -> bool {
    let root = match roots.get(&source_id) {
        Some(r) => r,
        None => return false,
    };
    let details = match events.get(&source_id) {
        Some(d) => d,
        None => return false,
    };
    // The connector canonicalizes its root (`folder.rs`), so event labels
    // are canonical while the stored config may hold a symlinked form
    // (`/tmp` → `/private/tmp` on macOS). Canonicalize to compare.
    let canon = std::fs::canonicalize(root).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| root.clone());
    details.iter().any(|d| repair_matches(d, &canon, external_id))
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Slash-normalize a filesystem path: `\` → `/`, drive letter lowercased.
/// Trailing slashes are preserved — `dir/` and `dir` differ as LIKE prefixes.
pub(crate) fn slash_norm(p: &str) -> String {
    let mut n = p.replace('\\', "/");
    if n.len() >= 2 && n.as_bytes()[1] == b':' && n.as_bytes()[0].is_ascii_alphabetic() {
        n.replace_range(0..1, &n[0..1].to_lowercase());
    }
    n
}

/// True for absolute filesystem prefixes: `/...`, `C:/...`, UNC `\\...`
/// (which slash-normalizes to `//...`). Drive-relative `C:x` reads as relative.
pub(crate) fn is_absolute_prefix(p: &str) -> bool {
    let n = slash_norm(p);
    n.starts_with('/') || (n.len() > 2 && n.as_bytes()[1] == b':' && n.as_bytes()[2] == b'/')
}

/// Folder source roots from stored configs: source_id → normalized root
/// (no trailing slash; `/` survives as itself). Non-folder sources and
/// configs without a path are skipped — absolute prefixes only resolve
/// through folder roots.
pub(crate) fn folder_roots(conn: &rusqlite::Connection) -> std::collections::HashMap<i64, String> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = match conn.prepare("SELECT id, config FROM sources WHERE connector = 'folder'") {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows: Vec<(i64, String)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(m) => m.filter_map(|r| r.ok()).collect(),
        Err(_) => return out,
    };
    for (sid, cfg) in rows {
        let path = serde_json::from_str::<serde_json::Value>(&cfg)
            .ok()
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        if path.trim().is_empty() {
            continue;
        }
        let trimmed = slash_norm(path.trim()).trim_end_matches('/').to_string();
        out.insert(sid, if trimmed.is_empty() { "/".to_string() } else { trimmed });
    }
    out
}

/// Resolve an absolute prefix through folder roots: one entry per matching
/// source, with the relative remainder (`None` = the whole source — the
/// prefix is the root itself or an ancestor of it). `source_id` scopes the
/// candidates. Trailing slashes survive into the remainder so `dir/` keeps
/// its LIKE precision.
pub(crate) fn absolute_remainders(
    roots: &std::collections::HashMap<i64, String>,
    prefix: &str,
    source_id: Option<i64>,
) -> Vec<(i64, Option<String>)> {
    let raw = slash_norm(prefix);
    let trimmed = raw.trim_end_matches('/');
    let p = if trimmed.is_empty() { "/" } else { trimmed };
    let mut out = Vec::new();
    for (sid, root) in roots {
        if source_id.is_some_and(|want| *sid != want) {
            continue;
        }
        if p == "/" || root == p || root.starts_with(&format!("{p}/")) {
            out.push((*sid, None));
            continue;
        }
        if let Some(rel) = raw.strip_prefix(&format!("{root}/")) {
            out.push((*sid, Some(rel.to_string())));
        }
    }
    out.sort_by_key(|(sid, _)| *sid);
    out
}

/// One item against a listing/search prefix. Relative prefixes match the
/// stored external id start (slash-normalized); absolute prefixes resolve
/// through `absolute_remainders`, so this mirrors the listing query exactly
/// (LIKE-prefix == case-folded `starts_with`).
pub(crate) fn item_under_prefix(
    external_id: &str,
    source_id: Option<i64>,
    roots: &std::collections::HashMap<i64, String>,
    prefix: &str,
) -> bool {
    if !is_absolute_prefix(prefix) {
        return external_id.replace('\\', "/").to_lowercase().starts_with(&slash_norm(prefix).to_lowercase());
    }
    let sid = match source_id {
        Some(s) => s,
        None => return false,
    };
    for (rsid, rel) in absolute_remainders(roots, prefix, Some(sid)) {
        if rsid != sid {
            continue;
        }
        return match rel {
            None => true,
            Some(r) => external_id.replace('\\', "/").to_lowercase().starts_with(&r.to_lowercase()),
        };
    }
    false
}

/// Metadata listing over ingested items — the discovery primitive that ends
/// id-scanning loops (1.0.24 retest #7: ~150 `GET /items/:id` probing ids
/// 200–300 to find `reportdata/` files). `path_prefix` matches the item path
/// start, slash-normalized so `reportdata/` also finds Windows `\` rows.
/// Relative prefixes match the stored external id; absolute prefixes resolve
/// through the owning folder source's root (R36.6).
/// Metadata only (title/path, no bytes), so no caller gate — but the
/// `public_only` scope still drops effective-Sensitive items (R35.5), like
/// every other public scope.
/// Listing filters (R36.15 adds `public_only`).
struct ListFilter<'a> {
    path_prefix: Option<&'a str>,
    source_id: Option<i64>,
    public_only: bool,
}

/// R35.5: the public scope is public sources minus effective-Sensitive
/// items (the per-document override drops even on a public source),
/// whatever the caller or provider.
const PUBLIC_ONLY_COND: &str =
    "source_id IN (SELECT id FROM sources WHERE label = 'public') AND COALESCE(sensitive_override, 0) != 1";

fn base_conds(public_only: bool) -> Vec<String> {
    if public_only { vec![PUBLIC_ONLY_COND.to_string()] } else { Vec::new() }
}

fn list_items(conn: &rusqlite::Connection, path_prefix: Option<&str>, source_id: Option<i64>, limit: i64, offset: i64) -> (Vec<Value>, i64) {
    list_items_filtered(conn, &ListFilter { path_prefix, source_id, public_only: false }, limit, offset)
}

fn list_items_filtered(conn: &rusqlite::Connection, f: &ListFilter, limit: i64, offset: i64) -> (Vec<Value>, i64) {
    let (path_prefix, source_id) = (f.path_prefix, f.source_id);
    if let Some(p) = path_prefix {
        if is_absolute_prefix(p) {
            return list_items_absolute(conn, p, f, limit, offset);
        }
    }
    let mut conds: Vec<String> = base_conds(f.public_only);
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(p) = path_prefix {
        let norm = p.replace('\\', "/");
        conds.push("REPLACE(external_id, '\\', '/') LIKE ? ESCAPE '\\'".to_string());
        params.push(rusqlite::types::Value::Text(format!("{}%", escape_like(&norm))));
    }
    if let Some(sid) = source_id {
        conds.push("source_id = ?".to_string());
        params.push(rusqlite::types::Value::Integer(sid));
    }
    run_list_query(conn, conds, params, limit, offset)
}

/// Absolute-prefix listing: the prefix resolves to per-source relative
/// remainders (`absolute_remainders`), each a `source_id` + LIKE arm. A
/// prefix covering no root matches nothing — never everything.
fn list_items_absolute(
    conn: &rusqlite::Connection,
    prefix: &str,
    f: &ListFilter,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
    let roots = folder_roots(conn);
    let mut ors: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let mut conds = base_conds(f.public_only);
    for (sid, rel) in absolute_remainders(&roots, prefix, f.source_id) {
        match rel {
            None => {
                ors.push("source_id = ?".to_string());
                params.push(rusqlite::types::Value::Integer(sid));
            }
            Some(r) => {
                ors.push("(source_id = ? AND REPLACE(external_id, '\\', '/') LIKE ? ESCAPE '\\')".to_string());
                params.push(rusqlite::types::Value::Integer(sid));
                params.push(rusqlite::types::Value::Text(format!("{}%", escape_like(&r))));
            }
        }
    }
    if ors.is_empty() {
        conds.push("1 = 0".to_string());
        return run_list_query(conn, conds, params, limit, offset);
    }
    conds.push(format!("({})", ors.join(" OR ")));
    run_list_query(conn, conds, params, limit, offset)
}

fn run_list_query(
    conn: &rusqlite::Connection,
    conds: Vec<String>,
    mut params: Vec<rusqlite::types::Value>,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
    let filter = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ingested_items {}", filter),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )
        .unwrap_or(0);
    let sql = format!(
        "SELECT id, source_id, title, external_id, sensitive_override, is_glossary FROM ingested_items {} ORDER BY external_id, id LIMIT ? OFFSET ?",
        filter
    );
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return (vec![], total),
    };
    let tuples: Vec<ListRow> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?.is_some_and(|v| v == 1).then_some(true),
                r.get::<_, i64>(5)? != 0,
            ))
        })
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    // R36.10: listing rows carry the PDF flag; texts load in one batched
    // query over the page's PDF rows only (metadata listing stays byte-free
    // for everything else).
    let flags = pdf_flags_for_page(conn, &tuples);
    let rows = tuples
        .into_iter()
        .map(|(id, sid, title, path, sensitive_override, is_glossary)| {
            json!({
                "id": id,
                "source_id": sid,
                "title": title,
                "path": path,
                "sensitive_override": sensitive_override,
                "is_glossary": is_glossary,
                "pdf_quality": flags.get(&id).cloned(),
            })
        })
        .collect();
    (rows, total)
}

/// Listing-page flags in three batched queries (texts, roots, repair
/// events) over the page's PDF rows only. Same precedence as the detail
/// view: current-state `suspect` wins over a historical `repaired`.
/// Listing row: id, source_id, title, path, sensitive_override, is_glossary.
type ListRow = (i64, i64, String, String, Option<bool>, bool);

fn pdf_flags_for_page(
    conn: &rusqlite::Connection,
    rows: &[ListRow],
) -> std::collections::HashMap<i64, String> {
    let mut out: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    let pdfs: Vec<(i64, i64, String)> = rows
        .iter()
        .filter(|(_, _, title, path, _, _)| item_is_pdf(title, path))
        .map(|(id, sid, _, path, _, _)| (*id, *sid, path.clone()))
        .collect();
    if pdfs.is_empty() {
        return out;
    }
    let ids: Vec<i64> = pdfs.iter().map(|(id, _, _)| *id).collect();
    let texts = pdf_texts_for(conn, &ids);
    let mut sids: Vec<i64> = pdfs.iter().map(|(_, sid, _)| *sid).collect();
    sids.sort();
    sids.dedup();
    let roots = folder_roots(conn);
    let events = repair_details_by_source(conn, &sids);
    for (id, sid, path) in &pdfs {
        if texts.get(id).is_some_and(|t| crate::extract::pdf_spacing_suspect(t)) {
            out.insert(*id, "suspect".to_string());
        } else if repair_hit(&roots, &events, *sid, path) {
            out.insert(*id, "repaired".to_string());
        }
    }
    out
}

fn pdf_texts_for(conn: &rusqlite::Connection, ids: &[i64]) -> std::collections::HashMap<i64, String> {
    let mut out: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT id, text FROM ingested_items WHERE id IN ({})",
        crate::common::placeholders(ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
    }) {
        for (id, text) in rows.flatten() {
            out.insert(id, text);
        }
    }
    out
}

pub async fn list_handler(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    validate_path_prefix(q.path_prefix.as_deref()).map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"detail": e}))))?;
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let limit = q.limit.unwrap_or(100).clamp(1, 1000);
        let offset = q.offset.unwrap_or(0).max(0);
        let filter = ListFilter { path_prefix: q.path_prefix.as_deref(), source_id: q.source_id, public_only: q.public_only.unwrap_or(false) };
        let (items, total) = list_items_filtered(&conn, &filter, limit, offset);
        json!({"items": items, "total": total})
    })
    .await
    .unwrap();
    Ok(Json(result))
}

fn fetch_item(conn: &rusqlite::Connection, item_id: i64) -> Option<Value> {
    let row: Option<(i64, i64, String, String, String, bool)> = conn
        .query_row(
            "SELECT id, source_id, title, external_id, text, sensitive_override FROM ingested_items WHERE id = ?1",
            [item_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get::<_, Option<i64>>(5).ok().flatten().is_some_and(|v| v == 1),
                ))
            },
        )
        .ok();
    let is_glossary: bool = conn
        .query_row("SELECT is_glossary FROM ingested_items WHERE id = ?1", [item_id], |r| r.get(0))
        .unwrap_or(false);
    row.map(|(id, sid, title, path, text, marked)| {
        let pdf_quality = pdf_quality_for(conn, sid, &title, &path, &text);
        let mut v = item_json(id, sid, title, path, text, marked.then_some(true), pdf_quality);
        v["is_glossary"] = json!(is_glossary);
        // R46.9: the item's images (embeds + its own file), bytes via /media.
        v["assets"] = json!(crate::assets::refs_for_item(conn, id));
        v
    })
}

/// R45.13: mark/unmark a file as a glossary and (re)parse it now — no
/// re-sync needed. Unmarking drops the file's glossary rows.
fn apply_glossary_mark(conn: &rusqlite::Connection, item_id: i64, mark: bool) -> Result<(), String> {
    let (source_id, text): (i64, String) = conn
        .query_row("SELECT source_id, COALESCE(text, '') FROM ingested_items WHERE id = ?1", [item_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE ingested_items SET is_glossary = ?1, updated_at = datetime('now') WHERE id = ?2",
        rusqlite::params![i64::from(mark), item_id],
    )
    .map_err(|e| e.to_string())?;
    let pairs = if mark { crate::vocab::parse_glossary_file(&text) } else { Vec::new() };
    crate::vocab::store_glossary_file(conn, item_id, source_id, &pairs).map(|_| ())
}

/// R35.5: one item read under the `public_only` scope, with an optional
/// line window. The whole-file and window forms share one byte gate on the
/// effective Sensitive rule (absorbs R36.19); the UI caller is exempt.
fn read_item(conn: &rusqlite::Connection, item_id: i64, q: &GetQuery, gate: &crate::read_gate::Gate) -> Result<Value, ReadRefusal> {
    const NOT_FOUND: &str = "Item not found";
    let (label, marked) = crate::read_gate::item_sensitivity(conn, item_id).ok_or(ReadRefusal::NotFound(NOT_FOUND))?;
    crate::read_gate::check_public(&label, marked, q.public_only.unwrap_or(false), NOT_FOUND)?;
    gate.check_bytes(&label, marked)?;
    let window = match q.lines.as_deref() {
        Some(raw) => Some(parse_lines(raw).map_err(ReadRefusal::BadRequest)?),
        None => None,
    };
    let mut v = fetch_item(conn, item_id).ok_or(ReadRefusal::NotFound(NOT_FOUND))?;
    let Some(w) = window else { return Ok(v) };
    let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let (content, start, end, total) = window_text(&text, &w).map_err(ReadRefusal::BadRequest)?;
    v["text"] = json!(content.clone());
    v["content"] = json!(content);
    v["lines"] = json!({"start": start, "end": end, "total": total});
    Ok(v)
}

pub async fn get_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
    Query(q): Query<GetQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let gate = crate::read_gate::Gate::from_request(&headers, &state.settings);
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        read_item(&conn, item_id, &q, &gate)
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(r) => Err(crate::read_gate::into_api_error(&state.data_dir, r).await),
    }
}

pub async fn patch_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
    Json(payload): Json<UpdateItemPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "ingested_items", item_id, "Item not found")?;
        if payload.is_pii {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"detail": "is_pii is removed; use sensitive_override (true to mark Sensitive, null to clear)"})),
            ));
        }
        if let Some(write) = payload.sensitive_override {
            match write {
                Some(true) => {
                    let _ = conn.execute("UPDATE ingested_items SET sensitive_override = 1, updated_at = datetime('now') WHERE id = ?1", [item_id]);
                }
                None => {
                    let _ = conn.execute("UPDATE ingested_items SET sensitive_override = NULL, updated_at = datetime('now') WHERE id = ?1", [item_id]);
                }
                Some(false) => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": "sensitive_override false is reserved; send true to mark Sensitive or null to clear"})),
                    ));
                }
            }
        }
        if let Some(mark) = payload.is_glossary {
            apply_glossary_mark(&conn, item_id, mark)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e}))))?;
        }
        match fetch_item(&conn, item_id) {
            Some(v) => Ok(v),
            None => Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Item not found"})))),
        }
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn seed_db() -> (tempfile::TempDir, rusqlite::Connection, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'cfg/routing-config.yml','routing-config.yml','ACME_FEE: x','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        (dir, conn, iid)
    }

    fn app_state(dir: &tempfile::TempDir) -> AppState {
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        AppState {
            data_dir: dir.path().to_string_lossy().to_string(),
            settings,
            jobs: crate::jobs::JobManager::new(),
            csrf_token: String::new(),
            embedder,
        }
    }

    #[tokio::test]
    async fn glossary_mark_parses_now_and_unmark_clears() {
        // R45.13: marking a file makes its rows active vocabulary at once;
        // unmarking removes them. No proposals, no re-sync.
        let (dir, conn, _) = seed_db();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (1,'terms.csv','terms.csv','term,expansion\nKYC,know your customer\nDD,direct debit\n','h2')",
            [],
        )
        .unwrap();
        let gid = conn.last_insert_rowid();
        let st = app_state(&dir);
        let mark = UpdateItemPayload { sensitive_override: None, is_pii: false, is_glossary: Some(true) };
        let Json(v) = patch_handler(State(st.clone()), Path(gid), Json(mark)).await.unwrap();
        assert_eq!(v["is_glossary"], true);
        let n = |c: &rusqlite::Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM vocabulary WHERE item_id=?1 AND origin='glossary'", [gid], |r| r.get(0)).unwrap()
        };
        assert_eq!(n(&conn), 2, "rows active right away");
        let terms = vec!["kyc".to_string()];
        assert_eq!(crate::vocab::vocab_expansion_terms(&conn, &terms, None), ["know your customer"]);
        let clear = UpdateItemPayload { sensitive_override: None, is_pii: false, is_glossary: Some(false) };
        let Json(v) = patch_handler(State(st), Path(gid), Json(clear)).await.unwrap();
        assert_eq!(v["is_glossary"], false);
        assert_eq!(n(&conn), 0, "unmark clears the rows");
    }

    #[test]
    fn fetch_item_returns_full_text_with_path() {
        let (_dir, conn, iid) = seed_db();
        let v = fetch_item(&conn, iid).expect("seeded item must fetch");
        assert_eq!(v.get("path").and_then(|x| x.as_str()), Some("cfg/routing-config.yml"));
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("ACME_FEE: x"));
        assert_eq!(v.get("content").and_then(|x| x.as_str()), Some("ACME_FEE: x"), "`content` aliases `text` (hits/citations parity)");
        assert!(v.get("sensitive_override").is_some_and(|x| x.is_null()), "new items default unmarked (inherit): {v}");
        assert!(fetch_item(&conn, iid + 99).is_none(), "missing item reads as None (404)");
        assert!(v.get("pdf_quality").is_some_and(|q| q.is_null()), "non-PDF rows carry a null flag");
    }

    /// Shredded spacing that trips the detector (same join shapes as the
    /// extractor's shredded fixture, enough suspects to clear the floor).
    fn shredded_text() -> String {
        "Cancellati ons and Correcti ons with noti fi cati ons for processi ng of subscripti ons and installati on guides".to_string()
    }

    fn seed_pdf_db() -> (tempfile::TempDir, rusqlite::Connection, i64, i64, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute(
            "INSERT INTO sources (name, connector, config) VALUES ('s','folder','{\"path\": \"/docs\"}')",
            [],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'bad.pdf','bad.pdf',?2,'h')",
            rusqlite::params![sid, shredded_text()],
        )
        .unwrap();
        let bad = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'good.pdf','good.pdf','Cancellations and notifications for processing.','h')",
            [sid],
        )
        .unwrap();
        let good = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'n.md','n.md','plain words','h')",
            [sid],
        )
        .unwrap();
        let md = conn.last_insert_rowid();
        (dir, conn, bad, good, md)
    }

    #[test]
    fn fetch_item_pdf_quality_suspect_and_null() {
        let (_dir, conn, bad, good, md) = seed_pdf_db();
        let v = fetch_item(&conn, bad).expect("seeded pdf must fetch");
        assert_eq!(v.get("pdf_quality").and_then(|q| q.as_str()), Some("suspect"), "shredded stored text flags");
        let v = fetch_item(&conn, good).expect("seeded pdf must fetch");
        assert!(v.get("pdf_quality").is_some_and(|q| q.is_null()), "clean PDF without a repair event reads null");
        let v = fetch_item(&conn, md).expect("seeded md must fetch");
        assert!(v.get("pdf_quality").is_some_and(|q| q.is_null()), "non-PDF never scores");
    }

    #[test]
    fn fetch_item_pdf_quality_repaired_needs_matching_event() {
        let (_dir, conn, bad, good, _md) = seed_pdf_db();
        let sid: i64 = conn.query_row("SELECT source_id FROM ingested_items WHERE id=?1", [good], |r| r.get(0)).unwrap();
        // A repair event for another file must not flag this one.
        conn.execute(
            "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'pdf split-word repair','/docs/other.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)')",
            [sid],
        )
        .unwrap();
        let v = fetch_item(&conn, good).unwrap();
        assert!(v.get("pdf_quality").is_some_and(|q| q.is_null()), "another file's event must not match");
        // The matching event flips the clean PDF to repaired …
        conn.execute(
            "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'pdf split-word repair','/docs/good.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)')",
            [sid],
        )
        .unwrap();
        let v = fetch_item(&conn, good).unwrap();
        assert_eq!(v.get("pdf_quality").and_then(|q| q.as_str()), Some("repaired"));
        // … but current-state damage wins over history.
        conn.execute(
            "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'pdf split-word repair','/docs/bad.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)')",
            [sid],
        )
        .unwrap();
        let v = fetch_item(&conn, bad).unwrap();
        assert_eq!(v.get("pdf_quality").and_then(|q| q.as_str()), Some("suspect"), "damage now beats repair then");
    }

    #[test]
    fn repair_label_matching_is_exact_and_slash_normalized() {
        assert!(repair_matches("/docs/r.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)", "/docs", "r.pdf"));
        assert!(repair_matches("C:\\docs\\r.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)", "C:\\docs", "r.pdf"));
        assert!(!repair_matches("/docs/r.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)", "/docs", "other.pdf"));
        assert!(!repair_matches("/docs/sub/r.pdf: split-word suspects 9/100 tokens, repair pass 1 (repaired)", "/docs", "r.pdf"), "a longer path must not suffix-match");
        assert!(!repair_matches("not an event detail", "/docs", "r.pdf"));
    }

    /// Live-probe regression: the connector canonicalizes its root, so the
    /// event label holds the real path while the config may hold a symlinked
    /// one (`/tmp` → `/private/tmp`). The flag must still match.
    #[cfg(unix)]
    #[test]
    fn fetch_item_pdf_quality_matches_symlinked_root() {
        let target = tempdir().unwrap();
        let linkdir = tempdir().unwrap();
        let link = linkdir.path().join("docslink");
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        let cfg = serde_json::json!({"path": link.to_string_lossy()}).to_string();
        conn.execute("INSERT INTO sources (name, connector, config) VALUES ('s','folder',?1)", [cfg]).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'r.pdf','r.pdf','Cancellations and notifications.','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        let canon = std::fs::canonicalize(target.path()).unwrap();
        let detail = format!("{}: split-word suspects 9/25 tokens, repair pass 0 (repaired)", canon.join("r.pdf").display());
        conn.execute(
            "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'pdf split-word repair',?2)",
            rusqlite::params![sid, detail],
        )
        .unwrap();
        let v = fetch_item(&conn, iid).unwrap();
        assert_eq!(v.get("pdf_quality").and_then(|q| q.as_str()), Some("repaired"), "symlinked config root must match the canonical label");
    }

    #[test]
    fn list_items_carries_pdf_quality() {
        let (_dir, conn, bad, good, md) = seed_pdf_db();
        let (items, total) = list_items(&conn, None, None, 100, 0);
        assert_eq!(total, 3);
        let flag = |id: i64| {
            items
                .iter()
                .find(|v| v.get("id").and_then(|x| x.as_i64()) == Some(id))
                .and_then(|v| v.get("pdf_quality").cloned())
        };
        assert_eq!(flag(bad).and_then(|q| q.as_str().map(|s| s.to_string())), Some("suspect".to_string()));
        assert!(flag(good).is_some_and(|q| q.is_null()));
        assert!(flag(md).is_some_and(|q| q.is_null()));
        assert!(items.iter().all(|v| v.get("text").is_none() && v.get("content").is_none()), "listing stays metadata-only");
    }

    fn seed_list_db() -> (tempfile::TempDir, rusqlite::Connection, i64, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('a','folder')", []).unwrap();
        let sa = conn.last_insert_rowid();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('b','folder')", []).unwrap();
        let sb = conn.last_insert_rowid();
        let paths = [
            (sa, "reportdata/a.yml"),
            (sa, "reportdata/b.yml"),
            (sa, "reportdata\\win.yml"),
            (sb, "reportdata/other.yml"),
            (sa, "cfg/c.yml"),
            (sa, "report%data/x.yml"),
            (sa, "report_data/y.yml"),
        ];
        for (sid, p) in paths {
            conn.execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,?2,?2,'t','h')",
                rusqlite::params![sid, p],
            )
            .unwrap();
        }
        (dir, conn, sa, sb)
    }

    fn list_paths(items: &[Value]) -> Vec<String> {
        items.iter().filter_map(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string())).collect()
    }

    #[test]
    fn list_items_filters_by_path_prefix() {
        let (_dir, conn, sa, sb) = seed_list_db();
        // Plain prefix: forward and backslash rows alike, ordered by path.
        let (items, total) = list_items(&conn, Some("reportdata/"), None, 100, 0);
        assert_eq!(total, 4);
        assert_eq!(list_paths(&items), vec!["reportdata/a.yml", "reportdata/b.yml", "reportdata/other.yml", "reportdata\\win.yml"]);
        assert!(items.iter().all(|v| v.get("text").is_none() && v.get("content").is_none()), "listing is metadata only");
        // LIKE specials in the prefix match literally, never as wildcards.
        let (items, total) = list_items(&conn, Some("report_"), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report_data/y.yml"]);
        let (items, total) = list_items(&conn, Some("report%"), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report%data/x.yml"]);
        // Source scope composes with the prefix.
        let (items, total) = list_items(&conn, Some("reportdata/"), Some(sb), 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["reportdata/other.yml"]);
        let (_items, total) = list_items(&conn, None, Some(sa), 100, 0);
        assert_eq!(total, 6);
        // Pagination walks the full set; total ignores the window.
        let (p1, total) = list_items(&conn, None, None, 3, 0);
        let (p2, _) = list_items(&conn, None, None, 3, 3);
        let (p3, _) = list_items(&conn, None, None, 3, 6);
        assert_eq!(total, 7);
        assert_eq!(p1.len() + p2.len() + p3.len(), 7);
        let mut all = list_paths(&p1);
        all.extend(list_paths(&p2));
        all.extend(list_paths(&p3));
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 7, "pages must cover distinct items");
    }

    #[tokio::test]
    async fn list_handler_serves_prefix_query() {
        let (dir, conn, _sa, _sb) = seed_list_db();
        drop(conn);
        let state = app_state(&dir);
        let q = ListQuery { path_prefix: Some("reportdata/".to_string()), source_id: None, limit: Some(2), offset: Some(1), public_only: None };
        let Json(v) = list_handler(State(state), Query(q)).await.expect("well-formed prefix lists");
        assert_eq!(v.get("total").and_then(|t| t.as_i64()), Some(4));
        let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
        assert_eq!(list_paths(&items), vec!["reportdata/b.yml", "reportdata/other.yml"]);
    }

    #[test]
    fn update_payload_parses_tri_state_and_removed_key() {
        let absent: UpdateItemPayload = serde_json::from_value(json!({})).unwrap();
        assert_eq!(absent.sensitive_override, None);
        assert!(!absent.is_pii);
        let clear: UpdateItemPayload = serde_json::from_value(json!({"sensitive_override": Value::Null})).unwrap();
        assert_eq!(clear.sensitive_override, Some(None), "present null clears, not no-ops");
        let set: UpdateItemPayload = serde_json::from_value(json!({"sensitive_override": true})).unwrap();
        assert_eq!(set.sensitive_override, Some(Some(true)));
        let legacy_null: UpdateItemPayload = serde_json::from_value(json!({"is_pii": Value::Null})).unwrap();
        assert!(legacy_null.is_pii, "even explicit null trips the removed key");
    }

    #[tokio::test]
    async fn patch_item_sensitive_override_round_trip() {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'d/f.md','f.md','body','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        drop(conn);
        let state = app_state(&dir);
        let mark = UpdateItemPayload { sensitive_override: Some(Some(true)), is_pii: false, is_glossary: None };
        let Json(v) = patch_handler(State(state.clone()), Path(iid), Json(mark))
            .await
            .expect("mark must succeed");
        assert_eq!(v.get("sensitive_override").and_then(|x| x.as_bool()), Some(true));
        // R35.5: the marked item is read back as the UI (an agent caller
        // would be gated — the persistence assert is about the mark).
        let mut ui = HeaderMap::new();
        ui.insert(crate::read_gate::UI_CLIENT_HEADER, "ui".parse().unwrap());
        let Json(got) = get_handler(State(state.clone()), Path(iid), Query(GetQuery { lines: None, public_only: None }), ui).await.expect("get must succeed");
        assert_eq!(got.get("sensitive_override").and_then(|x| x.as_bool()), Some(true), "mark persists");
        let clear = UpdateItemPayload { sensitive_override: Some(None), is_pii: false, is_glossary: None };
        let Json(v2) = patch_handler(State(state.clone()), Path(iid), Json(clear))
            .await
            .expect("clear must succeed");
        assert!(v2.get("sensitive_override").is_some_and(|x| x.is_null()), "clear restores inherit: {v2}");
        // Reserved false and the removed key are 400s naming the contract.
        let reserved = UpdateItemPayload { sensitive_override: Some(Some(false)), is_pii: false, is_glossary: None };
        match patch_handler(State(state.clone()), Path(iid), Json(reserved)).await {
            Err((s, Json(e))) => {
                assert_eq!(s, StatusCode::BAD_REQUEST);
                assert!(e["detail"].as_str().unwrap_or_default().contains("reserved"), "{e}");
            }
            Ok(_) => panic!("false must be a 400"),
        }
        let legacy: UpdateItemPayload = serde_json::from_value(json!({"is_pii": true})).expect("removed key still parses");
        match patch_handler(State(state.clone()), Path(iid), Json(legacy)).await {
            Err((s, Json(e))) => {
                assert_eq!(s, StatusCode::BAD_REQUEST);
                assert!(e["detail"].as_str().unwrap_or_default().contains("sensitive_override"), "{e}");
            }
            Ok(_) => panic!("is_pii must be a 400"),
        }
        // Missing item → 404, not a silent no-op.
        let miss = UpdateItemPayload { sensitive_override: Some(Some(true)), is_pii: false, is_glossary: None };
        match patch_handler(State(state), Path(iid + 99), Json(miss)).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("patch on missing item must 404"),
        }
    }

    fn seed_abs_db() -> (tempfile::TempDir, tempfile::TempDir, rusqlite::Connection, i64, i64, String) {
        let dbdir = tempdir().unwrap();
        let corpus = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dbdir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        let root = corpus.path().to_string_lossy().to_string();
        let cfg = serde_json::json!({"path": root}).to_string();
        conn.execute("INSERT INTO sources (name, connector, config) VALUES ('docs','folder',?1)", [cfg]).unwrap();
        let sf: i64 = conn.query_row("SELECT id FROM sources WHERE name='docs'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('jira','jira')", []).unwrap();
        let sj: i64 = conn.query_row("SELECT id FROM sources WHERE name='jira'", [], |r| r.get(0)).unwrap();
        for (sid, p) in [
            (sf, "reportdata/a.yml"),
            (sf, "reportdata\\win.yml"),
            (sf, "cfg/c.yml"),
            (sf, "report%data/x.yml"),
            (sj, "PROJ-1"),
        ] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,?2,?2,'t','h')",
                rusqlite::params![sid, p],
            )
            .unwrap();
        }
        (dbdir, corpus, conn, sf, sj, root)
    }

    #[test]
    fn list_items_accepts_absolute_prefixes() {
        let (_dbdir, _corpus, conn, sf, sj, root) = seed_abs_db();
        // The root itself lists the whole folder source (never the jira row).
        let (items, total) = list_items(&conn, Some(&root), None, 100, 0);
        assert_eq!(total, 4);
        assert!(list_paths(&items).iter().all(|p| *p != "PROJ-1"));
        // A subdirectory narrows to it, backslash rows included.
        let sub = format!("{root}/reportdata");
        let (items, total) = list_items(&conn, Some(&sub), None, 100, 0);
        assert_eq!(total, 2);
        assert_eq!(list_paths(&items), vec!["reportdata/a.yml", "reportdata\\win.yml"]);
        // Trailing slash + backslash separators spell the same prefix.
        let (items, total) = list_items(&conn, Some(&format!("{sub}/")), None, 100, 0);
        assert_eq!(total, 2);
        assert_eq!(list_paths(&items).len(), 2);
        let (items, total) = list_items(&conn, Some(&sub.replace('/', "\\")), None, 100, 0);
        assert_eq!(total, 2, "backslash absolute prefix must match: {}", sub);
        assert_eq!(list_paths(&items).len(), 2);
        // LIKE specials in the absolute tail match literally.
        let (items, total) = list_items(&conn, Some(&format!("{root}/report%")), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report%data/x.yml"]);
        // source_id composes: another source's root prefix finds nothing here.
        let (_items, total) = list_items(&conn, Some(&sub), Some(sj), 100, 0);
        assert_eq!(total, 0, "absolute prefix on a non-folder source matches nothing");
        let (_items, total) = list_items(&conn, Some(&sub), Some(sf), 100, 0);
        assert_eq!(total, 2);
        // An ancestor of the root covers the whole source; an unrelated
        // absolute path matches nothing — never everything.
        let parent = std::path::Path::new(&root).parent().unwrap().to_string_lossy().to_string();
        let (_items, total) = list_items(&conn, Some(&parent), Some(sf), 100, 0);
        assert_eq!(total, 4);
        let (_items, total) = list_items(&conn, Some("/no/such/root/anywhere"), None, 100, 0);
        assert_eq!(total, 0);
        let (items, total) = list_items(&conn, Some("/no/such/root/anywhere"), None, 1, 0);
        assert_eq!(total, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn absolute_prefix_helpers() {
        assert!(is_absolute_prefix("/a/b"));
        assert!(is_absolute_prefix("C:/corpus"));
        assert!(is_absolute_prefix("c:\\corpus\\sub"));
        assert!(is_absolute_prefix("\\\\srv\\share"));
        assert!(!is_absolute_prefix("reportdata/"));
        assert!(!is_absolute_prefix("C:relative"));
        assert!(!is_absolute_prefix(""));
        assert_eq!(slash_norm("C:\\A\\b"), "c:/A/b");
        let mut roots = std::collections::HashMap::new();
        roots.insert(1, "/data/corpus".to_string());
        roots.insert(2, "/data/other".to_string());
        // Root itself + ancestor cover the whole source; segment boundaries hold.
        assert_eq!(absolute_remainders(&roots, "/data/corpus", None), vec![(1, None)]);
        assert_eq!(absolute_remainders(&roots, "/data", None), vec![(1, None), (2, None)]);
        assert!(absolute_remainders(&roots, "/data/corp", None).is_empty(), "no segment-boundary bleed");
        assert_eq!(
            absolute_remainders(&roots, "/data/corpus/reportdata/", None),
            vec![(1, Some("reportdata/".to_string()))]
        );
        assert!(absolute_remainders(&roots, "/data/corpus/x", Some(2)).is_empty());
        // item_under_prefix mirrors the listing: relative, absolute, ancestor,
        // boundary, case-folded LIKE parity, and missing-root reads.
        assert!(item_under_prefix("reportdata/a.yml", Some(1), &roots, "reportdata/"));
        assert!(!item_under_prefix("cfg/c.yml", Some(1), &roots, "reportdata/"));
        assert!(item_under_prefix("reportdata/a.yml", Some(1), &roots, "/data/corpus/reportdata"));
        assert!(item_under_prefix("reportdata\\a.yml", Some(1), &roots, "/data/corpus/reportdata/"));
        assert!(item_under_prefix("cfg/c.yml", Some(1), &roots, "/data"));
        assert!(!item_under_prefix("f", Some(1), &roots, "/data/corp"));
        assert!(item_under_prefix("ReportData/a.yml", Some(1), &roots, "reportdata/"));
        assert!(!item_under_prefix("reportdata/a.yml", Some(9), &roots, "/data/corpus/reportdata"));
        assert!(!item_under_prefix("reportdata/a.yml", None, &roots, "/data/corpus/reportdata"));
    }

    #[test]
    fn parse_lines_accepts_the_three_forms_and_names_them_on_error() {
        assert_eq!(parse_lines("3-7"), Ok(LineWindow { start: 3, end: Some(7) }));
        assert_eq!(parse_lines(" 4 "), Ok(LineWindow { start: 4, end: Some(4) }));
        assert_eq!(parse_lines("5-"), Ok(LineWindow { start: 5, end: None }));
        assert_eq!(parse_lines("2-2"), Ok(LineWindow { start: 2, end: Some(2) }));
        for bad in ["", "0-3", "7-3", "a-b", "-4", "1-2-3", "1..4", "x"] {
            let err = parse_lines(bad).unwrap_err();
            assert!(err.contains("\"a-b\""), "{bad}: {err}");
        }
    }

    #[test]
    fn window_text_slices_clamps_end_and_rejects_start_past_eof() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!(window_text(text, &LineWindow { start: 2, end: Some(3) }).unwrap(), ("two\nthree".to_string(), 2, 3, 4));
        assert_eq!(window_text(text, &LineWindow { start: 3, end: None }).unwrap(), ("three\nfour".to_string(), 3, 4, 4));
        assert_eq!(window_text(text, &LineWindow { start: 4, end: Some(99) }).unwrap(), ("four".to_string(), 4, 4, 4), "end clamps");
        let err = window_text(text, &LineWindow { start: 5, end: Some(6) }).unwrap_err();
        assert!(err.contains("4 line(s)"), "{err}");
        assert!(window_text("", &LineWindow { start: 1, end: None }).is_err(), "empty text has no line 1");
        assert_eq!(window_text("a\r\nb", &LineWindow { start: 2, end: None }).unwrap().0, "b", "CRLF lines");
    }

    #[test]
    fn validate_path_prefix_rejects_malformed_forms_with_the_expected_form() {
        for ok in [None, Some(""), Some("  "), Some("docs/"), Some("cfg/routing"), Some("reportdata\\a"), Some("/data/corpus/cfg"), Some("C:/corpus/cfg"), Some("C:\\corpus"), Some("report%"), Some("a b/[draft]/")] {
            assert!(validate_path_prefix(ok).is_ok(), "{ok:?}");
        }
        for bad in ["docs/*.md", "cfg/?", "file:///data/x", "../etc", "docs/../x", "./docs", "~/notes", "C:docs", "a\u{0}b"] {
            let err = validate_path_prefix(Some(bad)).unwrap_err();
            assert!(err.contains("malformed") && err.contains("Expected a relative item path"), "{bad}: {err}");
        }
    }

    #[test]
    fn read_item_windows_scopes_and_gates() {
        use crate::read_gate::Gate;
        let (_dir, conn, iid) = seed_db();
        conn.execute("UPDATE ingested_items SET text = 'l1\nl2\nl3' WHERE id = ?1", [iid]).unwrap();
        let agent = Gate { ui_caller: false, agents_allowed: false };
        let trusted = Gate { ui_caller: false, agents_allowed: true };
        let ui = Gate { ui_caller: true, agents_allowed: false };
        let q = |lines: Option<&str>, public_only: Option<bool>| GetQuery { lines: lines.map(str::to_string), public_only };
        let v = read_item(&conn, iid, &q(Some("2-3"), None), &agent).ok().expect("internal source window");
        assert_eq!(v["content"], "l2\nl3");
        assert_eq!(v["text"], "l2\nl3");
        assert_eq!(v["lines"], json!({"start": 2, "end": 3, "total": 3}));
        assert!(matches!(read_item(&conn, iid, &q(Some("9"), None), &agent), Err(ReadRefusal::BadRequest(_))));
        assert!(matches!(read_item(&conn, iid, &q(Some("zz"), None), &agent), Err(ReadRefusal::BadRequest(_))));
        assert!(matches!(read_item(&conn, iid, &q(None, Some(true)), &agent), Err(ReadRefusal::NotFound(_))), "internal is not public");
        assert!(matches!(read_item(&conn, iid + 99, &q(None, None), &agent), Err(ReadRefusal::NotFound(_))));
        conn.execute("UPDATE sources SET label = 'sensitive'", []).unwrap();
        assert!(matches!(read_item(&conn, iid, &q(Some("1"), None), &agent), Err(ReadRefusal::Gated(_))), "untrusted window gated");
        assert!(matches!(read_item(&conn, iid, &q(None, None), &agent), Err(ReadRefusal::Gated(_))), "whole file gated too (R35.5)");
        assert!(read_item(&conn, iid, &q(Some("1"), None), &trusted).is_ok(), "trusted provider reads");
        assert!(read_item(&conn, iid, &q(None, None), &ui).is_ok(), "UI reads everything");
        conn.execute("UPDATE sources SET label = 'public'", []).unwrap();
        assert!(read_item(&conn, iid, &q(Some("1"), Some(true)), &agent).is_ok(), "public source in public scope");
        // R35.5: the override drops the item from the public scope even for
        // callers that are otherwise open.
        conn.execute("UPDATE ingested_items SET sensitive_override = 1 WHERE id = ?1", [iid]).unwrap();
        assert!(matches!(read_item(&conn, iid, &q(None, Some(true)), &trusted), Err(ReadRefusal::NotFound(_))), "marked drops from public scope");
        assert!(matches!(read_item(&conn, iid, &q(None, Some(true)), &ui), Err(ReadRefusal::NotFound(_))), "UI preview shows the share-safe view");
        assert!(read_item(&conn, iid, &q(None, None), &ui).is_ok(), "plain UI read unaffected");
    }

    #[test]
    fn listing_public_only_keeps_public_sources() {
        let (_dir, conn, sa, sb) = seed_list_db();
        conn.execute("UPDATE sources SET label = 'public' WHERE id = ?1", [sb]).unwrap();
        let all = list_items(&conn, None, None, 100, 0).1;
        let f = ListFilter { path_prefix: None, source_id: None, public_only: true };
        let (items, total) = list_items_filtered(&conn, &f, 100, 0);
        assert!(total > 0 && total < all, "{total} of {all}");
        assert!(items.iter().all(|i| i["source_id"] == sb), "{items:?}");
        let f = ListFilter { path_prefix: None, source_id: Some(sa), public_only: true };
        assert_eq!(list_items_filtered(&conn, &f, 100, 0).1, 0);
    }
}
