//! R47.2: the System tab's errors & warnings panel over `system_events`.
//!
//! `GET /system/errors` lists events (flat, or `group=true` to fold repeats
//! of one component + level + source + message into a row with a count and
//! first/last seen), `GET /system/errors/summary` feeds the filter controls,
//! and `POST /system/errors/dismiss` / `restore` hide or bring back what a
//! filter matches. Dismissing only stamps `dismissed_at` — rows are never
//! deleted, because the table is also the audit trail (MCP calls, gate
//! refusals) and feeds per-item flags (`pdf split-word repair`).

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::health::AppState;
use crate::pipeline::run_db;

const LEVELS: [&str; 3] = ["error", "warning", "info"];

type ApiError = (StatusCode, Json<Value>);

/// Query for the list and summary reads. `level` is the original single
/// filter; `levels` takes a comma list (`error,warning`).
#[derive(Deserialize, Default)]
pub struct ErrorsQuery {
    pub limit: Option<i64>,
    pub component: Option<String>,
    pub level: Option<String>,
    pub levels: Option<String>,
    pub source_id: Option<i64>,
    #[serde(default)]
    pub include_dismissed: bool,
    #[serde(default)]
    pub group: bool,
}

/// Body for dismiss/restore (required — `{}` matches every event): every
/// field narrows the match. `source_id` absent = any source, `null` = events
/// with no source (so a grouped row's dismiss hits exactly its own events).
#[derive(Deserialize, Default)]
pub struct DismissBody {
    pub component: Option<String>,
    pub level: Option<String>,
    pub levels: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    pub source_id: Option<Option<i64>>,
    pub message: Option<String>,
    pub ids: Option<Vec<i64>>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Option::<i64>::deserialize(d).map(Some)
}

#[derive(Default)]
struct Filter {
    component: Option<String>,
    levels: Vec<String>,
    /// `Some(None)` = events with no source.
    source_id: Option<Option<i64>>,
    message: Option<String>,
    ids: Option<Vec<i64>>,
    /// `Some(false)` = only shown rows, `Some(true)` = only dismissed rows.
    dismissed: Option<bool>,
}

fn bad_request(msg: String) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({ "detail": msg })))
}

fn parse_levels(single: Option<String>, many: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = many.into_iter().chain(single).map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
    out.sort();
    out.dedup();
    match out.iter().find(|l| !LEVELS.contains(&l.as_str())) {
        Some(bad) => Err(bad_request(format!("unknown level '{bad}'; use error, warning or info"))),
        None => Ok(out),
    }
}

impl Filter {
    fn from_query(q: &ErrorsQuery) -> Result<Self, ApiError> {
        let many = q.levels.as_deref().map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default();
        Ok(Filter {
            component: q.component.clone().filter(|c| !c.is_empty()),
            levels: parse_levels(q.level.clone(), many)?,
            source_id: q.source_id.map(Some),
            message: None,
            ids: None,
            dismissed: if q.include_dismissed { None } else { Some(false) },
        })
    }

    fn from_body(b: DismissBody, dismissed: bool) -> Result<Self, ApiError> {
        Ok(Filter {
            component: b.component.filter(|c| !c.is_empty()),
            levels: parse_levels(b.level, b.levels.unwrap_or_default())?,
            source_id: b.source_id,
            message: b.message,
            ids: b.ids,
            dismissed: Some(dismissed),
        })
    }

