//! R35.6: the person's opt-in for Sensitive content, per destination.
//!
//! Three destinations can receive effective-Sensitive bytes (R35.4's one
//! rule): the cloud embedder, the cloud answer provider, and AI agents
//! connected to this app (MCP / the REST skill). Each is closed by default
//! and opens for exactly one of:
//!
//! - **local** — a loopback provider or the built-in embedder (never leaves
//!   the machine; agents are never local);
//! - **environment** — the admin overrides `ANCHOR_CLOUD_TRUST=1` (every
//!   destination) or `ANCHOR_TRUSTED_PROVIDERS` (embed/answer hosts);
//! - **switch** — the person's Settings switch. Provider switches are bound to
//!   the destination they named: the stored value is the provider origin at
//!   opt-in time, so pointing the embedder or answer model at another host
//!   closes the gate until the person allows the new host.
//!
//! `allowed` is the one rule; every gate (ingest `embed_chunks`, re-embed
//! `fetch_embed_batch`, backfill SQL, answer generation, `read_gate::Gate`,
//! ask/search responses) reads it through `Embedder::cloud_trusted`,
//! `answer::is_answer_trusted` or `read_gate::Gate`. Switches live in
//! `app_settings` (no env fallback, not in `PUT /settings`, never exported in
//! backups) and change only through `PUT /settings/sensitive-sharing`, which
//! only the UI may call and which records an audit event per change.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Map, Value};

use crate::health::AppState;
use crate::settings::SettingsService;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Embed,
    Answer,
    Agents,
}

/// `app_settings` keys of the three switches (empty/absent = off).
pub const SHARING_KEYS: &[&str] = &["sensitive_share_embed", "sensitive_share_answer", "sensitive_share_agents"];
const AGENTS_ON: &str = "1";
pub const AGENTS_DESTINATION: &str = "AI agents connected to this app (MCP / skill)";

impl Destination {
    pub const ALL: [Destination; 3] = [Destination::Embed, Destination::Answer, Destination::Agents];

    pub fn id(self) -> &'static str {
        match self {
            Destination::Embed => "embed",
            Destination::Answer => "answer",
            Destination::Agents => "agents",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Destination::Embed => SHARING_KEYS[0],
            Destination::Answer => SHARING_KEYS[1],
            Destination::Agents => SHARING_KEYS[2],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Destination::Embed => "Cloud embedder",
            Destination::Answer => "Cloud answer provider",
            Destination::Agents => "AI agents (MCP / skill)",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id() == id)
    }
}

/// The HTTP embedder's base URL (`embed_base_url`, else Ollama's).
pub fn embed_base_url(settings: &SettingsService) -> String {
    settings
        .get("embed_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string())
}

/// The answer provider's base URL (unset = the OpenAI default, untrusted).
pub fn answer_base_url(settings: &SettingsService) -> String {
    settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string())
}

/// `scheme://host[:port]` of a base URL, lowercased, without userinfo or
/// path — the destination a provider switch names and is bound to.
pub fn origin(base_url: &str) -> String {
    let base = base_url.trim();
    let (scheme, rest) = base.split_once("://").unwrap_or(("", base));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority.rsplit('@').next().unwrap_or("");
    if scheme.is_empty() {
        return hostport.to_ascii_lowercase();
    }
    format!("{}://{}", scheme.to_ascii_lowercase(), hostport.to_ascii_lowercase())
}

/// What decides one destination, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationState {
    pub dest: Destination,
    /// The origin a provider switch is bound to (empty for agents and the
    /// built-in embedder).
    pub target: String,
    /// Loopback provider or the built-in embedder — trusted without a switch.
    pub local: bool,
    /// `ANCHOR_CLOUD_TRUST=1` or `ANCHOR_TRUSTED_PROVIDERS` lists the host.
    pub env: bool,
    /// The person's switch is on for this destination.
    pub switch_on: bool,
    /// A switch left on for a host that is no longer configured.
    pub stale_for: Option<String>,
}

