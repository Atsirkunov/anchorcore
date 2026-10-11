//! R20.9 — connect the shipped `anchorcore mcp` stdio mode into local MCP
//! clients (Claude Code, Claude Desktop, Cursor, opencode, Codex) from the
//! UI / onboarding wizard.
//!
//! Every write is behind an explicit button: config files are backed up and
//! merged (never clobbered), CLI-managed clients are driven through their own
//! `mcp add` command when the CLI is on PATH, and JSONC/hand-edited configs
//! are never rewritten (the UI falls back to a copy-paste snippet).

use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn claude_desktop_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var("APPDATA")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|a| PathBuf::from(a).join("Claude").join("claude_desktop_config.json"));
    }
    let home = home_dir()?;
    if cfg!(target_os = "macos") {
        return Some(home.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json"));
    }
    Some(home.join(".config").join("Claude").join("claude_desktop_config.json"))
}

fn cursor_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".cursor").join("mcp.json"))
}

fn opencode_path() -> Option<PathBuf> {
    // opencode V2 global config; prefers `opencode.json`, reports `opencode.jsonc`
    // when that is what exists (JSONC is never rewritten — the UI shows a snippet).
    let dir = home_dir()?.join(".config").join("opencode");
    let json = dir.join("opencode.json");
    if json.exists() {
        return Some(json);
    }
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        return Some(jsonc);
    }
    Some(json)
}

fn claude_code_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude.json"))
}

fn codex_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".codex").join("config.toml"))
}

fn cli_output(cmd: &str, args: &[&str]) -> Option<std::process::Output> {
    if let Ok(out) = std::process::Command::new(cmd).args(args).output() {
        return Some(out);
    }
    #[cfg(windows)]
    // npm-style shims (.cmd) need a shell; CreateProcess cannot run them directly.
    if let Ok(out) = std::process::Command::new("cmd").arg("/C").arg(cmd).args(args).output() {
        return Some(out);
    }
    None
}

fn cli_on_path(cmd: &str) -> bool {
    // Cheap detection for the wizard rows (no process spawn per poll).
    let Ok(path) = std::env::var("PATH") else { return false };
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let mut name = cmd.to_string();
            name.push_str(ext);
            if dir.join(&name).is_file() {
                return true;
            }
        }
    }
    false
}

fn cli_available(cmd: &str) -> bool {
    cli_output(cmd, &["--version"]).map(|o| o.status.success()).unwrap_or(false)
}

fn run_cli(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = cli_output(cmd, args).ok_or_else(|| format!("`{cmd}` not found on PATH"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if err.is_empty() { format!("exit {}", out.status) } else { err.chars().take(400).collect() };
    Err(format!("`{cmd}` failed: {err}"))
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{} is not valid JSON ({e}) — open it and add the snippet from the card manually", path.display()))
}

fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".anchorcore.bak");
    PathBuf::from(s)
}

fn backup_existing(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let b = backup_path(path);
    std::fs::copy(path, &b).map_err(|e| format!("backup {}: {e}", b.display()))?;
    Ok(Some(b))
}

fn write_text_with_backup(path: &Path, text: String) -> Result<Option<PathBuf>, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let backup = backup_existing(path)?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(backup)
}

fn write_json_with_backup(path: &Path, doc: &Value) -> Result<Option<PathBuf>, String> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    write_text_with_backup(path, format!("{text}\n"))
}

// --- Claude Desktop / Cursor / Claude Code user scope: `mcpServers.<name>` ---

fn server_entry(exe: &str) -> Value {
    json!({"command": exe, "args": ["mcp"]})
}

fn set_mcp_server(path: &Path, exe: &str) -> Result<Option<PathBuf>, String> {
    let mut doc = read_json(path)?.unwrap_or_else(|| json!({}));
    let obj = doc.as_object_mut().ok_or_else(|| format!("{}: top level is not a JSON object", path.display()))?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| format!("{}: mcpServers is not a JSON object", path.display()))?;
    servers.insert("anchorcore".to_string(), server_entry(exe));
    write_json_with_backup(path, &doc)
}