    /// ` WHERE …` (or empty) plus its parameters, in order.
    fn where_sql(&self) -> (String, Vec<SqlValue>) {
        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<SqlValue> = Vec::new();
        if let Some(c) = &self.component {
            clauses.push("component = ?".into());
            params.push(SqlValue::Text(c.clone()));
        }
        if !self.levels.is_empty() {
            clauses.push(format!("level IN ({})", crate::common::placeholders(self.levels.len())));
            params.extend(self.levels.iter().map(|l| SqlValue::Text(l.clone())));
        }
        match self.source_id {
            Some(Some(sid)) => {
                clauses.push("source_id = ?".into());
                params.push(SqlValue::Integer(sid));
            }
            Some(None) => clauses.push("source_id IS NULL".into()),
            None => {}
        }
        if let Some(m) = &self.message {
            clauses.push("message = ?".into());
            params.push(SqlValue::Text(m.clone()));
        }
        if let Some(ids) = &self.ids {
            clauses.push(format!("id IN ({})", crate::common::placeholders(ids.len().max(1))));
            params.extend(ids.iter().map(|i| SqlValue::Integer(*i)));
            if ids.is_empty() {
                params.push(SqlValue::Null);
            }
        }
        match self.dismissed {
            Some(true) => clauses.push("dismissed_at IS NOT NULL".into()),
            Some(false) => clauses.push("dismissed_at IS NULL".into()),
            None => {}
        }
        if clauses.is_empty() {
            (String::new(), params)
        } else {
            (format!(" WHERE {}", clauses.join(" AND ")), params)
        }
    }
}

fn db_err(e: rusqlite::Error) -> String {
    e.to_string()
}

fn source_name(conn: &Connection, source_id: Option<i64>) -> Option<String> {
    source_id.and_then(|sid| conn.query_row("SELECT name FROM sources WHERE id=?1", [sid], |r| r.get(0)).ok())
}

fn list_flat(conn: &Connection, f: &Filter, limit: i64) -> Result<Vec<Value>, String> {
    let (where_sql, mut params) = f.where_sql();
    let sql = format!(
        "SELECT id, component, level, source_id, message, COALESCE(detail, ''), created_at, dismissed_at FROM system_events{where_sql} \
         ORDER BY created_at DESC, id DESC LIMIT ?"
    );
    params.push(SqlValue::Integer(limit));
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            let source_id: Option<i64> = r.get(3)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "component": r.get::<_, String>(1)?, "level": r.get::<_, String>(2)?,
                "source_id": source_id, "source_name": source_name(conn, source_id),
                "message": r.get::<_, String>(4)?, "detail": r.get::<_, String>(5)?,
                "created_at": r.get::<_, String>(6)?, "dismissed_at": r.get::<_, Option<String>>(7)?,
            }))
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

/// Repeats fold into one row; `detail` is the newest occurrence's.
fn list_grouped(conn: &Connection, f: &Filter, limit: i64) -> Result<Vec<Value>, String> {
    let (where_sql, mut params) = f.where_sql();
    let sql = format!(
        "SELECT g.component, g.level, g.source_id, g.message, g.n, g.first_seen, g.last_seen, g.last_id, g.all_dismissed, \
         COALESCE((SELECT detail FROM system_events WHERE id = g.last_id), '') FROM ( \
           SELECT component, level, source_id, message, COUNT(*) AS n, MIN(created_at) AS first_seen, \
           MAX(created_at) AS last_seen, MAX(id) AS last_id, MIN(dismissed_at IS NOT NULL) AS all_dismissed \
           FROM system_events{where_sql} GROUP BY component, level, source_id, message) g \
         ORDER BY g.last_seen DESC, g.last_id DESC LIMIT ?"
    );
    params.push(SqlValue::Integer(limit));
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            let source_id: Option<i64> = r.get(2)?;
            Ok(json!({
                "component": r.get::<_, String>(0)?, "level": r.get::<_, String>(1)?,
                "source_id": source_id, "source_name": source_name(conn, source_id),
                "message": r.get::<_, String>(3)?, "count": r.get::<_, i64>(4)?,
                "first_seen": r.get::<_, String>(5)?, "last_seen": r.get::<_, String>(6)?,
                "id": r.get::<_, i64>(7)?, "dismissed": r.get::<_, i64>(8)? == 1, "detail": r.get::<_, String>(9)?,
            }))
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

/// GET /system/errors — newest first; dismissed rows only with
/// `include_dismissed=true`.
pub async fn errors_handler(State(state): State<AppState>, Query(q): Query<ErrorsQuery>) -> Result<Json<Value>, ApiError> {
    let filter = Filter::from_query(&q)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let group = q.group;
    run_db(&state.data_dir, move |conn| {
        if group {
            list_grouped(conn, &filter, limit)
        } else {
            list_flat(conn, &filter, limit)
        }
    })
    .await
    .map(|rows| Json(Value::Array(rows)))
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "detail": e }))))
}

