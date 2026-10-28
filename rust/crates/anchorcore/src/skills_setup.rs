//! R23.7 — install the served agent skills (`setup`, `search`) into local
//! agent clients' skills directories from the System-tab skill section.
//!
//! This is the blocked-fallback path behind MCP-first routing: the UI only
//! offers Install when no MCP client is connected — never pushed alongside
//! a working MCP. Every write is behind an explicit per-skill Install button
//! plus a confirm step stating the target path.
//!
//! Structure mirrors `mcp_setup.rs`: a per-client skills-dir matrix, backup +
//! never-clobber writes (`SKILL.md.anchorcore.bak` siblings), and `{ok, ...}`
//! JSON the UI expects. Targets are built from the allowlist only — the
//! request carries a client id + skill name, never a path — so a write can
//! never land outside the matrix.
//!
//! R47.4: the search skill installs as `ask-docs` by default and takes an
//! optional `handle` (its folder and `name:`), validated to one folder name.
//! Copies are recognised by the body marker (or the pre-marker
//! `anchorcore-search` name), so an install under a new handle retires the
//! old copy instead of leaving two skills that both trigger.

use axum::extract::Path as UrlPath;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// Same embedded bytes the R23.6 `GET /system/skills/:name` endpoint serves
// (duplicated `include_str!` — `system.rs` owns the serving copy; this module
// must not depend on its privates).
const SETUP_SKILL_MD: &str = include_str!("../../../../.agents/skills/anchorcore-setup/SKILL.md");
const SEARCH_SKILL_MD: &str = include_str!("../../../../.agents/skills/ask-docs/SKILL.md");

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn join_many(home: &Path, parts: &[&str]) -> PathBuf {
    let mut p = home.to_path_buf();
    for part in parts {
        p = p.join(part);
    }
    p
}

// --- per-client skills-dir matrix (the allowlist) ---

const CLIENT_IDS: [&str; 5] = ["claude-code", "claude-desktop", "cursor", "opencode", "codex"];

/// Home-relative skills root per client. `claude-desktop` mirrors the
/// `mcp_setup.rs` config-path platform split; the rest are the clients'
/// documented user-level skills dirs.
fn skills_subpath(id: &str) -> Option<Vec<&'static str>> {
    match id {
        "claude-code" => Some(vec![".claude", "skills"]),
        "claude-desktop" => {
            if cfg!(windows) {
                None // handled via APPDATA below
            } else if cfg!(target_os = "macos") {
                Some(vec!["Library", "Application Support", "Claude", "skills"])
            } else {
                Some(vec![".config", "Claude", "skills"])
            }
        }
        "cursor" => Some(vec![".cursor", "skills"]),
        "opencode" => Some(vec![".config", "opencode", "skills"]),
        "codex" => Some(vec![".codex", "skills"]),
        _ => None,
    }
}

fn claude_desktop_skills_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var("APPDATA")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|a| PathBuf::from(a).join("Claude").join("skills"));
    }
    let home = home_dir()?;
    skills_subpath("claude-desktop").map(|parts| join_many(&home, &parts))
}

fn skills_root_for(id: &str) -> Option<PathBuf> {
    if id == "claude-desktop" {
        return claude_desktop_skills_dir();
    }
    let home = home_dir()?;
    skills_subpath(id).map(|parts| join_many(&home, &parts))
}

fn client_name(id: &str) -> Option<&'static str> {
    match id {
        "claude-code" => Some("Claude Code"),
        "claude-desktop" => Some("Claude Desktop"),
        "cursor" => Some("Cursor"),
        "opencode" => Some("opencode"),
        "codex" => Some("Codex"),
        _ => None,
    }
}

