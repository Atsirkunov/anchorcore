//! R25.4: `anchorcore export` / `anchorcore import` — the headless/Team path.
//!
//! The app owns the SQLite file, so the CLI prefers the running API when one is
//! reachable (`ANCHOR_BACKEND_URL`, else `http://127.0.0.1:<port>`, plus the
//! default 8000) and otherwise works on the database directly (app stopped).
//! Both paths run the same engine (`backup.rs`), so the archive is identical.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::backup::{self, ExportOptions, ImportMode, ImportOptions, REPLACE_CONFIRM};

#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    /// Where to write the archive (default: ./anchorcore-backup-<timestamp>.anchorcore-backup)
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Include source credentials in an encrypted secrets.bundle (needs the passphrase env)
    #[arg(long)]
    pub include_secrets: bool,
    /// Environment variable holding the bundle passphrase (never a CLI argument)
    #[arg(long, default_value = "ANCHOR_BACKUP_PASSPHRASE")]
    pub passphrase_env: String,
    /// Include embeddings (same embed model only)
    #[arg(long)]
    pub include_embeddings: bool,
    /// Include jobs / system events / users
    #[arg(long)]
    pub include_ops: bool,
    /// Export only this project's sources
    #[arg(long)]
    pub project_id: Option<i64>,
}

#[derive(clap::Args, Debug)]
pub struct ImportArgs {
    /// The .anchorcore-backup archive to import
    #[arg(long)]
    pub file: PathBuf,
    /// merge (default) or replace
    #[arg(long, default_value = "merge")]
    pub mode: String,
    /// Report what would happen without writing anything
    #[arg(long)]
    pub dry_run: bool,
    /// Required with --mode replace: acknowledges that local data is deleted
    #[arg(long)]
    pub yes_destructive: bool,
    /// Environment variable holding the secrets.bundle passphrase
    #[arg(long, default_value = "ANCHOR_BACKUP_PASSPHRASE")]
    pub passphrase_env: String,
    /// Folder remap as source_uid=new_path (repeatable)
    #[arg(long = "remap")]
    pub remap: Vec<String>,
}

/// R45.4: `anchorcore embed-model fetch|status`.
#[derive(clap::Args, Debug)]
pub struct EmbedModelArgs {
    #[command(subcommand)]
    pub action: EmbedModelAction,
}

#[derive(clap::Subcommand, Debug)]
pub enum EmbedModelAction {
    /// Download the pinned weights (or verify files dropped into the model
    /// directory), convert to f16 and mark the model ready
    Fetch {
        /// Never touch the network: only verify + convert files already there
        #[arg(long)]
        offline: bool,
    },
    /// Print where the model lives and whether it is ready
    Status,
}