fn count_by(conn: &Connection, column: &str, f: &Filter) -> Result<Vec<(Value, i64)>, String> {
    let (where_sql, params) = f.where_sql();
    let sql = format!("SELECT {column}, COUNT(*) FROM system_events{where_sql} GROUP BY {column} ORDER BY COUNT(*) DESC");
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            let key = match r.get::<_, SqlValue>(0)? {
                SqlValue::Integer(i) => json!(i),
                SqlValue::Text(t) => json!(t),
                _ => Value::Null,
            };
            Ok((key, r.get::<_, i64>(1)?))
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

fn summary(conn: &Connection, f: &Filter) -> Result<Value, String> {
    let mut levels = json!({ "error": 0, "warning": 0, "info": 0 });
    for (level, n) in count_by(conn, "level", f)? {
        if let Some(l) = level.as_str() {
            levels[l] = json!(n);
        }
    }
    let components: Vec<Value> =
        count_by(conn, "component", f)?.into_iter().map(|(c, n)| json!({ "component": c, "count": n })).collect();
    let sources: Vec<Value> = count_by(conn, "source_id", f)?
        .into_iter()
        .filter_map(|(sid, n)| sid.as_i64().map(|s| json!({ "source_id": s, "source_name": source_name(conn, Some(s)), "count": n })))
        .collect();
    let dismissed: i64 =
        conn.query_row("SELECT COUNT(*) FROM system_events WHERE dismissed_at IS NOT NULL", [], |r| r.get(0)).map_err(db_err)?;
    Ok(json!({ "levels": levels, "components": components, "sources": sources, "dismissed": dismissed }))
}

/// GET /system/errors/summary — counts per level, component and source for
/// the filter controls (shown rows unless `include_dismissed=true`), plus
/// the dismissed total. Level/component/source query keys are ignored so
/// every option keeps its count while another is selected.
pub async fn summary_handler(State(state): State<AppState>, Query(q): Query<ErrorsQuery>) -> Result<Json<Value>, ApiError> {
    let filter = Filter { dismissed: if q.include_dismissed { None } else { Some(false) }, ..Filter::default() };
    run_db(&state.data_dir, move |conn| summary(conn, &filter))
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "detail": e }))))
}

async fn set_dismissed(state: AppState, body: DismissBody, dismiss: bool) -> Result<Json<Value>, ApiError> {
    // Match the rows in the opposite state: dismiss shown ones, restore dismissed ones.
    let filter = Filter::from_body(body, !dismiss)?;
    let set = if dismiss { "datetime('now')" } else { "NULL" };
    let n = run_db(&state.data_dir, move |conn| {
        let (where_sql, params) = filter.where_sql();
        conn.execute(&format!("UPDATE system_events SET dismissed_at = {set}{where_sql}"), rusqlite::params_from_iter(params))
            .map_err(db_err)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "detail": e }))))?;
    let key = if dismiss { "dismissed" } else { "restored" };
    Ok(Json(json!({ key: n })))
}

/// POST /system/errors/dismiss — hide every shown event the body matches.
pub async fn dismiss_handler(State(state): State<AppState>, Json(body): Json<DismissBody>) -> Result<Json<Value>, ApiError> {
    set_dismissed(state, body, true).await
}

/// POST /system/errors/restore — show again every dismissed event the body matches.
pub async fn restore_handler(State(state): State<AppState>, Json(body): Json<DismissBody>) -> Result<Json<Value>, ApiError> {
    set_dismissed(state, body, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_merge_dedupe_and_reject_unknown() {
        assert_eq!(parse_levels(Some("Error".into()), vec!["warning".into(), "error".into()]).unwrap(), vec!["error", "warning"]);
        assert!(parse_levels(None, vec![]).unwrap().is_empty());
        assert!(parse_levels(Some("fatal".into()), vec![]).is_err());
    }

    #[test]
    fn empty_id_list_matches_nothing() {
        let f = Filter { ids: Some(vec![]), ..Filter::default() };
        let (sql, params) = f.where_sql();
        assert_eq!(sql, " WHERE id IN (?)");
        assert_eq!(params, vec![SqlValue::Null]);
    }
}