/// Presence check for the UI badge: the skills dir itself, or the client's
/// home dot-dir (a client install with no skills yet still counts).
fn client_home_marker(id: &str) -> Option<Vec<&'static str>> {
    match id {
        "claude-code" => Some(vec![".claude"]),
        "claude-desktop" => {
            if cfg!(windows) {
                None
            } else if cfg!(target_os = "macos") {
                Some(vec!["Library", "Application Support", "Claude"])
            } else {
                Some(vec![".config", "Claude"])
            }
        }
        "cursor" => Some(vec![".cursor"]),
        "opencode" => Some(vec![".config", "opencode"]),
        "codex" => Some(vec![".codex"]),
        _ => None,
    }
}

fn client_detected(id: &str) -> bool {
    if skills_root_for(id).map(|p| p.exists()).unwrap_or(false) {
        return true;
    }
    if id == "claude-desktop" && cfg!(windows) {
        return std::env::var("APPDATA")
            .ok()
            .map(|a| PathBuf::from(a).join("Claude").exists())
            .unwrap_or(false);
    }
    home_dir()
        .zip(client_home_marker(id))
        .map(|(home, parts)| join_many(&home, &parts).exists())
        .unwrap_or(false)
}

// --- skills ---

const SKILL_IDS: [&str; 2] = ["setup", "search"];

/// Canonical on-disk skill dir (`anchorcore-*`, matching the repo
/// `.agents/skills/` names). Anything else — including `..` — is rejected,
/// so the skill name can never escape the target dir.
fn canonical_skill_dir(name: &str) -> Option<&'static str> {
    match name.trim() {
        "setup" | "anchorcore-setup" => Some("anchorcore-setup"),
        "search" | "ask-docs" | "anchorcore-search" => Some("ask-docs"),
        _ => None,
    }
}

fn skill_bytes(name: &str) -> Option<&'static str> {
    match canonical_skill_dir(name)? {
        "anchorcore-setup" => Some(SETUP_SKILL_MD),
        "ask-docs" => Some(SEARCH_SKILL_MD),
        _ => None,
    }
}

/// The only path constructor: allowlist root + allowlist dir + fixed
/// filename. No request-controlled path component survives validation.
fn install_target_path(client: &str, skill: &str) -> Result<PathBuf, String> {
    let root = skills_root_for(client).ok_or_else(|| format!("unknown client '{client}'"))?;
    let dir = canonical_skill_dir(skill).ok_or_else(|| format!("Unknown skill '{skill}' (setup, search)"))?;
    Ok(root.join(dir).join("SKILL.md"))
}

// --- search-skill handle (R47.4) ---

const SEARCH_DIR: &str = "ask-docs";
const SEARCH_MARKER: &str = "<!-- anchorcore-skill: search";
/// `name:` values older builds installed the search skill under (no marker).
const LEGACY_SEARCH_NAMES: [&str; 1] = ["anchorcore-search"];

fn is_search_skill(name: &str) -> bool {
    canonical_skill_dir(name) == Some(SEARCH_DIR)
}

/// A handle becomes a folder and the skill's `name:`: lowercase letters,
/// digits and inner hyphens, at most 64 chars (the agents' own rule), so it
/// can never be a path. The setup skill's folder is taken.
fn validate_handle(raw: &str) -> Result<String, String> {
    let h = raw.trim();
    let shape_ok = !h.is_empty()
        && h.len() <= 64
        && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !h.starts_with('-')
        && !h.ends_with('-');
    if !shape_ok {
        return Err(format!("Handle '{h}' must be 1-64 lowercase letters, digits or inner hyphens (like ask-docs)"));
    }
    if h == "anchorcore-setup" {
        return Err("Handle 'anchorcore-setup' is the setup skill's folder — pick another".to_string());
    }
    Ok(h.to_string())
}

fn frontmatter_name(content: &str) -> Option<&str> {
    let rest = content.strip_prefix("---")?;
    let block = &rest[..rest.find("\n---")?];
    block.lines().find_map(|l| l.strip_prefix("name:")).map(str::trim)
}

/// True when SKILL.md content is a copy of our search skill, whatever its
/// folder is called.
fn is_our_search_copy(content: &str) -> bool {
    content.contains(SEARCH_MARKER) || frontmatter_name(content).is_some_and(|n| LEGACY_SEARCH_NAMES.contains(&n))
}