fn remove_mcp_server(path: &Path) -> Result<bool, String> {
    let Some(mut doc) = read_json(path)? else { return Ok(false) };
    let removed = doc.get_mut("mcpServers").and_then(|s| s.as_object_mut()).and_then(|s| s.remove("anchorcore")).is_some();
    if removed {
        write_json_with_backup(path, &doc)?;
    }
    Ok(removed)
}

fn json_server_connected(path: &Path, exe: &str) -> bool {
    let Ok(Some(doc)) = read_json(path) else { return false };
    doc.get("mcpServers")
        .and_then(|s| s.get("anchorcore"))
        .and_then(|v| v.get("command"))
        .and_then(|c| c.as_str())
        == Some(exe)
}

// --- opencode V2: `mcp.servers.<name>` with `type: local` + `command` array ---

fn set_opencode_server(path: &Path, exe: &str) -> Result<Option<PathBuf>, String> {
    let mut doc = read_json(path)?.unwrap_or_else(|| json!({}));
    let obj = doc.as_object_mut().ok_or_else(|| format!("{}: top level is not a JSON object", path.display()))?;
    let mcp = obj.entry("mcp").or_insert_with(|| json!({}));
    let mcp = mcp.as_object_mut().ok_or_else(|| format!("{}: mcp is not a JSON object", path.display()))?;
    let servers = mcp.entry("servers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| format!("{}: mcp.servers is not a JSON object", path.display()))?;
    servers.insert("anchorcore".to_string(), json!({"type": "local", "command": [exe, "mcp"]}));
    write_json_with_backup(path, &doc)
}

fn remove_opencode_server(path: &Path) -> Result<bool, String> {
    let Some(mut doc) = read_json(path)? else { return Ok(false) };
    let removed = doc
        .get_mut("mcp")
        .and_then(|m| m.get_mut("servers"))
        .and_then(|s| s.as_object_mut())
        .and_then(|s| s.remove("anchorcore"))
        .is_some();
    if removed {
        write_json_with_backup(path, &doc)?;
    }
    Ok(removed)
}

fn opencode_server_connected(path: &Path, exe: &str) -> bool {
    let Ok(Some(doc)) = read_json(path) else { return false };
    doc.get("mcp")
        .and_then(|m| m.get("servers"))
        .and_then(|s| s.get("anchorcore"))
        .and_then(|v| v.get("command"))
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        == Some(exe)
}

fn jsonc_guard(path: &Path) -> Result<(), String> {
    if path.extension().and_then(|e| e.to_str()) == Some("jsonc") {
        return Err(format!("{} has comments (JSONC) — open it and add/remove the snippet from the card manually", path.display()));
    }
    Ok(())
}

// --- Codex: `~/.codex/config.toml`, `[mcp_servers.anchorcore]` section ---

fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn codex_connected(text: &str, exe: &str) -> bool {
    text.contains("[mcp_servers.anchorcore]") && text.contains(&format!("command = \"{}\"", toml_escape(exe)))
}

fn set_codex_server(path: &Path, exe: &str) -> Result<Option<PathBuf>, String> {
    let existing = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?
    } else {
        String::new()
    };
    if existing.contains("[mcp_servers.anchorcore]") {
        return Err(format!("{} already has [mcp_servers.anchorcore] — remove it first (`codex mcp remove anchorcore`)", path.display()));
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("\n[mcp_servers.anchorcore]\ncommand = \"{}\"\nargs = [\"mcp\"]\n", toml_escape(exe)));
    write_text_with_backup(path, text)
}

fn remove_codex_section(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut kept: Vec<&str> = Vec::new();
    let mut in_section = false;
    let mut removed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[mcp_servers.anchorcore]" {
            in_section = true;
            removed = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            in_section = false;
        }
        if !in_section {
            kept.push(line);
        }
    }
    if removed {
        write_text_with_backup(path, format!("{}\n", kept.join("\n")))?;
    }
    Ok(removed)
}

