//! R46.12: "Unplaced images" — people attribute the images sync could not
//! place (an image file no synced document links: attribution `none`).
//!
//! Owner decision 2026-10-23 (b): captions are people-only. Nothing here
//! calls a model, proposes a caption or ranks a guess; the only writer is
//! `PATCH /media/:uid` from a person. Same rule as the glossary and
//! `verified`.
//!
//! A person's caption becomes one chunk, `kind='caption'`, uid
//! `caption:<asset uid>`, path ending in `caption by a person`. It sits on
//! the target (an item, a section of one, or an entity's item) or, with no
//! target, on the image's own item, so it takes that item's scope and
//! source label like any other chunk. Targets must be in the image's own
//! source (one label for image and caption). The chunk is derived state:
//! `materialize` rebuilds it from `assets.manual_caption`/`manual_target`,
//! and a document re-sync keeps it (`store_doc_chunks` skips the kind) and
//! re-resolves its section (`refresh_for_item`). Targets are stored by uid
//! + section path, never local ids, so they survive re-sync and backup.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

pub const CAPTION_KIND: &str = "caption";
/// Leaf crumb of every caption chunk's path: how a caption cites.
pub const CAPTION_CRUMB: &str = "caption by a person";
const UID_PREFIX: &str = "caption:";
const MAX_CAPTION_CHARS: usize = 2000;
const ATTRIBUTIONS: &[&str] = &["none", "ignored", "manual", "referenced", "embedded", "contextual"];

#[derive(Debug, Deserialize, Default, Clone)]
pub struct TargetIn {
    pub item_id: Option<i64>,
    pub section_id: Option<i64>,
    pub entity_id: Option<i64>,
}

/// `PATCH /media/:uid` body. Absent = unchanged; `target: null` and
/// `caption: ""` clear.
#[derive(Debug, Deserialize, Default)]
pub struct Patch {
    pub caption: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub target: Option<Option<TargetIn>>,
    pub ignore: Option<bool>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, PartialEq)]
pub enum PatchError {
    NotFound,
    Invalid(String),
    Db(String),
}

