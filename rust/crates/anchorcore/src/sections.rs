//! Sections TOC — R14.1 hierarchical tree.
//!
//! R36.6 navigation: `GET /items/:id/outline` renders one item's section
//! tree with per-section sizes, and `GET /nodes/:id` looks up a single tree
//! node (nodes ≡ sections; chunks are leaves reached via the section-chunks
//! and item endpoints) with opt-in parent/children/siblings embedding.

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
pub struct ListQuery {
    pub item_id: Option<i64>,
    pub source_id: Option<i64>,
    /// Outline depth cap: only sections with `level <= depth` (huge markdown
    /// files can hold thousands of headings — walk the tree incrementally).
    pub depth: Option<i64>,
}

fn section_row(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let meta_text: String = row.get(11)?;
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "item_id": row.get::<_, i64>(1)?,
        "parent_id": row.get::<_, Option<i64>>(2)?,
        "level": row.get::<_, i64>(3)?,
        "title": row.get::<_, String>(4)?,
        "path": row.get::<_, String>(5)?,
        "chunk_range": row.get::<_, String>(6)?,
        "summary": row.get::<_, String>(7)?,
        "line_start": row.get::<_, i64>(8)?,
        "line_end": row.get::<_, i64>(9)?,
        "kind": row.get::<_, String>(10)?,
        "meta": serde_json::from_str::<Value>(&meta_text).unwrap_or(Value::Null),
        "created_at": row.get::<_, Option<String>>(12)?,
    }))
}

pub async fn list_handler(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let item_id = q.item_id;
    let source_id = q.source_id;
    let depth = q.depth;
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut sql = String::from(
            "SELECT id, item_id, parent_id, level, title, path, chunk_range, summary, line_start, line_end, kind, meta, created_at FROM sections WHERE 1=1",
        );
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(id) = item_id {
            sql.push_str(" AND item_id = ?");
            params.push(rusqlite::types::Value::Integer(id));
        }
        if let Some(sid) = source_id {
            sql.push_str(" AND item_id IN (SELECT id FROM ingested_items WHERE source_id = ?)");
            params.push(rusqlite::types::Value::Integer(sid));
        }
        if let Some(d) = depth {
            sql.push_str(" AND level <= ?");
            params.push(rusqlite::types::Value::Integer(d));
        }
        sql.push_str(" ORDER BY level ASC, id ASC");
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), section_row) {
            Ok(m) => m,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Json(Value::Array(out))
    })
    .await
    .unwrap();
    result
}

/// One tree node: section identity plus its size (leaf chunk count +
/// total chunk chars) so clients can render and budget without fetching leaves.
struct SectionNode {
    id: i64,
    item_id: i64,
    parent_id: Option<i64>,
    level: i64,
    title: String,
    path: String,
    chunk_count: i64,
    total_chars: i64,
    line_start: i64,
    line_end: i64,
    kind: String,
    meta: String,
}

fn node_json(n: &SectionNode) -> Value {
    json!({
        "id": n.id,
        "item_id": n.item_id,
        "parent_id": n.parent_id,
        "level": n.level,
        "title": n.title.clone(),
        "path": n.path.clone(),
        "chunk_count": n.chunk_count,
        "total_chars": n.total_chars,
        "line_start": n.line_start,
        "line_end": n.line_end,
        "kind": n.kind.clone(),
        "meta": serde_json::from_str::<Value>(&n.meta).unwrap_or(Value::Null),
    })
}

fn node_from_row(row: &rusqlite::Row) -> rusqlite::Result<SectionNode> {
    Ok(SectionNode {
        id: row.get(0)?,
        item_id: row.get(1)?,
        parent_id: row.get(2)?,
        level: row.get(3)?,
        title: row.get(4)?,
        path: row.get(5)?,
        chunk_count: row.get(6)?,
        total_chars: row.get(7)?,
        line_start: row.get(8)?,
        line_end: row.get(9)?,
        kind: row.get(10)?,
        meta: row.get(11)?,
    })
}

