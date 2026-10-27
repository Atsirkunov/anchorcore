//! R36.15: shared scope + label gate for the agent read surfaces that
//! return bytes by id (`GET /items/:id?lines=`, `GET /nodes/:id?include=text`)
//! and the `public_only` scope on item/outline/node reads.
//!
//! Same rules as `/qa/search` and `/media/:uid` for the same caller:
//! - `public_only=true` serves only items of `public`-labelled sources;
//!   anything else reads as 404 (the share-safe surface never confirms that
//!   a non-public item exists).
//! - under an untrusted answer provider, bytes of a `sensitive`/`pii`
//!   source are refused with 403 `refusal: "gated_provider"` and a gate
//!   event (`answer::record_gate_event`), like `/media`.

use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

pub type ApiError = (StatusCode, Json<Value>);

/// Why a read was refused. `Gated` carries the label for the audit event,
/// which is async and must be recorded outside the blocking DB closure.
pub enum ReadRefusal {
    NotFound(&'static str),
    BadRequest(String),
    Gated(String),
}

pub fn gated_label(label: &str) -> bool {
    label == "sensitive" || label == "pii"
}

/// Source label of an item (`internal` when the source row is missing).
pub fn item_label(conn: &rusqlite::Connection, item_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT COALESCE(s.label, 'internal') FROM ingested_items i LEFT JOIN sources s ON s.id = i.source_id WHERE i.id = ?1",
        [item_id],
        |r| r.get(0),
    )
    .ok()
}

/// `public_only` scope: a non-public item reads as not found.
pub fn check_public(label: &str, public_only: bool, not_found: &'static str) -> Result<(), ReadRefusal> {
    if public_only && label != "public" {
        return Err(ReadRefusal::NotFound(not_found));
    }
    Ok(())
}

/// Byte gate: Sensitive bytes never go to an untrusted provider's agent.
pub fn check_bytes(label: &str, trusted: bool) -> Result<(), ReadRefusal> {
    if !trusted && gated_label(label) {
        return Err(ReadRefusal::Gated(label.to_string()));
    }
    Ok(())
}

/// Refusal → HTTP error (`{"detail": …}`), recording the gate event first.
pub async fn into_api_error(data_dir: &str, refusal: ReadRefusal) -> ApiError {
    match refusal {
        ReadRefusal::NotFound(detail) => (StatusCode::NOT_FOUND, Json(json!({"detail": detail}))),
        ReadRefusal::BadRequest(detail) => (StatusCode::BAD_REQUEST, Json(json!({"detail": detail}))),
        ReadRefusal::Gated(label) => {
            crate::answer::record_gate_event(data_dir, std::slice::from_ref(&label)).await;
            let body = json!({"detail": "Content withheld from an untrusted answer provider (Sensitive source)", "refusal": "gated_provider"});
            (StatusCode::FORBIDDEN, Json(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_byte_gate_rules() {
        assert!(check_public("public", true, "x").is_ok());
        assert!(check_public("internal", false, "x").is_ok());
        assert!(matches!(check_public("internal", true, "x"), Err(ReadRefusal::NotFound("x"))));
        assert!(check_bytes("sensitive", true).is_ok(), "trusted provider sees everything");
        assert!(check_bytes("internal", false).is_ok());
        assert!(matches!(check_bytes("pii", false), Err(ReadRefusal::Gated(l)) if l == "pii"));
        assert!(matches!(check_bytes("sensitive", false), Err(ReadRefusal::Gated(_))));
    }
}