// --- client plumbing ---

const CLIENT_IDS: [&str; 5] = ["claude-code", "claude-desktop", "cursor", "opencode", "codex"];

fn client_spec(id: &str) -> Option<(&'static str, Option<&'static str>, Option<PathBuf>)> {
    match id {
        "claude-code" => Some(("Claude Code", Some("claude"), claude_code_path())),
        "claude-desktop" => Some(("Claude Desktop", None, claude_desktop_path())),
        "cursor" => Some(("Cursor", None, cursor_path())),
        "opencode" => Some(("opencode", Some("opencode"), opencode_path())),
        "codex" => Some(("Codex", Some("codex"), codex_path())),
        _ => None,
    }
}

fn client_connected(id: &str, exe: &str) -> bool {
    match id {
        "claude-code" => claude_code_path().map(|p| json_server_connected(&p, exe)).unwrap_or(false),
        "claude-desktop" => claude_desktop_path().map(|p| json_server_connected(&p, exe)).unwrap_or(false),
        "cursor" => cursor_path().map(|p| json_server_connected(&p, exe)).unwrap_or(false),
        "opencode" => opencode_path().map(|p| opencode_server_connected(&p, exe)).unwrap_or(false),
        "codex" => codex_path()
            .map(|p| std::fs::read_to_string(&p).map(|t| codex_connected(&t, exe)).unwrap_or(false))
            .unwrap_or(false),
        _ => false,
    }
}

fn client_row(id: &str, exe: &str) -> Value {
    let Some((name, cli, file)) = client_spec(id) else {
        return json!({});
    };
    let cli_ok = cli.map(cli_on_path).unwrap_or(false);
    let file_ok = file.as_ref().map(|p| p.exists()).unwrap_or(false);
    let dir_ok = file.as_ref().and_then(|p| p.parent()).map(|d| d.exists()).unwrap_or(false);
    json!({
        "id": id,
        "name": name,
        "detected": cli_ok || file_ok || dir_ok,
        "method": if cli_ok { "cli" } else { "file" },
        "connected": client_connected(id, exe),
        "config_path": file.map(|p| p.to_string_lossy().to_string()),
    })
}

fn file_target(path: Option<PathBuf>) -> Result<PathBuf, String> {
    path.ok_or_else(|| "cannot resolve the config path (no home directory?)".to_string())
}

fn file_result(path: &Path, backup: Option<PathBuf>) -> Value {
    json!({
        "method": "file",
        "config_path": path.to_string_lossy(),
        "backup": backup.map(|b| b.to_string_lossy().to_string()),
    })
}

fn connect_claude_code(exe: &str) -> Result<Value, String> {
    if !cli_available("claude") {
        return Err("`claude` CLI not found on PATH — install Claude Code, or copy the command line from the card".to_string());
    }
    run_cli("claude", &["mcp", "add", "-s", "user", "anchorcore", "--", exe, "mcp"])?;
    Ok(json!({"method": "cli"}))
}

fn connect_mcp_servers_json(path: Option<PathBuf>, exe: &str) -> Result<Value, String> {
    let path = file_target(path)?;
    let backup = set_mcp_server(&path, exe)?;
    Ok(file_result(&path, backup))
}

fn connect_opencode(exe: &str) -> Result<Value, String> {
    if cli_available("opencode") {
        run_cli("opencode", &["mcp", "add", "anchorcore", "--global", "--", exe, "mcp"])?;
        return Ok(json!({"method": "cli"}));
    }
    let path = file_target(opencode_path())?;
    jsonc_guard(&path)?;
    Ok(file_result(&path, set_opencode_server(&path, exe)?))
}

fn connect_codex(exe: &str) -> Result<Value, String> {
    if cli_available("codex") {
        run_cli("codex", &["mcp", "add", "anchorcore", "--", exe, "mcp"])?;
        return Ok(json!({"method": "cli"}));
    }
    let path = file_target(codex_path())?;
    Ok(file_result(&path, set_codex_server(&path, exe)?))
}