impl From<rusqlite::Error> for PatchError {
    fn from(e: rusqlite::Error) -> Self {
        PatchError::Db(e.to_string())
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub attribution: Option<String>,
    pub source_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn caption_uid(asset_uid: &str) -> String {
    format!("{UID_PREFIX}{asset_uid}")
}

fn invalid(msg: &str) -> PatchError {
    PatchError::Invalid(msg.to_string())
}

// ----- targets -----

fn section_target(conn: &Connection, id: i64) -> Result<(i64, Value), PatchError> {
    let row: Option<(i64, String)> = conn.query_row("SELECT item_id, path FROM sections WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let (item, path) = row.ok_or_else(|| invalid("target section not found"))?;
    Ok((item, json!({"section": path})))
}

fn entity_target(conn: &Connection, id: i64) -> Result<(i64, Value), PatchError> {
    let row: Option<(Option<i64>, String)> =
        conn.query_row("SELECT item_id, COALESCE(uid,'') FROM entities WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    match row {
        Some((Some(item), uid)) if !uid.is_empty() => Ok((item, json!({"entity": uid}))),
        Some(_) => Err(invalid("target entity has no document")),
        None => Err(invalid("target entity not found")),
    }
}

/// The stored form of a target (`assets.manual_target`): uids + section
/// path. Validated here, before any write (single-writer rule).
fn store_target(conn: &Connection, source_id: i64, t: &TargetIn) -> Result<String, PatchError> {
    let given = [t.item_id, t.section_id, t.entity_id].iter().filter(|v| v.is_some()).count();
    if given != 1 {
        return Err(invalid("target takes exactly one of item_id, section_id, entity_id"));
    }
    let (item_id, mut stored) = match (t.section_id, t.entity_id) {
        (Some(id), _) => section_target(conn, id)?,
        (_, Some(id)) => entity_target(conn, id)?,
        _ => (t.item_id.unwrap_or_default(), json!({})),
    };
    let row: Option<(i64, String)> =
        conn.query_row("SELECT source_id, COALESCE(uid,'') FROM ingested_items WHERE id=?1", [item_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let (item_source, item_uid) = row.ok_or_else(|| invalid("target item not found"))?;
    if item_source != source_id {
        return Err(invalid("target must be in the image's own source"));
    }
    stored["item"] = json!(item_uid);
    Ok(stored.to_string())
}

/// Where the caption chunk sits now: (item id, section id, section path).
/// A target that no longer resolves falls back to its item, then to the
/// image's own item and section; `own_crumb` (a PDF image's `p. N`) names
/// that place more precisely than the section path.
fn resolve_target(conn: &Connection, stored: &str, own_item: i64, own_section: Option<i64>, own_crumb: Option<&str>) -> (i64, Option<i64>, String) {
    let own = || {
        let path = own_section.and_then(|id| conn.query_row("SELECT path FROM sections WHERE id=?1", [id], |r| r.get::<_, String>(0)).ok());
        let crumb = own_crumb.map(str::to_string).or(path).unwrap_or_default();
        (own_item, own_section, crumb)
    };
    let Ok(t) = serde_json::from_str::<Value>(stored) else { return own() };
    let by_uid = |table: &str, uid: &str| -> Option<i64> {
        let col = if table == "entities" { "item_id" } else { "id" };
        conn.query_row(&format!("SELECT {col} FROM {table} WHERE uid=?1"), [uid], |r| r.get(0)).ok().flatten()
    };
    let entity_item = t["entity"].as_str().and_then(|u| by_uid("entities", u));
    let Some(item) = entity_item.or_else(|| t["item"].as_str().and_then(|u| by_uid("ingested_items", u))) else {
        return own();
    };
    let section = t["section"].as_str().and_then(|p| {
        conn.query_row("SELECT id, path FROM sections WHERE item_id=?1 AND path=?2 ORDER BY level, id LIMIT 1", params![item, p], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .ok()
    });
    match section {
        Some((id, path)) => (item, Some(id), path),
        None => (item, None, String::new()),
    }
}

// ----- the caption chunk -----

/// `p. N` from a PDF image's context (`p. N — nearby text`, R46.16).
fn page_crumb(context: &str) -> Option<String> {
    let head = context.split(" — ").next()?;
    let n = head.strip_prefix("p. ")?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| head.to_string())
}

/// Rebuild one asset's caption chunk from its people-owned fields. An
/// unchanged caption at an unchanged path keeps its vector; otherwise the
/// vector clears and the backfill re-embeds it.
pub fn materialize(conn: &Connection, asset_id: i64) -> Result<(), String> {
    let row: (String, i64, Option<i64>, String, String, String, String) = conn
        .query_row("SELECT uid, item_id, section_id, manual_caption, manual_target, attribution, context FROM assets WHERE id=?1", [asset_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })
        .map_err(|e| e.to_string())?;
    let (uid, own_item, own_section, caption, target, attribution, context) = row;
    let chunk_uid = caption_uid(&uid);
    if caption.trim().is_empty() || attribution == "ignored" {
        conn.execute("DELETE FROM chunks WHERE uid=?1", [&chunk_uid]).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let page = page_crumb(&context);
    let (item, section, crumb) = resolve_target(conn, &target, own_item, own_section, page.as_deref());
    let path = if crumb.is_empty() { CAPTION_CRUMB.to_string() } else { format!("{crumb} > {CAPTION_CRUMB}") };
    let source_ref: String = conn
        .query_row("SELECT external_id FROM ingested_items WHERE id=?1", [own_item], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    write_caption_chunk(conn, &chunk_uid, (item, section), &path, &source_ref, &caption)
}

/// Update in place (keeping the vector when content + path are unchanged),
/// else insert.
fn write_caption_chunk(conn: &Connection, chunk_uid: &str, (item, section): (i64, Option<i64>), path: &str, source_ref: &str, caption: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE chunks SET item_id=?2, section_id=?3, path=?4, source_ref=?5, content=?6, \
             embedding = CASE WHEN content=?6 AND path=?4 THEN embedding END, \
             embed_fp = CASE WHEN content=?6 AND path=?4 THEN embed_fp END WHERE uid=?1",
            params![chunk_uid, item, section, path, source_ref, caption],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        conn.execute(
            "INSERT INTO chunks (item_id, kind, source_ref, content, section_id, level, path, created_at, uid) VALUES (?1,?2,?3,?4,?5,0,?6,datetime('now'),?7)",
            params![item, CAPTION_KIND, source_ref, caption, section, path, chunk_uid],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// After an item's chunks and sections are rebuilt (re-sync): re-resolve
/// every caption that sits on it, targets it, or belongs to its images.
pub fn refresh_for_item(conn: &Connection, item_id: i64) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id FROM assets a WHERE a.manual_caption <> '' AND (a.item_id = ?1 \
             OR EXISTS (SELECT 1 FROM chunks c WHERE c.uid = 'caption:' || a.uid AND c.item_id = ?1) \
             OR (CASE WHEN json_valid(a.manual_target) THEN json_extract(a.manual_target, '$.item') END) = (SELECT uid FROM ingested_items WHERE id = ?1))",
        )
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt.query_map([item_id], |r| r.get(0)).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();
    ids.into_iter().try_for_each(|id| materialize(conn, id))
}

// ----- PATCH -----

struct Current {
    id: i64,
    source_id: i64,
    origin: String,
    attribution: String,
    caption: String,
    target: String,
    /// What sync derives when a person clears their fields: `none` for
    /// files (refreshed after the write), the page/chunk split for embeds.
    sync_attribution: &'static str,
}

fn load_current(conn: &Connection, uid: &str) -> Result<Current, PatchError> {
    conn.query_row(
        "SELECT a.id, i.source_id, a.origin, a.attribution, a.manual_caption, a.manual_target \
         FROM assets a JOIN ingested_items i ON i.id = a.item_id WHERE a.uid=?1",
        [uid],
        |r| {
            Ok(Current {
                sync_attribution: "none",
                id: r.get(0)?,
                source_id: r.get(1)?,
                origin: r.get(2)?,
                attribution: r.get(3)?,
                caption: r.get(4)?,
                target: r.get(5)?,
            })
        },
    )
    .optional()?
    .ok_or(PatchError::NotFound)
    .and_then(|mut cur| {
        if cur.origin == "embed" {
            cur.sync_attribution = crate::assets::embed_sync_attribution(conn, cur.id).map_err(PatchError::Db)?;
        }
        Ok(cur)
    })
}

fn next_caption(cur: &Current, patch: &Patch) -> Result<String, PatchError> {
    let Some(c) = &patch.caption else { return Ok(cur.caption.clone()) };
    let c = c.trim();
    if c.chars().count() > MAX_CAPTION_CHARS {
        return Err(PatchError::Invalid(format!("caption is longer than {MAX_CAPTION_CHARS} characters")));
    }
    Ok(c.to_string())
}

/// People's fields decide: ignored wins, then any caption/target is
/// `manual`; clearing both hands the asset back to what sync derives.
fn next_attribution(cur: &Current, patch: &Patch, caption: &str, target: &str) -> String {
    let ignored = patch.ignore.unwrap_or(cur.attribution == "ignored");
    if ignored {
        "ignored".to_string()
    } else if !caption.is_empty() || !target.is_empty() {
        "manual".to_string()
    } else {
        cur.sync_attribution.to_string()
    }
}

pub fn apply_patch(conn: &Connection, uid: &str, patch: &Patch) -> Result<Value, PatchError> {
    let cur = load_current(conn, uid)?;
    let caption = next_caption(&cur, patch)?;
    let target = match &patch.target {
        None => cur.target.clone(),
        Some(None) => String::new(),
        Some(Some(t)) => store_target(conn, cur.source_id, t)?,
    };
    let attribution = next_attribution(&cur, patch, &caption, &target);
    conn.execute("BEGIN IMMEDIATE", [])?;
    let res = write_patch(conn, &cur, &caption, &target, &attribution);
    conn.execute(if res.is_ok() { "COMMIT" } else { "ROLLBACK" }, [])?;
    res.map_err(PatchError::Db)?;
    get(conn, uid)?.ok_or(PatchError::NotFound)
}

fn write_patch(conn: &Connection, cur: &Current, caption: &str, target: &str, attribution: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE assets SET manual_caption=?2, manual_target=?3, attribution=?4, updated_at=datetime('now') WHERE id=?1",
        params![cur.id, caption, target, attribution],
    )
    .map_err(|e| e.to_string())?;
    materialize(conn, cur.id)?;
    if cur.origin == "file" && attribution == "none" {
        crate::assets::refresh_file_attribution(conn, cur.source_id)?;
    }
    Ok(())
}

// ----- listing -----

const LIST_COLUMNS: &str = "a.uid, a.item_id, i.source_id, s.name, a.origin, a.src, i.external_id, a.mime, a.w, a.h, a.size, \
    a.bytes_hash IS NOT NULL, a.alt, a.caption, a.context, a.attribution, a.manual_caption, a.manual_target, a.updated_at";
const LIST_FROM: &str = "FROM assets a JOIN ingested_items i ON i.id = a.item_id JOIN sources s ON s.id = i.source_id";

/// Output names of `LIST_COLUMNS`, in order (`stored` and
/// `manual_target` are rewritten into `url` and `target`).
const LIST_NAMES: &[&str] = &[
    "asset_id", "item_id", "source_id", "source_name", "origin", "src", "path", "mime", "w", "h", "size",
    "stored", "alt", "caption", "context", "attribution", "manual_caption", "manual_target", "updated_at",
];

fn json_col(v: rusqlite::types::ValueRef) -> Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Null | ValueRef::Blob(_) => Value::Null,
    }
}

/// A stored target as the UI sees it: current ids + where it points.
fn target_view(conn: &Connection, stored: &str, own_item: i64) -> Value {
    if stored.is_empty() {
        return Value::Null;
    }
    let (item, section, path) = resolve_target(conn, stored, own_item, None, None);
    let title: String = conn.query_row("SELECT title FROM ingested_items WHERE id=?1", [item], |r| r.get(0)).unwrap_or_default();
    json!({"item_id": item, "section_id": section, "section_path": path, "item_title": title})
}

fn list_row(conn: &Connection, r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let mut row = serde_json::Map::new();
    for (i, name) in LIST_NAMES.iter().enumerate() {
        row.insert(name.to_string(), json_col(r.get_ref(i)?));
    }
    let uid = row["asset_id"].as_str().unwrap_or_default().to_string();
    let stored = row.remove("stored").and_then(|v| v.as_i64()) == Some(1);
    row.insert("url".into(), if stored { json!(format!("{}/{uid}", crate::assets::MEDIA_ROUTE)) } else { Value::Null });
    let target = row.remove("manual_target").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let own_item = row["item_id"].as_i64().unwrap_or_default();
    row.insert("target".into(), target_view(conn, &target, own_item));
    Ok(Value::Object(row))
}

pub fn get(conn: &Connection, uid: &str) -> Result<Option<Value>, PatchError> {
    let sql = format!("SELECT {LIST_COLUMNS} {LIST_FROM} WHERE a.uid=?1");
    Ok(conn.query_row(&sql, [uid], |r| list_row(conn, r)).optional()?)
}

/// `GET /media`: one attribution (default `none`, `all` for every asset),
/// optional source, newest first. `total` is the unpaged count (the badge).
pub fn list(conn: &Connection, q: &ListQuery) -> Result<Value, String> {
    let attribution = q.attribution.as_deref().unwrap_or("none").trim().to_ascii_lowercase();
    if attribution != "all" && !ATTRIBUTIONS.contains(&attribution.as_str()) {
        return Err(format!("attribution must be one of {} or all", ATTRIBUTIONS.join(", ")));
    }
    let filter = "WHERE (?1 = 'all' OR a.attribution = ?1) AND (?2 IS NULL OR i.source_id = ?2)";
    let total: i64 = conn
        .query_row(&format!("SELECT COUNT(*) {LIST_FROM} {filter}"), params![attribution, q.source_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let (limit, offset) = (q.limit.unwrap_or(50).clamp(1, 200), q.offset.unwrap_or(0).max(0));
    let sql = format!("SELECT {LIST_COLUMNS} {LIST_FROM} {filter} ORDER BY a.id DESC LIMIT ?3 OFFSET ?4");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Value> = stmt
        .query_map(params![attribution, q.source_id, limit, offset], |r| list_row(conn, r))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(json!({"items": rows, "total": total, "limit": limit, "offset": offset}))
}