impl DestinationState {
    pub fn allowed(&self) -> bool {
        self.local || self.env || self.switch_on
    }

    /// Human name of the destination the switch allows.
    pub fn destination(&self) -> String {
        match self.dest {
            Destination::Agents => AGENTS_DESTINATION.to_string(),
            Destination::Embed if self.target.is_empty() => "the built-in model on this computer".to_string(),
            _ => self.target.clone(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.dest.id(),
            "label": self.dest.label(),
            "destination": self.destination(),
            "local": self.local,
            "env": self.env,
            "switch": self.switch_on,
            "allowed": self.allowed(),
            "stale_for": self.stale_for,
        })
    }
}

/// One destination's state from settings + environment.
pub fn state(settings: &SettingsService, dest: Destination) -> DestinationState {
    let stored = settings.get(dest.key(), None);
    if dest == Destination::Agents {
        return DestinationState {
            dest,
            target: String::new(),
            local: false,
            env: crate::common::env_cloud_trust(),
            switch_on: stored.as_deref() == Some(AGENTS_ON),
            stale_for: None,
        };
    }
    let builtin = dest == Destination::Embed && crate::embedder::active_provider_for(settings) == crate::embedder::Provider::Builtin;
    let base = if dest == Destination::Embed { embed_base_url(settings) } else { answer_base_url(settings) };
    let target = if builtin { String::new() } else { origin(&base) };
    let switch_on = !target.is_empty() && stored.as_deref() == Some(target.as_str());
    DestinationState {
        dest,
        local: builtin || crate::common::provider_is_local(&base),
        env: crate::common::provider_env_trusted(&base),
        switch_on,
        stale_for: stored.filter(|_| !switch_on),
        target,
    }
}

/// The one rule: may effective-Sensitive content reach `dest`?
pub fn allowed(settings: &SettingsService, dest: Destination) -> bool {
    state(settings, dest).allowed()
}

/// Full card state: `{embed, answer, agents}`.
pub fn snapshot(settings: &SettingsService) -> Value {
    let mut out = Map::new();
    for dest in Destination::ALL {
        out.insert(dest.id().to_string(), state(settings, dest).to_json());
    }
    Value::Object(out)
}

type ApiError = (StatusCode, Json<Value>);

fn unprocessable(detail: String) -> ApiError {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": detail})))
}

/// Parse `{embed?, answer?, agents?}: bool` and refuse a switch that has
/// nothing to allow (local or built-in destination).
fn parse_changes(settings: &SettingsService, body: &Value) -> Result<Vec<(DestinationState, bool)>, ApiError> {
    let Some(obj) = body.as_object() else {
        return Err(unprocessable("expected an object like {\"agents\": true}".to_string()));
    };
    let mut changes = Vec::new();
    for (k, v) in obj {
        let Some(dest) = Destination::from_id(k) else {
            return Err(unprocessable(format!("unknown destination '{k}'; use embed, answer or agents")));
        };
        let Some(on) = v.as_bool() else {
            return Err(unprocessable(format!("{k} must be true or false")));
        };
        let st = state(settings, dest);
        if on && st.target.is_empty() && dest != Destination::Agents {
            return Err(unprocessable(format!("{k}: the destination is the built-in model on this computer — nothing to allow")));
        }
        if on && st.local {
            return Err(unprocessable(format!("{k}: {} is local — Sensitive content already flows there", st.target)));
        }
        changes.push((st, on));
    }
    Ok(changes)
}