fn connect_client(id: &str, exe: &str) -> Result<Value, String> {
    match id {
        "claude-code" => connect_claude_code(exe),
        "claude-desktop" => connect_mcp_servers_json(claude_desktop_path(), exe),
        "cursor" => connect_mcp_servers_json(cursor_path(), exe),
        "opencode" => connect_opencode(exe),
        "codex" => connect_codex(exe),
        _ => Err(format!("unknown client '{id}'")),
    }
}

fn disconnect_claude_code() -> Result<Value, String> {
    if !cli_available("claude") {
        return Err("`claude` CLI not found on PATH".to_string());
    }
    run_cli("claude", &["mcp", "remove", "-s", "user", "anchorcore"])?;
    Ok(json!({"method": "cli"}))
}

fn disconnect_mcp_servers_json(path: Option<PathBuf>) -> Result<Value, String> {
    let path = file_target(path)?;
    let removed = remove_mcp_server(&path)?;
    Ok(json!({"method": "file", "removed": removed, "config_path": path.to_string_lossy()}))
}

fn disconnect_opencode() -> Result<Value, String> {
    let path = file_target(opencode_path())?;
    jsonc_guard(&path)?;
    Ok(json!({"method": "file", "removed": remove_opencode_server(&path)?, "config_path": path.to_string_lossy()}))
}

fn disconnect_codex() -> Result<Value, String> {
    if cli_available("codex") {
        run_cli("codex", &["mcp", "remove", "anchorcore"])?;
        return Ok(json!({"method": "cli"}));
    }
    let path = file_target(codex_path())?;
    Ok(json!({"method": "file", "removed": remove_codex_section(&path)?, "config_path": path.to_string_lossy()}))
}

fn disconnect_client(id: &str, _exe: &str) -> Result<Value, String> {
    match id {
        "claude-code" => disconnect_claude_code(),
        "claude-desktop" => disconnect_mcp_servers_json(claude_desktop_path()),
        "cursor" => disconnect_mcp_servers_json(cursor_path()),
        "opencode" => disconnect_opencode(),
        "codex" => disconnect_codex(),
        _ => Err(format!("unknown client '{id}'")),
    }
}

fn current_exe_str() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("cannot resolve this executable's path: {e}"))
}

/// Best-effort: turn a result into the `{ok, ...}` JSON the UI expects.
fn finish(result: Result<Value, String>) -> Value {
    match result {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("ok".to_string(), json!(true));
            }
            v
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

#[derive(Deserialize)]
pub struct ClientRequest {
    pub client: String,
}

/// GET /system/mcp/clients — detection + connection state for the wizard card.
pub async fn clients_handler() -> Json<Value> {
    let result = tokio::task::spawn_blocking(|| {
        let exe = current_exe_str().unwrap_or_default();
        let mut clients: Vec<Value> = Vec::new();
        for id in CLIENT_IDS {
            clients.push(client_row(id, &exe));
        }
        json!({"clients": clients})
    })
    .await
    .unwrap();
    Json(result)
}

/// POST /system/mcp/connect — explicit user action from the UI only.
pub async fn connect_handler(Json(req): Json<ClientRequest>) -> Json<Value> {
    let client = req.client;
    let result = tokio::task::spawn_blocking(move || {
        let exe = current_exe_str()?;
        connect_client(&client, &exe)
    })
    .await
    .unwrap();
    Json(finish(result))
}

/// POST /system/mcp/disconnect — removes only the `anchorcore` entry.
pub async fn disconnect_handler(Json(req): Json<ClientRequest>) -> Json<Value> {
    let client = req.client;
    let result = tokio::task::spawn_blocking(move || {
        let exe = current_exe_str()?;
        disconnect_client(&client, &exe)
    })
    .await
    .unwrap();
    Json(finish(result))
}