/// The served bytes with the handle as the skill's name and in its
/// invocation example; the default handle returns them unchanged.
fn personalize_search(handle: &str) -> String {
    if handle == SEARCH_DIR {
        return SEARCH_SKILL_MD.to_string();
    }
    SEARCH_SKILL_MD
        .replacen(&format!("\nname: {SEARCH_DIR}\n"), &format!("\nname: {handle}\n"), 1)
        .replace(&format!("`/{SEARCH_DIR} "), &format!("`/{handle} "))
}

/// Folders under `root` holding a copy of the search skill.
fn installed_search_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|d| std::fs::read_to_string(d.join("SKILL.md")).is_ok_and(|c| is_our_search_copy(&c)))
        .collect();
    dirs.sort();
    dirs
}

/// Install under `root/<handle>/`, then retire every other copy (its
/// SKILL.md becomes the `.anchorcore.bak` sibling, so the agent stops
/// loading it but nothing is deleted).
fn install_search_at(root: &Path, handle: &str) -> Result<(PathBuf, InstallOutcome, Vec<PathBuf>), String> {
    let path = root.join(handle).join("SKILL.md");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if !is_our_search_copy(&existing) {
            return Err(format!("'{handle}' already holds a different skill ({}) — pick another handle", path.display()));
        }
    }
    let outcome = install_skill_file(&path, &personalize_search(handle))?;
    let mut retired = Vec::new();
    for dir in installed_search_dirs(root).into_iter().filter(|d| d.file_name() != Some(std::ffi::OsStr::new(handle))) {
        let old = dir.join("SKILL.md");
        let bak = backup_path(&old);
        std::fs::rename(&old, &bak).map_err(|e| format!("retire {}: {e}", old.display()))?;
        retired.push(bak);
    }
    Ok((path, outcome, retired))
}

// --- never-clobber write ---

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

enum InstallOutcome {
    Written { backup: Option<PathBuf> },
    AlreadyInstalled,
}