/// Store one switch and record its audit event. Turning a switch on is a
/// `warning` (Sensitive content may now leave the app's own view); off is
/// `info`.
fn apply_change(settings: &SettingsService, conn: &rusqlite::Connection, st: &DestinationState, on: bool) -> Result<(), String> {
    let value = match (on, st.dest) {
        (false, _) => String::new(),
        (true, Destination::Agents) => AGENTS_ON.to_string(),
        (true, _) => st.target.clone(),
    };
    settings.set(st.dest.key(), &value, conn)?;
    let (level, verb) = if on { ("warning", "on") } else { ("info", "off") };
    let msg = format!("Sensitive sharing {verb}: {} — {}", st.dest.label(), st.destination());
    let detail = format!("switch={} by the person at the UI", st.dest.key());
    conn.execute(
        "INSERT INTO system_events (component, level, message, detail) VALUES ('settings', ?1, ?2, ?3)",
        rusqlite::params![level, msg, detail],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_handler(State(state): State<AppState>) -> Json<Value> {
    Json(snapshot(&state.settings))
}

/// `PUT /settings/sensitive-sharing` — UI only: an agent must never opt
/// itself in. Validates every change before the first write.
pub async fn put_handler(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    if !crate::read_gate::caller_is_ui(&headers) {
        let detail = "Only the person at the AnchorCore app can change Sensitive sharing (Settings → Sensitive sources)";
        return Err((StatusCode::FORBIDDEN, Json(json!({"detail": detail}))));
    }
    let changes = parse_changes(&state.settings, &body)?;
    let settings = state.settings.clone();
    let embed_on = changes.iter().any(|(st, on)| *on && st.dest == Destination::Embed);
    crate::pipeline::run_db(&state.data_dir, move |conn| {
        changes.iter().try_for_each(|(st, on)| apply_change(&settings, conn, st, *on))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e}))))?;
    if embed_on {
        // Chunks the gate held back are now eligible: wake the backfill.
        state.embedder.chunks_pending.notify_one();
    }
    Ok(Json(snapshot(&state.settings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(dir: &tempfile::TempDir) -> (std::sync::Arc<SettingsService>, rusqlite::Connection) {
        let conn = crate::db::init_db(&crate::db::resolve_db_path(&dir.path().to_string_lossy())).unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let s = std::sync::Arc::new(SettingsService::new(store));
        s.set_data_dir(dir.path());
        (s, conn)
    }

    struct CleanEnv {
        trust: Option<String>,
        allow: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl CleanEnv {
        fn new() -> Self {
            let lock = crate::common::TRUST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let trust = std::env::var("ANCHOR_CLOUD_TRUST").ok();
            let allow = std::env::var("ANCHOR_TRUSTED_PROVIDERS").ok();
            std::env::remove_var("ANCHOR_CLOUD_TRUST");
            std::env::remove_var("ANCHOR_TRUSTED_PROVIDERS");
            Self { trust, allow, _lock: lock }
        }
    }

    impl Drop for CleanEnv {
        fn drop(&mut self) {
            for (k, v) in [("ANCHOR_CLOUD_TRUST", &self.trust), ("ANCHOR_TRUSTED_PROVIDERS", &self.allow)] {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn origin_drops_path_userinfo_and_case() {
        assert_eq!(origin("https://API.OpenAI.com/v1/"), "https://api.openai.com");
        assert_eq!(origin("http://user:pw@llm.corp:8080/v1"), "http://llm.corp:8080");
        assert_eq!(origin(" https://x.example?y=1 "), "https://x.example");
    }

    #[test]
    fn closed_by_default_and_local_needs_no_switch() {
        let _env = CleanEnv::new();
        let dir = tempfile::tempdir().unwrap();
        let (s, conn) = svc(&dir);
        s.set("answer_base_url", "https://api.openai.com/v1", &conn).unwrap();
        s.set("embed_base_url", "https://embed.example.com/v1", &conn).unwrap();
        for dest in Destination::ALL {
            assert!(!allowed(&s, dest), "{dest:?} closed by default");
        }
        s.set("answer_base_url", "http://localhost:11434/v1", &conn).unwrap();
        s.set("embed_base_url", "http://127.0.0.1:11434", &conn).unwrap();
        assert!(allowed(&s, Destination::Answer) && allowed(&s, Destination::Embed));
        assert!(!allowed(&s, Destination::Agents), "agent trust is never inferred from the answer provider");
    }

    #[test]
    fn provider_switch_is_bound_to_its_host() {
        let _env = CleanEnv::new();
        let dir = tempfile::tempdir().unwrap();
        let (s, conn) = svc(&dir);
        s.set("answer_base_url", "https://api.openai.com/v1", &conn).unwrap();
        s.set("embed_base_url", "https://embed.example.com/v1", &conn).unwrap();
        let st = state(&s, Destination::Answer);
        apply_change(&s, &conn, &st, true).unwrap();
        assert!(allowed(&s, Destination::Answer));
        assert!(!allowed(&s, Destination::Agents), "switches are independent");
        assert!(!allowed(&s, Destination::Embed));
        // Another host closes the gate and reports the stale consent.
        s.set("answer_base_url", "https://llm.other.example/v1", &conn).unwrap();
        let moved = state(&s, Destination::Answer);
        assert!(!moved.allowed());
        assert_eq!(moved.stale_for.as_deref(), Some("https://api.openai.com"));
        let events: i64 = conn.query_row("SELECT COUNT(*) FROM system_events WHERE component='settings' AND level='warning' AND message LIKE 'Sensitive sharing on:%'", [], |r| r.get(0)).unwrap();
        assert_eq!(events, 1, "turning a switch on is audited");
    }

    #[test]
    fn agents_switch_and_env_overrides() {
        let _env = CleanEnv::new();
        let dir = tempfile::tempdir().unwrap();
        let (s, conn) = svc(&dir);
        s.set("answer_base_url", "https://api.openai.com/v1", &conn).unwrap();
        s.set("embed_base_url", "https://embed.example.com/v1", &conn).unwrap();
        apply_change(&s, &conn, &state(&s, Destination::Agents), true).unwrap();
        assert!(allowed(&s, Destination::Agents));
        assert!(!allowed(&s, Destination::Answer), "the agents switch never opens a provider");
        apply_change(&s, &conn, &state(&s, Destination::Agents), false).unwrap();
        assert!(!allowed(&s, Destination::Agents));

        std::env::set_var("ANCHOR_TRUSTED_PROVIDERS", "https://embed.example.com");
        let embed = state(&s, Destination::Embed);
        assert!(embed.env && embed.allowed() && !embed.switch_on, "allowlist shows as set by environment");
        assert!(!allowed(&s, Destination::Agents), "the provider allowlist never opens agents");
        std::env::set_var("ANCHOR_CLOUD_TRUST", "1");
        for dest in Destination::ALL {
            assert!(state(&s, dest).env, "{dest:?}: ANCHOR_CLOUD_TRUST opens every destination");
        }
    }

    #[test]
    fn switch_keys_ignore_env_and_generic_settings() {
        let _env = CleanEnv::new();
        let dir = tempfile::tempdir().unwrap();
        let (s, _conn) = svc(&dir);
        std::env::set_var("ANCHOR_SENSITIVE_SHARE_AGENTS", "1");
        let on = allowed(&s, Destination::Agents);
        std::env::remove_var("ANCHOR_SENSITIVE_SHARE_AGENTS");
        assert!(!on, "a switch is never set by a same-named env var");
        assert!(!crate::settings::ALL_KEYS.iter().any(|k| SHARING_KEYS.contains(k)), "PUT /settings cannot flip a switch");
        assert!(s.snapshot(None).get("sensitive_share_agents").is_none());
    }

    #[test]
    fn local_destinations_refuse_a_switch() {
        let _env = CleanEnv::new();
        let dir = tempfile::tempdir().unwrap();
        let (s, conn) = svc(&dir);
        s.set("answer_base_url", "http://localhost:11434/v1", &conn).unwrap();
        assert!(parse_changes(&s, &json!({"answer": true})).is_err());
        assert!(parse_changes(&s, &json!({"answer": false})).is_ok(), "off is always accepted");
        assert!(parse_changes(&s, &json!({"bogus": true})).is_err());
        assert!(parse_changes(&s, &json!({"agents": "yes"})).is_err());
        assert!(parse_changes(&s, &json!({"agents": true})).is_ok());
    }
}