const NODE_SIZES_SELECT: &str = "SELECT s.id, s.item_id, s.parent_id, s.level, s.title, s.path, COUNT(c.id), COALESCE(SUM(LENGTH(c.content)), 0), s.line_start, s.line_end, s.kind, s.meta FROM sections s LEFT JOIN chunks c ON c.section_id = s.id";

/// Whole item's sections with sizes, ordered for tree rendering (`level`,
/// then insertion order — same as the list handler).
fn item_sections(conn: &rusqlite::Connection, item_id: i64) -> Vec<SectionNode> {
    let mut stmt = match conn.prepare(&format!("{NODE_SIZES_SELECT} WHERE s.item_id = ?1 GROUP BY s.id ORDER BY s.level ASC, s.id ASC")) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([item_id], node_from_row).map(|m| m.filter_map(|r| r.ok()).collect()).unwrap_or_default()
}

fn one_section(conn: &rusqlite::Connection, section_id: i64) -> Option<SectionNode> {
    conn.query_row(&format!("{NODE_SIZES_SELECT} WHERE s.id = ?1 GROUP BY s.id"), [section_id], node_from_row).ok()
}

#[derive(Deserialize, Default)]
pub struct OutlineQuery {
    /// Depth cap: only nodes with `level <= depth` (huge markdown files can
    /// hold thousands of headings — walk the outline incrementally).
    pub depth: Option<i64>,
    /// R36.15: the share-safe scope — a non-public item reads as 404.
    pub public_only: Option<bool>,
}

/// R35.5: `public_only` scope for outline/node reads (metadata only, so
/// no byte gate — but an effective-Sensitive item reads as not found, like
/// any out-of-scope item). Returns the item's effective inputs for the
/// byte gate when the caller goes on to read text.
fn scope_item(conn: &rusqlite::Connection, item_id: i64, public_only: bool, not_found: &'static str) -> Result<(String, bool), ReadRefusal> {
    let (label, marked) = crate::read_gate::item_sensitivity(conn, item_id).ok_or(ReadRefusal::NotFound(not_found))?;
    crate::read_gate::check_public(&label, marked, public_only, not_found)?;
    Ok((label, marked))
}

pub async fn outline_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
    Query(q): Query<OutlineQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let depth = q.depth;
    let public_only = q.public_only.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        if let Err(ReadRefusal::NotFound(d)) = scope_item(&conn, item_id, public_only, "Item not found") {
            return Err((StatusCode::NOT_FOUND, Json(json!({"detail": d}))));
        }
        let nodes = item_sections(&conn, item_id);
        let sections: Vec<Value> = nodes
            .iter()
            .filter(|n| depth.map(|d| n.level <= d).unwrap_or(true))
            .map(node_json)
            .collect();
        let count = sections.len();
        Ok(json!({"item_id": item_id, "section_count": count, "sections": sections}))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

#[derive(Deserialize, Default)]
pub struct NodeQuery {
    pub include: Option<String>,
    /// R36.15: cap on `include=text` (chars, default `NODE_TEXT_DEFAULT_CHARS`).
    pub max_chars: Option<usize>,
    /// R36.15: the share-safe scope — a node of a non-public item reads as 404.
    pub public_only: Option<bool>,
}

/// R36.15: `include=text` default and ceiling (chars).
pub const NODE_TEXT_DEFAULT_CHARS: usize = 32_000;
pub const NODE_TEXT_MAX_CHARS: usize = 1_000_000;

/// R36.15: `text` in the `include=` list (separate from the link selector
/// so the link triple keeps its shape).
fn wants_text(include: Option<&str>) -> bool {
    include.is_some_and(|list| list.split(',').any(|t| t.trim().eq_ignore_ascii_case("text")))
}