fn install_skill_file(path: &Path, bytes: &str) -> Result<InstallOutcome, String> {
    if path.exists() {
        let existing = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if existing == bytes {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        let backup = backup_existing(path)?;
        std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
        return Ok(InstallOutcome::Written { backup });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(InstallOutcome::Written { backup: None })
}

// --- handlers ---

fn client_row(id: &str) -> Value {
    let Some(name) = client_name(id) else { return json!({}) };
    let dir = skills_root_for(id);
    // R47.4: handles the search skill is installed under (the UI defaults to it).
    let search_handles: Vec<String> = dir
        .as_deref()
        .map(installed_search_dirs)
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    json!({
        "id": id,
        "name": name,
        "detected": client_detected(id),
        "skills_dir": dir.map(|p| p.to_string_lossy().to_string()),
        "search_handles": search_handles,
    })
}

/// GET /system/skills/install-targets — skills + per-client target dirs for
/// the Install confirm dialog. Pure data, no writes.
pub async fn install_targets_handler() -> Json<Value> {
    let result = tokio::task::spawn_blocking(|| {
        let skills: Vec<Value> = SKILL_IDS
            .iter()
            .map(|id| {
                let dir = canonical_skill_dir(id).unwrap_or(*id);
                json!({"name": id, "dir": dir, "file": "SKILL.md", "renamable": is_search_skill(id)})
            })
            .collect();
        let mut clients: Vec<Value> = Vec::new();
        for id in CLIENT_IDS {
            clients.push(client_row(id));
        }
        json!({"skills": skills, "clients": clients})
    })
    .await
    .unwrap();
    Json(result)
}

#[derive(Deserialize)]
pub struct InstallRequest {
    pub client: String,
    /// R47.4: search skill only — folder + `name:` (default `ask-docs`).
    pub handle: Option<String>,
}

/// POST /system/skills/:name/install — explicit user action from the UI only.
/// Writes the embedded skill bytes with backup + never-clobber semantics.
pub async fn install_handler(UrlPath(name): UrlPath<String>, Json(req): Json<InstallRequest>) -> Json<Value> {
    let (client, handle) = (req.client, req.handle);
    let result = tokio::task::spawn_blocking(move || install_skill(&client, &name, handle.as_deref())).await.unwrap();
    Json(finish(result))
}

fn install_skill(client: &str, skill: &str, handle: Option<&str>) -> Result<Value, String> {
    if is_search_skill(skill) {
        return install_search(client, handle);
    }
    if handle.is_some() {
        return Err(format!("Only the search skill ({SEARCH_DIR}) takes a handle"));
    }
    let path = install_target_path(client, skill)?;
    let bytes = skill_bytes(skill).ok_or_else(|| format!("Unknown skill '{skill}' (setup, search)"))?;
    match install_skill_file(&path, bytes)? {
        InstallOutcome::AlreadyInstalled => Ok(json!({
            "path": path.to_string_lossy(),
            "already_installed": true,
            "backup": Value::Null,
        })),
        InstallOutcome::Written { backup } => Ok(json!({
            "path": path.to_string_lossy(),
            "already_installed": false,
            "backup": backup.map(|b| b.to_string_lossy().to_string()),
        })),
    }
}

fn install_search(client: &str, handle: Option<&str>) -> Result<Value, String> {
    let root = skills_root_for(client).ok_or_else(|| format!("unknown client '{client}'"))?;
    let handle = validate_handle(handle.unwrap_or(SEARCH_DIR))?;
    let (path, outcome, retired) = install_search_at(&root, &handle)?;
    let (already_installed, backup) = match outcome {
        InstallOutcome::AlreadyInstalled => (true, None),
        InstallOutcome::Written { backup } => (false, backup),
    };
    Ok(json!({
        "path": path.to_string_lossy(),
        "handle": handle,
        "already_installed": already_installed && retired.is_empty(),
        "backup": backup.map(|b| b.to_string_lossy().to_string()),
        "retired": retired.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn target_under(home: &Path, client: &str, skill: &str) -> PathBuf {
        // Test-only twin of `install_target_path` with an injected home, so
        // install tests never touch the real HOME or mutate env.
        let parts = skills_subpath(client).expect("known client in tests");
        join_many(home, &parts).join(canonical_skill_dir(skill).unwrap()).join("SKILL.md")
    }

    fn repo_skill_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../.agents/skills/{name}/SKILL.md"));
        std::fs::read(&path).unwrap()
    }

    #[test]
    fn embedded_bytes_match_repo_skill_files() {
        assert_eq!(SETUP_SKILL_MD.as_bytes(), repo_skill_bytes("anchorcore-setup").as_slice());
        assert_eq!(SEARCH_SKILL_MD.as_bytes(), repo_skill_bytes("ask-docs").as_slice());
    }

    #[test]
    fn install_writes_skill_file_to_matrix_path() {
        let home = tmp_home();
        for skill in ["setup", "search"] {
            let path = target_under(home.path(), "claude-code", skill);
            assert_eq!(
                path,
                home.path().join(".claude").join("skills").join(canonical_skill_dir(skill).unwrap()).join("SKILL.md")
            );
            let bytes = skill_bytes(skill).unwrap();
            match install_skill_file(&path, bytes).unwrap() {
                InstallOutcome::Written { backup } => assert!(backup.is_none(), "fresh install has no backup"),
                InstallOutcome::AlreadyInstalled => panic!("fresh install must write"),
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn reinstall_identical_is_noop_success() {
        let home = tmp_home();
        let path = target_under(home.path(), "cursor", "search");
        let bytes = skill_bytes("search").unwrap();
        install_skill_file(&path, bytes).unwrap();
        assert!(!backup_path(&path).exists());
        match install_skill_file(&path, bytes).unwrap() {
            InstallOutcome::AlreadyInstalled => {}
            InstallOutcome::Written { .. } => panic!("identical reinstall must be a no-op"),
        }
        assert!(!backup_path(&path).exists(), "no-op must not create a backup");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
    }

    #[test]
    fn reinstall_differing_content_backs_up_before_writing() {
        let home = tmp_home();
        let path = target_under(home.path(), "codex", "setup");
        let bytes = skill_bytes("setup").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "hand-edited skill content").unwrap();
        match install_skill_file(&path, bytes).unwrap() {
            InstallOutcome::Written { backup } => {
                let b = backup.expect("differing content must back up");
                assert_eq!(b, backup_path(&path));
                assert!(b.exists());
                assert_eq!(std::fs::read_to_string(&b).unwrap(), "hand-edited skill content");
            }
            InstallOutcome::AlreadyInstalled => panic!("differing content must rewrite"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
    }

    #[test]
    fn handles_are_one_folder_name() {
        assert_eq!(validate_handle(" ask-docs ").unwrap(), "ask-docs");
        assert_eq!(validate_handle("kb2").unwrap(), "kb2");
        for bad in ["", "-x", "x-", "Ask", "a b", "a/b", "..", "../x", "a.b", "anchorcore-setup", &"a".repeat(65)] {
            assert!(validate_handle(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn custom_handle_renames_and_retires_older_copies() {
        let home = tmp_home();
        let root = home.path().join("skills");
        // An install from an older build (pre-marker, old name).
        std::fs::create_dir_all(root.join("anchorcore-search")).unwrap();
        std::fs::write(root.join("anchorcore-search/SKILL.md"), "---\nname: anchorcore-search\ndescription: old\n---\nbody").unwrap();
        let (path, _, retired) = install_search_at(&root, "kb").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\nname: kb\n") && text.contains("`/kb what did we decide"), "personalised name + example");
        assert_eq!(text.replace("\nname: kb\n", "\nname: ask-docs\n").replace("`/kb ", "`/ask-docs "), SEARCH_SKILL_MD, "nothing else changes");
        assert_eq!(retired, vec![root.join("anchorcore-search/SKILL.md.anchorcore.bak")]);
        assert!(!root.join("anchorcore-search/SKILL.md").exists(), "the old copy no longer loads");
        // Back to the default: the custom copy is retired in turn.
        let (path, _, retired) = install_search_at(&root, SEARCH_DIR).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SEARCH_SKILL_MD);
        assert_eq!(retired, vec![root.join("kb/SKILL.md.anchorcore.bak")]);
        assert_eq!(installed_search_dirs(&root), vec![root.join(SEARCH_DIR)]);
    }

    #[test]
    fn a_foreign_skill_at_the_handle_is_never_overwritten() {
        let home = tmp_home();
        let root = home.path().join("skills");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/SKILL.md"), "---\nname: notes\n---\nsomeone else's").unwrap();
        let err = install_search_at(&root, "notes").err().expect("refused");
        assert!(err.contains("different skill"), "{err}");
        assert_eq!(std::fs::read_to_string(root.join("notes/SKILL.md")).unwrap(), "---\nname: notes\n---\nsomeone else's");
    }

    #[test]
    fn setup_skill_takes_no_handle() {
        let err = install_skill("claude-code", "setup", Some("x")).err().expect("refused");
        assert!(err.contains("Only the search skill"), "{err}");
    }

    #[test]
    fn dir_allowlist_is_pinned() {
        // Every client id resolves to exactly one home-relative skills root;
        // anything else resolves to nothing. Additions here are deliberate.
        let home = Path::new("/home/testuser");
        let expected: &[(&str, &str)] = &[
            ("claude-code", "/home/testuser/.claude/skills"),
            ("cursor", "/home/testuser/.cursor/skills"),
            ("opencode", "/home/testuser/.config/opencode/skills"),
            ("codex", "/home/testuser/.codex/skills"),
        ];
        for (id, want) in expected {
            let got = join_many(home, &skills_subpath(id).unwrap());
            assert_eq!(got, PathBuf::from(want), "allowlist moved for {id}");
        }
        if cfg!(target_os = "macos") {
            let got = join_many(home, &skills_subpath("claude-desktop").unwrap());
            assert_eq!(got, PathBuf::from("/home/testuser/Library/Application Support/Claude/skills"));
        } else if cfg!(windows) {
            assert!(skills_subpath("claude-desktop").is_none(), "windows desktop goes via APPDATA");
        } else {
            let got = join_many(home, &skills_subpath("claude-desktop").unwrap());
            assert_eq!(got, PathBuf::from("/home/testuser/.config/Claude/skills"));
        }
        for bad in ["", "vscode", "CLaude-code", "../x", ".", "claude-code/.."] {
            assert!(skills_subpath(bad).is_none(), "{bad} must not resolve");
            assert!(client_name(bad).is_none(), "{bad} must have no name");
        }
        assert_eq!(CLIENT_IDS.len(), 5, "client count is pinned");
    }

    #[test]
    fn every_matrix_target_stays_under_home() {
        let home = Path::new("/home/testuser");
        for id in CLIENT_IDS {
            let Some(parts) = skills_subpath(id) else { continue }; // windows desktop: APPDATA branch
            for skill in ["setup", "search", "anchorcore-setup", "anchorcore-search", "ask-docs"] {
                let dir = canonical_skill_dir(skill).unwrap();
                let target = join_many(home, &parts).join(dir).join("SKILL.md");
                assert!(target.starts_with(home), "{target:?} escapes home");
                assert_eq!(target.file_name().unwrap(), "SKILL.md");
                assert_eq!(target.parent().unwrap().file_name().unwrap(), dir);
            }
        }
    }

    #[test]
    fn skill_names_accept_aliases_and_reject_traversal() {
        assert_eq!(canonical_skill_dir("setup"), Some("anchorcore-setup"));
        assert_eq!(canonical_skill_dir("anchorcore-setup"), Some("anchorcore-setup"));
        assert_eq!(canonical_skill_dir("search"), Some("ask-docs"));
        assert_eq!(canonical_skill_dir("ask-docs"), Some("ask-docs"));
        assert_eq!(canonical_skill_dir("anchorcore-search"), Some("ask-docs"), "old name still resolves");
        for bad in ["", "SETUP", "setup.md", "..", "../x", "a/../../etc", "anchorcore-setup/../x", "skill"] {
            assert!(canonical_skill_dir(bad).is_none(), "{bad} must be rejected");
            assert!(skill_bytes(bad).is_none(), "{bad} must have no bytes");
        }
    }

    #[test]
    fn install_target_rejects_unknown_client_and_skill() {
        // Uses the real HOME-backed constructor: only the error paths run, no writes.
        // Holds the env lock so the HOME-mutating test below can't interleave.
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(install_target_path("vscode", "setup").unwrap_err().contains("unknown client"));
        assert!(install_target_path("cursor", "nope").unwrap_err().contains("Unknown skill"));
        assert!(install_target_path("", "").is_err());
    }

    #[test]
    fn finish_wraps_ok_and_err_for_ui() {
        let ok = finish(Ok(json!({"path": "/x/SKILL.md", "already_installed": false})));
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["path"], "/x/SKILL.md");
        let err = finish(Err("nope".to_string()));
        assert_eq!(err["ok"], false);
        assert_eq!(err["error"], "nope");
    }

    /// HOME resolution honors `HOME` then `USERPROFILE` (mirrors
    /// `mcp_setup.rs`); guarded by the crate env lock like the other
    /// env-knob tests.
    #[test]
    fn home_dir_prefers_home_over_userprofile() {
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var("HOME").ok();
        let old_profile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", "/tmp/ac-home-test");
        std::env::set_var("USERPROFILE", "/tmp/ac-profile-test");
        assert_eq!(home_dir(), Some(PathBuf::from("/tmp/ac-home-test")));
        std::env::remove_var("HOME");
        assert_eq!(home_dir(), Some(PathBuf::from("/tmp/ac-profile-test")));
        // Empty-but-set HOME does not fall through (mcp_setup.rs parity).
        std::env::set_var("HOME", "");
        assert_eq!(home_dir(), None);
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}
