//! R35.5: the one gate for every path that hands Sensitive bytes to
//! something other than the person at the UI.
//!
//! Caller rule (default closed): the SPA sends `X-Anchor-Client: ui` and is
//! never gated; every other caller (MCP proxy, skill, curl) is gated unless
//! the provider is trusted. Until R35.6's switches land, "trusted" is still
//! inferred from the answer provider (`answer::is_answer_trusted` — local or
//! allowlisted); the header only exempts the response to the UI, never bytes
//! leaving the machine toward a cloud model.
//!
//! Sensitivity is R35.4's effective rule (per-document override or the
//! `sensitive` source label), derived at the gate entry point. Single-item
//! byte reads refuse with 403 `refusal: "gated_provider"`; multi-item lists
//! drop the gated rows — both record one `answer::record_gate_event` audit
//! event per request. `public_only` scopes drop effective-Sensitive whatever
//! the caller or provider (a direct read 404s, so the share-safe surface
//! never confirms a sensitive item exists).
//!
//! Guardrail scope: this stops a well-behaved agent from leaking Sensitive
//! content by accident. It is not a boundary against a local process, which
//! can read the SQLite file directly.

use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};

pub type ApiError = (StatusCode, Json<Value>);

/// The SPA identifies itself on every `api.ts request()` (R35.5). Agents
/// (MCP, skill, curl) send no such header and stay gated. The value match is
/// exact but case-insensitive; anything else (absent included) is an agent.
pub const UI_CLIENT_HEADER: &str = "x-anchor-client";
pub const UI_CLIENT_VALUE: &str = "ui";

pub fn caller_is_ui(headers: &HeaderMap) -> bool {
    headers
        .get(UI_CLIENT_HEADER)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(UI_CLIENT_VALUE.as_bytes()))
}

/// Why a read was refused. `Gated` carries the label for the audit event,
/// which is async and must be recorded outside the blocking DB closure.
pub enum ReadRefusal {
    NotFound(&'static str),
    BadRequest(String),
    Gated(String),
}

/// One request's gate state: who is asking and whether the answer provider
/// may receive Sensitive content.
pub struct Gate {
    pub ui_caller: bool,
    pub provider_trusted: bool,
}

impl Gate {
    pub fn from_request(headers: &HeaderMap, settings: &crate::settings::SettingsService) -> Self {
        Self {
            ui_caller: caller_is_ui(headers),
            provider_trusted: crate::answer::is_answer_trusted(settings),
        }
    }

    /// The response path is open: the person at the UI always sees
    /// everything, and a trusted provider's callers flow until R35.6.
    pub fn open(&self) -> bool {
        self.ui_caller || self.provider_trusted
    }

    /// Byte gate for one item's effective Sensitive state.
    pub fn check_bytes(&self, label: &str, sensitive_override: bool) -> Result<(), ReadRefusal> {
        if self.open() {
            return Ok(());
        }
        match crate::common::cloud_gate_reason(label, sensitive_override) {
            Some(reason) => Err(ReadRefusal::Gated(reason)),
            None => Ok(()),
        }
    }
}

/// `public_only` scope: anything outside a public source — or an
/// effective-Sensitive item on one — reads as not found, whatever the caller
/// or provider (the UI's own public preview shows this same view).
pub fn check_public(label: &str, sensitive_override: bool, public_only: bool, not_found: &'static str) -> Result<(), ReadRefusal> {
    if public_only && (label != "public" || crate::common::effective_sensitive(label, sensitive_override)) {
        return Err(ReadRefusal::NotFound(not_found));
    }
    Ok(())
}

/// An item's source label (`internal` when the source row is missing) plus
/// its per-document override — the effective rule's two inputs.
pub fn item_sensitivity(conn: &rusqlite::Connection, item_id: i64) -> Option<(String, bool)> {
    conn.query_row(
        "SELECT COALESCE(s.label, 'internal'), COALESCE(i.sensitive_override, 0) \
         FROM ingested_items i LEFT JOIN sources s ON s.id = i.source_id WHERE i.id = ?1",
        [item_id],
        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? == 1)),
    )
    .ok()
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

    fn gate(ui_caller: bool, provider_trusted: bool) -> Gate {
        Gate { ui_caller, provider_trusted }
    }

    #[test]
    fn ui_header_match_is_exact_and_case_insensitive() {
        let mut ui = HeaderMap::new();
        ui.insert(UI_CLIENT_HEADER, "ui".parse().unwrap());
        assert!(caller_is_ui(&ui));
        let mut shout = HeaderMap::new();
        shout.insert(UI_CLIENT_HEADER, "UI".parse().unwrap());
        assert!(caller_is_ui(&shout), "value case folds");
        let mut agent = HeaderMap::new();
        agent.insert(UI_CLIENT_HEADER, "agent".parse().unwrap());
        assert!(!caller_is_ui(&agent));
        assert!(!caller_is_ui(&HeaderMap::new()), "absent header is an agent");
    }

    #[test]
    fn byte_gate_caller_rule() {
        // Agent + untrusted: effective-Sensitive refuses, the rest flows.
        let closed = gate(false, false);
        assert!(matches!(closed.check_bytes("sensitive", false), Err(ReadRefusal::Gated(_))));
        assert!(matches!(closed.check_bytes("internal", true), Err(ReadRefusal::Gated(l)) if l == crate::common::ITEM_SENSITIVE_MARK));
        assert!(closed.check_bytes("internal", false).is_ok());
        assert!(closed.check_bytes("public", false).is_ok());
        // The UI is never gated; trust still opens the agent path (R35.6).
        assert!(gate(true, false).check_bytes("sensitive", true).is_ok());
        assert!(gate(false, true).check_bytes("sensitive", true).is_ok());
    }

    #[test]
    fn public_scope_drops_sensitive_whatever_the_caller() {
        assert!(check_public("public", false, true, "x").is_ok());
        assert!(matches!(check_public("internal", false, true, "x"), Err(ReadRefusal::NotFound("x"))));
        assert!(matches!(check_public("public", true, true, "x"), Err(ReadRefusal::NotFound("x"))));
        assert!(check_public("sensitive", false, false, "x").is_ok(), "scope off changes nothing");
    }
}