/// Same subtree rule as retrieval's parent return (`retrieval.rs`
/// `subtree_leaf_chunks`): own + descendants' leaf chunks, document order,
/// document and caption kinds only (never summaries or entity paraphrase).
const NODE_SUBTREE_LEAVES: &str = "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT s.id FROM sections s JOIN sub ON s.parent_id = sub.id) SELECT c.content FROM chunks c WHERE c.section_id IN (SELECT id FROM sub) AND c.kind IN ('document','caption') ORDER BY c.id";

fn subtree_texts(conn: &rusqlite::Connection, section_id: i64) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(NODE_SUBTREE_LEAVES) else { return vec![] };
    stmt.query_map([section_id], |r| r.get::<_, String>(0)).map(|m| m.filter_map(|r| r.ok()).collect()).unwrap_or_default()
}

/// R36.15: the section's ordered leaf text, joined like a parent-return
/// hit (`\n\n`), cut at `max_chars` on a char boundary. `text_chars` is the
/// full size so a caller can tell how much it is missing.
fn node_text_fields(conn: &rusqlite::Connection, section_id: i64, max_chars: usize) -> Value {
    let pieces = subtree_texts(conn, section_id);
    let full = pieces.join("\n\n");
    let total = full.chars().count();
    let text: String = if total > max_chars { full.chars().take(max_chars).collect() } else { full };
    json!({"text": text, "text_truncated": total > max_chars, "text_chars": total, "text_chunk_count": pieces.len()})
}

fn merge_text(out: &mut Value, fields: Value) {
    if let (Some(o), Value::Object(f)) = (out.as_object_mut(), fields) {
        o.extend(f);
    }
}

/// `include=` embedding selector: comma list from `parent,children,siblings`
/// (JSON:API-style — absent or empty embeds nothing; unknown tokens ignored).
fn parse_node_include(include: Option<&str>) -> (bool, bool, bool) {
    let mut parent = false;
    let mut children = false;
    let mut siblings = false;
    if let Some(list) = include {
        for tok in list.split(',') {
            match tok.trim().to_lowercase().as_str() {
                "parent" => parent = true,
                "children" => children = true,
                "siblings" => siblings = true,
                _ => {}
            }
        }
    }
    (parent, children, siblings)
}

fn node_links(node: &SectionNode, all: &[SectionNode], want_parent: bool, want_children: bool, want_siblings: bool) -> Value {
    let mut out = json!({"node": node_json(node)});
    if want_parent {
        let p = node.parent_id.and_then(|pid| all.iter().find(|s| s.id == pid));
        out["parent"] = p.map(node_json).unwrap_or(Value::Null);
    }
    if want_children {
        let kids: Vec<Value> = all.iter().filter(|s| s.parent_id == Some(node.id)).map(node_json).collect();
        out["children"] = Value::Array(kids);
    }
    if want_siblings {
        let sibs: Vec<Value> = all.iter().filter(|s| s.id != node.id && s.parent_id == node.parent_id).map(node_json).collect();
        out["siblings"] = Value::Array(sibs);
    }
    out
}

pub async fn node_handler(
    State(state): State<AppState>,
    Path(node_id): Path<i64>,
    Query(q): Query<NodeQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let gate = crate::read_gate::Gate::from_request(&headers, &state.settings);
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        read_node(&conn, node_id, &q, &gate)
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(r) => Err(crate::read_gate::into_api_error(&state.data_dir, r).await),
    }
}

/// One node read: scope first (404), then links, then the byte-gated text.
fn read_node(conn: &rusqlite::Connection, node_id: i64, q: &NodeQuery, gate: &crate::read_gate::Gate) -> Result<Value, ReadRefusal> {
    const NOT_FOUND: &str = "Node not found";
    let node = one_section(conn, node_id).ok_or(ReadRefusal::NotFound(NOT_FOUND))?;
    let (label, marked) = scope_item(conn, node.item_id, q.public_only.unwrap_or(false), NOT_FOUND)?;
    let include = q.include.as_deref();
    let (want_parent, want_children, want_siblings) = parse_node_include(include);
    let mut out = if want_parent || want_children || want_siblings {
        node_links(&node, &item_sections(conn, node.item_id), want_parent, want_children, want_siblings)
    } else {
        json!({"node": node_json(&node)})
    };
    if wants_text(include) {
        gate.check_bytes(&label, marked)?;
        let cap = q.max_chars.unwrap_or(NODE_TEXT_DEFAULT_CHARS).clamp(1, NODE_TEXT_MAX_CHARS);
        merge_text(&mut out, node_text_fields(conn, node.id, cap));
    }
    Ok(out)
}