pub async fn run_embed_model(data_dir: &Path, args: EmbedModelArgs) -> i32 {
    attach_parent_console();
    let dir = crate::model_store::model_dir(&crate::model_store::models_root(data_dir));
    match args.action {
        EmbedModelAction::Status => {
            let state = if crate::model_store::ready(&dir) { "ready" } else { "missing" };
            println!("{} {} ({})", crate::model_store::MODEL_NAME, state, dir.display());
            0
        }
        EmbedModelAction::Fetch { offline } => {
            crate::model_store::try_begin();
            match crate::model_store::prepare(&dir, !offline).await {
                Ok(()) => {
                    println!("{} ready at {}", crate::model_store::MODEL_NAME, dir.display());
                    0
                }
                Err(e) => {
                    eprintln!("embed-model fetch failed: {e}");
                    1
                }
            }
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    // Release builds are windowed (no console popup on double-click); a CLI run
    // from a terminal should still print. Harmless when already attached.
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn passphrase_from_env(env_name: &str) -> Result<Option<String>, String> {
    match std::env::var(env_name) {
        Ok(v) if !v.trim().is_empty() => Ok(Some(v)),
        _ => Ok(None),
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

/// The running app's base URL, if one answers `/health`. `ANCHOR_BACKEND_URL`
/// is an explicit override — when set, only that URL is tried.
async fn api_base(port: u16) -> Option<String> {
    let override_url = std::env::var("ANCHOR_BACKEND_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .map(|url| url.trim().trim_end_matches('/').to_string());
    let mut candidates = Vec::new();
    match &override_url {
        Some(url) => candidates.push(url.clone()),
        None => {
            candidates.push(format!("http://127.0.0.1:{}", port));
            if port != 8000 {
                candidates.push("http://127.0.0.1:8000".to_string());
            }
        }
    }
    let client = client().ok()?;
    for base in candidates {
        let probe = client
            .get(format!("{}/health", base))
            .timeout(Duration::from_millis(800))
            .send()
            .await;
        if probe.map(|r| r.status().is_success()).unwrap_or(false) {
            return Some(base);
        }
    }
    None
}

async fn csrf_token(client: &reqwest::Client, base: &str) -> Result<String, String> {
    let value: Value = client
        .get(format!("{}/csrf", base))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(value.get("csrf_token").and_then(|v| v.as_str()).unwrap_or("").to_string())
}

fn locked_hint(msg: String, data_dir: &Path) -> String {
    if msg.to_lowercase().contains("locked") {
        format!(
            "{msg}\nHint: AnchorCore is running and owns the database. Start it (or point the CLI at \
             its API with ANCHOR_BACKEND_URL) and retry, or stop it and re-run. Data dir: {}",
            data_dir.display()
        )
    } else {
        msg
    }
}

fn count_line(counts: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(obj) = counts.as_object() {
        for (k, v) in obj {
            if v.as_u64().unwrap_or(0) > 0 {
                parts.push(format!("{}={}", k, v));
            }
        }
    }
    parts.join(" ")
}

async fn export_via_api(base: &str, opts: &ExportOptions, out: &Option<PathBuf>) -> Result<PathBuf, String> {
    use std::io::Write as _;
    let client = client()?;
    let token = csrf_token(&client, base).await?;
    let mut resp = client
        .post(format!("{}/backup/export", base))
        .header("x-csrf-token", token)
        .json(opts)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("export failed: {} {}", resp.status(), resp.text().await.unwrap_or_default()));
    }
    let server_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("filename=\"").nth(1))
        .map(|v| v.trim_end_matches('"').to_string())
        .unwrap_or_else(|| "anchorcore-backup.anchorcore-backup".to_string());
    let path = match out {
        Some(p) => p.clone(),
        None => std::env::current_dir().map_err(|e| e.to_string())?.join(server_name),
    };
    let mut file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
    }
    file.flush().map_err(|e| e.to_string())?;
    Ok(path)
}

async fn export_direct(data_dir: &Path, opts: &ExportOptions, out: &Option<PathBuf>) -> Result<PathBuf, String> {
    let res = backup::export_to_file(data_dir, opts).map_err(|e| locked_hint(e, data_dir))?;
    println!(
        "Exported {} record(s): {}",
        res.counts.sources + res.counts.items + res.counts.entities + res.counts.chunks,
        count_line(&serde_json::to_value(&res.counts).unwrap_or(Value::Null))
    );
    let path = match out {
        Some(p) => {
            std::fs::rename(&res.path, p).map_err(|e| format!("{}: {e}", p.display()))?;
            p.clone()
        }
        None => res.path.clone(),
    };
    if res.needs_secret > 0 {
        println!("{} source credential(s) must be re-entered after import (needs_secret in the manifest).", res.needs_secret);
    }
    Ok(path)
}

/// Run `anchorcore export`. Returns the process exit code.
pub async fn run_export(data_dir: &Path, port: u16, args: ExportArgs) -> i32 {
    attach_parent_console();
    let passphrase = match passphrase_from_env(&args.passphrase_env) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if args.include_secrets && passphrase.is_none() {
        eprintln!("error: --include-secrets needs a passphrase in ${} (never pass it as an argument)", args.passphrase_env);
        return 1;
    }
    let opts = ExportOptions {
        include_secrets: args.include_secrets,
        passphrase,
        include_embeddings: args.include_embeddings,
        include_ops: args.include_ops,
        project_id: args.project_id,
    };
    let result = match api_base(port).await {
        Some(base) => {
            println!("Using the running AnchorCore at {base}");
            export_via_api(&base, &opts, &args.out).await
        }
        None => export_direct(data_dir, &opts, &args.out).await,
    };
    match result {
        Ok(path) => {
            println!("Archive: {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn import_options(args: &ImportArgs) -> Result<ImportOptions, String> {
    let mode = match args.mode.to_lowercase().as_str() {
        "merge" => ImportMode::Merge,
        "replace" => ImportMode::Replace,
        other => return Err(format!("--mode must be merge or replace (got {other})")),
    };
    if mode == ImportMode::Replace && !args.dry_run && !args.yes_destructive {
        return Err(format!("--mode replace deletes local data; re-run with --yes-destructive (after a safety backup). The confirmation string is \"{REPLACE_CONFIRM}\"."));
    }
    let mut remap = std::collections::BTreeMap::new();
    for entry in &args.remap {
        let Some((uid, path)) = entry.split_once('=') else {
            return Err(format!("--remap expects source_uid=new_path (got {entry})"));
        };
        remap.insert(uid.to_string(), path.to_string());
    }
    Ok(ImportOptions {
        mode,
        dry_run: args.dry_run,
        confirm: args.yes_destructive.then(|| REPLACE_CONFIRM.to_string()),
        passphrase: passphrase_from_env(&args.passphrase_env)?,
        remap,
    })
}

fn print_import_report(report: &Value) {
    let mode = report.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
    let counts = report.get("counts").cloned().unwrap_or(Value::Null);
    let imported = report.get("imported").cloned().unwrap_or(Value::Null);
    let skipped = report.get("skipped").cloned().unwrap_or(Value::Null);
    println!("Mode: {mode}");
    println!("In archive: {}", count_line(&counts));
    println!("Imported:   {}", count_line(&imported));
    println!("Kept local: {}", count_line(&skipped));
    if let Some(backup) = report.get("safety_backup").and_then(|v| v.as_str()) {
        println!("Safety backup: {backup}");
    }
    if let Some(note) = report.get("secrets_note").and_then(|v| v.as_str()) {
        println!("Secrets: {note}");
    } else if let Some(n) = report.get("secrets_imported").and_then(|v| v.as_u64()) {
        if n > 0 {
            println!("Secrets imported: {n}");
        }
    }
    if let Some(needs) = report.get("needs_secret").and_then(|v| v.as_array()) {
        if !needs.is_empty() {
            println!("Credentials to re-enter: {}", needs.len());
        }
    }
    if let Some(jobs) = report.get("reembed_job_ids").and_then(|v| v.as_array()) {
        if !jobs.is_empty() {
            println!("Re-embed jobs queued: {}", jobs.len());
        }
    }
    if let Some(warnings) = report.get("warnings").and_then(|v| v.as_array()) {
        for w in warnings.iter().filter_map(|v| v.as_str()).take(20) {
            println!("warning: {w}");
        }
    }
}

async fn import_via_api(base: &str, file: &Path, opts: &ImportOptions) -> Result<Value, String> {
    let client = client()?;
    let token = csrf_token(&client, base).await?;
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let form = reqwest::multipart::Form::new()
        .text("options", serde_json::to_string(opts).map_err(|e| e.to_string())?)
        .part("file", reqwest::multipart::Part::bytes(bytes).file_name(name));
    let resp = client
        .post(format!("{}/backup/import", base))
        .header("x-csrf-token", token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(|s| s.to_string()))
            .unwrap_or(text);
        return Err(detail);
    }
    serde_json::from_str(&text).map_err(|e| format!("unexpected reply: {e}"))
}

/// Run `anchorcore import`. Returns the process exit code.
pub async fn run_import(data_dir: &Path, port: u16, args: ImportArgs) -> i32 {
    attach_parent_console();
    let opts = match import_options(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if !args.file.exists() {
        eprintln!("error: no such archive: {}", args.file.display());
        return 1;
    }
    let result = match api_base(port).await {
        Some(base) => {
            println!("Using the running AnchorCore at {base}");
            import_via_api(&base, &args.file, &opts).await
        }
        None => backup::import_archive(data_dir, &args.file, &opts)
            .map_err(|e| locked_hint(e, data_dir))
            .map(|report| serde_json::to_value(&report).unwrap_or(Value::Null)),
    };
    match result {
        Ok(report) => {
            print_import_report(&report);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the process-global env mutation for this module's tests.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn backend_url(url: &str) -> Self {
            let lock = crate::secrets::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("ANCHOR_BACKEND_URL").ok();
            std::env::set_var("ANCHOR_BACKEND_URL", url);
            Self { _lock: lock, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("ANCHOR_BACKEND_URL", v),
                None => std::env::remove_var("ANCHOR_BACKEND_URL"),
            }
        }
    }

    fn export_args() -> ExportArgs {
        ExportArgs {
            out: None,
            include_secrets: false,
            passphrase_env: "ANCHOR_BACKUP_PASSPHRASE_TEST_MISSING".to_string(),
            include_embeddings: false,
            include_ops: false,
            project_id: None,
        }
    }

    fn import_args(file: &str) -> ImportArgs {
        ImportArgs {
            file: PathBuf::from(file),
            mode: "merge".to_string(),
            dry_run: false,
            yes_destructive: false,
            passphrase_env: "ANCHOR_BACKUP_PASSPHRASE_TEST_MISSING".to_string(),
            remap: vec![],
        }
    }

    #[tokio::test]
    async fn export_refuses_secrets_without_passphrase_env() {
        let mut args = export_args();
        args.include_secrets = true;
        let code = run_export(Path::new("."), 1, args).await;
        assert_eq!(code, 1);
    }

    #[test]
    fn import_options_validate_mode_and_confirmation() {
        let mut args = import_args("x.anchorcore-backup");
        args.mode = "wipe".to_string();
        assert!(import_options(&args).is_err());

        let mut args = import_args("x.anchorcore-backup");
        args.mode = "replace".to_string();
        assert!(import_options(&args).is_err(), "replace needs --yes-destructive");
        args.yes_destructive = true;
        let opts = import_options(&args).unwrap();
        assert_eq!(opts.mode, ImportMode::Replace);
        assert_eq!(opts.confirm.as_deref(), Some(REPLACE_CONFIRM));
        // a dry run never needs the destructive confirmation
        let mut args = import_args("x.anchorcore-backup");
        args.mode = "replace".to_string();
        args.dry_run = true;
        assert!(import_options(&args).is_ok());
    }

    #[test]
    fn import_options_parse_remap() {
        let mut args = import_args("x.anchorcore-backup");
        args.remap = vec!["uid-1=/tmp/docs".to_string(), "bad".to_string()];
        assert!(import_options(&args).is_err());
        args.remap = vec!["uid-1=/tmp/docs".to_string()];
        let opts = import_options(&args).unwrap();
        assert_eq!(opts.remap.get("uid-1").map(|s| s.as_str()), Some("/tmp/docs"));
    }

    #[tokio::test]
    async fn dry_run_import_prints_and_exits_zero() {
        // a real archive, imported as a dry run into a temp data dir
        let dir = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let src_db = crate::db::resolve_db_path(&src.path().to_string_lossy());
        let conn = crate::db::init_db(&src_db).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let archive = backup::export_to_file(src.path(), &ExportOptions::default()).unwrap();
        let args = ImportArgs {
            file: archive.path.clone(),
            mode: "merge".to_string(),
            dry_run: true,
            yes_destructive: false,
            passphrase_env: "ANCHOR_BACKUP_PASSPHRASE_TEST_MISSING".to_string(),
            remap: vec![],
        };
        let _env = EnvGuard::backend_url("http://127.0.0.1:1");
        assert_eq!(run_import(dir.path(), 1, args).await, 0);
        let conn = crate::db::init_db(&crate::db::resolve_db_path(&dir.path().to_string_lossy())).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "dry run must not write");
    }
}