fn section_chunk_row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, i64>(0)?,
        "item_id": r.get::<_, Option<i64>>(1)?,
        "source_ref": r.get::<_, String>(2)?,
        "content": r.get::<_, String>(3)?,
        "level": r.get::<_, i64>(4)?,
        "path": r.get::<_, String>(5)?,
        "line_start": r.get::<_, i64>(6)?,
        "line_end": r.get::<_, i64>(7)?,
        "created_at": r.get::<_, Option<String>>(8)?,
    }))
}

pub async fn chunks_handler(
    State(state): State<AppState>,
    axum::extract::Path(section_id): axum::extract::Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let gate = crate::read_gate::Gate::from_request(&headers, &state.settings);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        // R35.5: a section's chunks are one item's bytes — one byte gate.
        // A missing section reads as an empty list (pre-R35.5 contract).
        if let Some(node) = one_section(&conn, section_id) {
            let (label, marked) = crate::read_gate::item_sensitivity(&conn, node.item_id).unwrap_or_else(|| ("internal".to_string(), false));
            gate.check_bytes(&label, marked)?;
        }
        let mut stmt = match conn.prepare(
            "SELECT id, item_id, source_ref, content, level, path, line_start, line_end, created_at FROM chunks WHERE section_id = ?1 ORDER BY id",
        ) {
            Ok(s) => s,
            Err(e) => return Ok(Json(json!({"error": e.to_string()}))),
        };
        let rows = stmt.query_map([section_id], section_chunk_row).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Ok(Json(Value::Array(out)))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(v),
        Err(r) => Err(crate::read_gate::into_api_error(&state.data_dir, r).await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Seeds one item with a three-section tree: root `Intro` (one 11-char
    /// chunk), root `API` (no chunks), child `get_user` under `API` (3+5 chars).
    fn seed_tree() -> (tempfile::TempDir, rusqlite::Connection, i64, i64, i64, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'api.md','api.md','body','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,0,'Intro','Intro')", [iid]).unwrap();
        let intro = conn.last_insert_rowid();
        conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,NULL,0,'API','API')", [iid]).unwrap();
        let api = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,?2,1,'get_user','API › get_user')",
            rusqlite::params![iid, api],
        )
        .unwrap();
        let sym = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,'api.md','hello world')", rusqlite::params![iid, intro]).unwrap();
        conn.execute("INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,'api.md','aaa')", rusqlite::params![iid, sym]).unwrap();
        conn.execute("INSERT INTO chunks (item_id, section_id, source_ref, content) VALUES (?1,?2,'api.md','bbbbb')", rusqlite::params![iid, sym]).unwrap();
        (dir, conn, iid, intro, api, sym)
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

    #[test]
    fn outline_orders_tree_and_sizes_sections() {
        let (_dir, conn, iid, intro, api, sym) = seed_tree();
        let nodes = item_sections(&conn, iid);
        assert_eq!(nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![intro, api, sym], "level then insertion order");
        assert_eq!((nodes[0].chunk_count, nodes[0].total_chars), (1, 11));
        assert_eq!((nodes[1].chunk_count, nodes[1].total_chars), (0, 0), "childless section reads zero, not NULL");
        assert_eq!((nodes[2].chunk_count, nodes[2].total_chars), (2, 8));
        assert_eq!(nodes[2].parent_id, Some(api));
        assert!(item_sections(&conn, iid + 99).is_empty());
        let v = node_json(&nodes[2]);
        assert_eq!(v.get("title").and_then(|t| t.as_str()), Some("get_user"));
        assert_eq!(v.get("path").and_then(|t| t.as_str()), Some("API › get_user"));
        assert_eq!(v.get("parent_id").and_then(|t| t.as_i64()), Some(api));
        assert_eq!(v.get("level").and_then(|t| t.as_i64()), Some(1));
    }

    #[test]
    fn parse_node_include_selects_links() {
        assert_eq!(parse_node_include(None), (false, false, false));
        assert_eq!(parse_node_include(Some("")), (false, false, false));
        assert_eq!(parse_node_include(Some("parent,children,siblings")), (true, true, true));
        assert_eq!(parse_node_include(Some("children")), (false, true, false));
        assert_eq!(parse_node_include(Some(" Parent , BOGUS ")), (true, false, false), "case/space tolerant, unknown ignored");
    }

    fn ids(v: &Value, key: &str) -> Vec<i64> {
        v.get(key).and_then(|a| a.as_array()).cloned().unwrap_or_default().iter().filter_map(|n| n.get("id").and_then(|i| i.as_i64())).collect()
    }

    #[tokio::test]
    async fn outline_handler_serves_tree_and_404s() {
        let (dir, conn, iid, intro, _api, _sym) = seed_tree();
        drop(conn);
        let state = app_state(&dir);
        let Json(v) = outline_handler(State(state.clone()), Path(iid), Query(OutlineQuery { depth: None, public_only: None })).await.expect("outline must succeed");
        assert_eq!(v.get("item_id").and_then(|i| i.as_i64()), Some(iid));
        assert_eq!(v.get("section_count").and_then(|c| c.as_i64()), Some(3));
        let sections = v.get("sections").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].get("id").and_then(|i| i.as_i64()), Some(intro));
        assert_eq!(sections[0].get("chunk_count").and_then(|c| c.as_i64()), Some(1));
        match outline_handler(State(state), Path(iid + 99), Query(OutlineQuery { depth: None, public_only: None })).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("outline on missing item must 404"),
        }
    }

    #[tokio::test]
    async fn outline_handler_depth_caps_levels() {
        // R38.3: `?depth=0` serves roots only (the seed tree is two roots +
        // one level-1 child); absent depth serves everything.
        let (dir, conn, iid, intro, api, _sym) = seed_tree();
        drop(conn);
        let state = app_state(&dir);
        let Json(v) = outline_handler(State(state.clone()), Path(iid), Query(OutlineQuery { depth: Some(0), public_only: None })).await.expect("outline must succeed");
        assert_eq!(v.get("section_count").and_then(|c| c.as_i64()), Some(2));
        assert_eq!(ids(&v, "sections"), vec![intro, api]);
    }

    #[tokio::test]
    async fn node_handler_embeds_navigable_links() {
        let (dir, conn, _iid, intro, api, sym) = seed_tree();
        drop(conn);
        let state = app_state(&dir);
        // Bare lookup carries the node only (JSON:API-style include).
        let Json(v) = node_handler(State(state.clone()), Path(sym), Query(NodeQuery::default()), HeaderMap::default()).await.expect("node must succeed");
        assert_eq!(v.get("node").and_then(|n| n.get("id")).and_then(|i| i.as_i64()), Some(sym));
        assert!(v.get("parent").is_none() && v.get("children").is_none() && v.get("siblings").is_none());
        // Full include on the leaf: parent up, no children, no siblings.
        let full = NodeQuery { include: Some("parent,children,siblings".to_string()), ..Default::default() };
        let Json(v) = node_handler(State(state.clone()), Path(sym), Query(full), HeaderMap::default()).await.expect("node must succeed");
        assert_eq!(v.get("parent").and_then(|n| n.get("id")).and_then(|i| i.as_i64()), Some(api));
        assert_eq!(ids(&v, "children"), Vec::<i64>::new());
        assert_eq!(ids(&v, "siblings"), Vec::<i64>::new());
        // Mid-tree: explicit null parent for roots, children + siblings lateral.
        let Json(v) = node_handler(State(state.clone()), Path(api), Query(NodeQuery { include: Some("parent,children,siblings".to_string()), ..Default::default() }), HeaderMap::default()).await.expect("node must succeed");
        assert!(v.get("parent").is_some_and(|p| p.is_null()), "root parent is explicit null");
        assert_eq!(ids(&v, "children"), vec![sym]);
        assert_eq!(ids(&v, "siblings"), vec![intro]);
        // Partial include embeds only what was asked.
        let Json(v) = node_handler(State(state.clone()), Path(intro), Query(NodeQuery { include: Some("siblings".to_string()), ..Default::default() }), HeaderMap::default()).await.expect("node must succeed");
        assert!(v.get("parent").is_none() && v.get("children").is_none());
        assert_eq!(ids(&v, "siblings"), vec![api]);
        match node_handler(State(state), Path(sym + 99), Query(NodeQuery::default()), HeaderMap::default()).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("node on missing section must 404"),
        }
    }

    #[test]
    fn node_text_assembles_subtree_in_document_order_and_truncates() {
        let (_dir, conn, _iid, intro, api, sym) = seed_tree();
        let v = node_text_fields(&conn, api, 1000);
        assert_eq!(v["text"], "aaa\n\nbbbbb", "childless parent reads its descendants' leaves");
        assert_eq!((v["text_truncated"].as_bool(), v["text_chars"].as_u64(), v["text_chunk_count"].as_u64()), (Some(false), Some(10), Some(2)));
        let v = node_text_fields(&conn, sym, 4);
        assert_eq!((v["text"].as_str(), v["text_truncated"].as_bool(), v["text_chars"].as_u64()), (Some("aaa\n"), Some(true), Some(10)));
        assert_eq!(node_text_fields(&conn, intro, 1000)["text"], "hello world");
        assert!(wants_text(Some("parent, TEXT")) && !wants_text(Some("parent,texts")) && !wants_text(None));
    }

    #[test]
    fn read_node_scopes_and_gates_text_only() {
        use crate::read_gate::Gate;
        let (_dir, conn, _iid, _intro, api, _sym) = seed_tree();
        let agent = Gate { ui_caller: false, provider_trusted: false };
        let trusted = Gate { ui_caller: false, provider_trusted: true };
        let ui = Gate { ui_caller: true, provider_trusted: false };
        let q = |include: &str, public_only: Option<bool>| NodeQuery { include: Some(include.to_string()), max_chars: None, public_only };
        let v = read_node(&conn, api, &q("text,children", None), &agent).ok().expect("internal source reads");
        assert_eq!(v["text"], "aaa\n\nbbbbb");
        assert_eq!(ids(&v, "children").len(), 1, "links and text combine");
        assert!(matches!(read_node(&conn, api, &q("text", Some(true)), &agent), Err(ReadRefusal::NotFound(_))), "internal is not public");
        conn.execute("UPDATE sources SET label = 'sensitive'", []).unwrap();
        assert!(matches!(read_node(&conn, api, &q("text", None), &agent), Err(ReadRefusal::Gated(_))), "untrusted text gated");
        assert!(read_node(&conn, api, &q("children", None), &agent).is_ok(), "metadata stays readable");
        assert!(read_node(&conn, api, &q("text", None), &trusted).is_ok(), "trusted provider reads");
        assert!(read_node(&conn, api, &q("text", None), &ui).is_ok(), "UI reads everything");
        conn.execute("UPDATE sources SET label = 'public'", []).unwrap();
        assert!(read_node(&conn, api, &q("text", Some(true)), &agent).is_ok());
        assert!(matches!(read_node(&conn, api + 99, &q("text", None), &trusted), Err(ReadRefusal::NotFound(_))));
    }
}
