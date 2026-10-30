use rusqlite::{Connection, Result};
use rusqlite_migration::{Migrations, M};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub fn resolve_db_path(data_dir: &str) -> PathBuf {
    if let Ok(url) = std::env::var("ANCHOR_DATABASE_URL") {
        if let Some(p) = url.strip_prefix("sqlite:///") {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        if let Some(p) = url.strip_prefix("sqlite://") {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        // handle postgresql case - fallback to data_dir
    }
    PathBuf::from(data_dir).join("anchorcore.db")
}

static MIGRATED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn migrated_paths() -> &'static Mutex<HashSet<PathBuf>> {
    MIGRATED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Open without migrations — fast per-request (P1 4.1: avoids WAL/pragma + 10 migrations scan per HTTP request).
/// Use after boot has called `init_db` once.
pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Open SQLite at `path`, set PRAGMAs, run migrations once per path.
/// Mirrors `backend/app/db.py:44` + `backend/app/main.py:_run_migrations`.
pub fn init_db(path: &Path) -> Result<Connection> {
    init_db_tracked(path).map_err(|f| rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(f.error)))
}

/// R41.7: why boot could not finish, and what state the data is in — both
/// in words the startup error screen shows as is.
#[derive(Debug)]
pub struct InitFailure {
    pub error: String,
    pub data_state: String,
}

impl InitFailure {
    fn new(error: impl ToString, data_state: impl Into<String>) -> Self {
        Self { error: error.to_string(), data_state: data_state.into() }
    }
}

const UNCHANGED: &str = "Nothing in your library was changed: the upgrade stops before touching data.";

/// R41.7: what this boot has to do, decided before anything changes.
struct UpgradePlan {
    from: usize,
    to: usize,
    backup: bool,
    step: u32,
    of: u32,
    backup_file: Option<PathBuf>,
}

impl UpgradePlan {
    fn for_db(conn: &Connection) -> Result<Self, InitFailure> {
        let from = conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .map_err(|e| InitFailure::new(e, UNCHANGED))?
            .max(0) as usize;
        let to = MIGRATIONS.len();
        let backup = legacy_cleanup_needs_backup(conn).map_err(|e| InitFailure::new(e, UNCHANGED))?;
        // backup (if any) + one step per pending migration + finishing up
        let of = u32::from(backup) + to.saturating_sub(from) as u32 + 1;
        crate::startup::plan(of, from > 0 && to > from);
        Ok(Self { from, to, backup, step: 0, of, backup_file: None })
    }

    fn has_upgrade_work(&self) -> bool {
        self.backup || self.to > self.from
    }

    fn next(&mut self, phase: &'static str, detail: &str) {
        self.step += 1;
        crate::startup::step(self.step, phase, detail);
    }
}

/// Same as [`init_db`], with the failure explained for the startup screen
/// and every phase reported to `startup` (R41.7).
pub fn init_db_tracked(path: &Path) -> Result<Connection, InitFailure> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Fast path: if this path already migrated in this process, just open with pragmas
    if migrated_paths().lock().unwrap_or_else(|e| e.into_inner()).contains(path) {
        return open_db(path).map_err(|e| InitFailure::new(e, UNCHANGED));
    }
    // PRAGMAs — must match Python (WAL, busy_timeout, synchronous, foreign_keys)
    let mut conn = open_db(path).map_err(|e| {
        InitFailure::new(format!("cannot open the database: {e}"), "The database file could not be opened; it was not changed.")
    })?;
    let mut plan = UpgradePlan::for_db(&conn)?;
    if plan.has_upgrade_work() {
        check_free_disk(path)?;
    }
    if plan.backup {
        plan.next("backup", "Making a safety backup of your library");
        plan.backup_file = backup_before_legacy_cleanup(&conn, path).map_err(|e| InitFailure::new(e, UNCHANGED))?;
    }
    // Run migrations (see `migrations()` below). If the DB already exists from
    // Python/Alembic, this will apply only missing ones; otherwise it creates all.
    run_migrations(&mut conn, &mut plan)?;
    plan.next("finishing", "Checking the index");
    finish_init(&conn).map_err(|e| {
        InitFailure::new(
            e,
            "Every schema migration finished and was kept; the final column check failed and removed nothing. Restarting AnchorCore runs the check again.",
        )
    })?;
    migrated_paths().lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf());
    Ok(conn)
}

/// R41.7 (5): the safety backup plus a table rewrite need roughly twice the
/// database on disk — refuse up front rather than fail halfway.
fn check_free_disk(path: &Path) -> Result<(), InitFailure> {
    let size = |p: PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let db_bytes = size(path.to_path_buf()) + size(PathBuf::from(format!("{}-wal", path.display())));
    let dir = path.parent().unwrap_or(Path::new("."));
    let free = crate::startup::free_bytes(dir);
    tracing::info!(
        "R41.7: database upgrade — database {} , needs about {} free, {} free; rough estimate {} min on this size",
        crate::startup::human_bytes(db_bytes),
        crate::startup::human_bytes(db_bytes.saturating_mul(2)),
        free.map(crate::startup::human_bytes).unwrap_or_else(|| "unknown".to_string()),
        db_bytes / 600_000_000 + 1
    );
    match crate::startup::disk_shortfall(db_bytes, free) {
        Some(msg) => Err(InitFailure::new(msg, UNCHANGED)),
        None => Ok(()),
    }
}

/// Post-migration upkeep (idempotent; runs on every boot).
fn finish_init(conn: &Connection) -> Result<()> {
    // Idempotent column adds for incremental schema (b39, b30, b26, etc.)
    ensure_columns(conn)?;
    // R25.1: rows created before the uid columns existed (or by insert paths
    // that don't set one) get their stable id here — SQL cannot make UUIDv7.
    let _ = backfill_uids(conn);
    ensure_vec(conn);
    // Ensure PRAGMAs after migrations (some migrations may reset)
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // R36.2: items an older chunker produced are rebuilt from their stored
    // text now — after `ensure_columns` (chunks.kind/path/section_id) and
    // `ensure_vec` (the vec_chunks sync triggers must see the deletes).
    // Migration 28 holds the per-item marker; a no-op query when current.
    // Failures are per item and retried next boot.
    if let Err(e) = crate::pipeline::rechunk_outdated_items(conn) {
        tracing::warn!("R36.2: chunker upgrade skipped: {}", e);
    }
    Ok(())
}

fn try_add_column(conn: &Connection, sql: &str) {
    if let Err(e) = conn.execute(sql, []) {
        let msg = e.to_string();
        // "duplicate column name" is expected when Python already added it
        if !msg.contains("duplicate column") && !msg.contains("already exists") {
            tracing::warn!("ensure_columns failed: {} -> {}", sql, msg);
        }
    }
}

fn ensure_columns(conn: &Connection) -> Result<()> {
    // sources.label (b39)
    try_add_column(conn, "ALTER TABLE sources ADD COLUMN label VARCHAR(20) NOT NULL DEFAULT 'internal'");
    // ingested_items additional (b26 + misc)
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN doc_type VARCHAR(50) NOT NULL DEFAULT ''");
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN window_hashes TEXT NOT NULL DEFAULT '{}'");
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN distill_hashes TEXT NOT NULL DEFAULT '{}'");
    // R35.4: per-document Sensitive override (NULL = inherit from source)
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN sensitive_override INTEGER");
    // Incremental folder sync: stat fingerprint (NULL = unknown, always extract)
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN file_mtime_ms INTEGER");
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN file_size INTEGER");
    // F-03: completion marker (NULL = never fully processed; backfilled once by migration 18)
    try_add_column(conn, "ALTER TABLE ingested_items ADD COLUMN processed_hash TEXT");
    // entities.window_text/window_index/dispute_count (b26 + B3)
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN window_text TEXT NOT NULL DEFAULT ''");
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN window_index INTEGER");
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN dispute_count INTEGER NOT NULL DEFAULT 0");
    // chunks additions (B30, B18)
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN kind VARCHAR(20) NOT NULL DEFAULT 'document'");
    // R35.4 absorbs R35.3: the chunk PII columns are dropped by migration 29
    // and must never be re-added here.
    // R14.1 hierarchical TOC
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN section_id INTEGER REFERENCES sections(id) ON DELETE SET NULL");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN level INTEGER NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN path TEXT NOT NULL DEFAULT ''");
    // relationships.note (R27.4 conflict links)
    try_add_column(conn, "ALTER TABLE relationships ADD COLUMN note TEXT NOT NULL DEFAULT ''");
    // ensure sections table exists even on old DBs that migrated before 11_sections.sql
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS sections (
            id INTEGER PRIMARY KEY,
            item_id INTEGER NOT NULL REFERENCES ingested_items(id) ON DELETE CASCADE,
            parent_id INTEGER REFERENCES sections(id) ON DELETE SET NULL,
            level INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '',
            path TEXT NOT NULL DEFAULT '',
            chunk_range TEXT NOT NULL DEFAULT '',
            summary TEXT NOT NULL DEFAULT '',
            summary_embedding BLOB,
            summary_hash TEXT NOT NULL DEFAULT '',
            line_start INTEGER NOT NULL DEFAULT 0,
            line_end INTEGER NOT NULL DEFAULT 0,
            kind TEXT NOT NULL DEFAULT 'section',
            meta TEXT NOT NULL DEFAULT '{}',
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_sections_item_id ON sections(item_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_sections_parent_id ON sections(parent_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_chunks_section_id ON chunks(section_id)", []);
    // R41.10: mirrors migration 31 for DBs whose migration run stopped early
    // (the re-chunk check below does one lookup per item).
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_chunks_item_id ON chunks(item_id)", []);
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN summary_hash TEXT NOT NULL DEFAULT ''");
    // R38.1 markdown ranges (mirrors 19_markdown_ranges.sql for straggler DBs)
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN line_start INTEGER NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN line_end INTEGER NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN kind TEXT NOT NULL DEFAULT 'section'");
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN meta TEXT NOT NULL DEFAULT '{}'");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN line_start INTEGER NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN line_end INTEGER NOT NULL DEFAULT 0");
    // R14.4 tags
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS tags (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            embedding BLOB,
            count INTEGER NOT NULL DEFAULT 1,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_tags_name ON tags(name)", []);
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS chunk_tags (
            chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
            tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (chunk_id, tag_id)
        )", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_chunk_tags_tag_id ON chunk_tags(tag_id)", []);
    ensure_uid_columns(conn);
    Ok(())
}

/// R25.1: tables that carry a stable UUIDv7 `uid` (plan §6.1). `disputes` and
/// `merge_actions` stay keyed by entity uid refs instead of owning a uid.
pub const UID_TABLES: &[&str] = &[
    "sources",
    "ingested_items",
    "entities",
    "chunks",
    "sections",
    "tags",
    "projects",
];

/// Generate a stable record id — UUIDv7 (time-ordered, sortable).
pub fn new_uid() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Idempotent uid columns + unique indexes; mirrors `14_uids.sql` for DBs whose
/// migration version already advanced past it (same pattern as the ALTERs above).
fn ensure_uid_columns(conn: &Connection) {
    for table in UID_TABLES {
        try_add_column(conn, &format!("ALTER TABLE {} ADD COLUMN uid TEXT", table));
        let idx = format!("CREATE UNIQUE INDEX IF NOT EXISTS ix_{}_uid ON {}(uid)", table, table);
        let _ = conn.execute(&idx, []);
    }
}

/// Assign a UUIDv7 to every row that has none. Called at boot and before an
/// export/import snapshot so no record is ever serialized without a stable id.
pub fn backfill_uids(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    let mut filled = 0usize;
    for table in UID_TABLES {
        let select = format!("SELECT id FROM {} WHERE uid IS NULL OR uid = ''", table);
        let ids: Vec<i64> = {
            let mut stmt = tx.prepare(&select)?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        if ids.is_empty() {
            continue;
        }
        let update = format!("UPDATE {} SET uid = ?1 WHERE id = ?2", table);
        let mut stmt = tx.prepare(&update)?;
        for id in ids {
            stmt.execute(rusqlite::params![new_uid(), id])?;
            filled += 1;
        }
    }
    tx.commit()?;
    Ok(filled)
}

fn ensure_vec(conn: &Connection) {
    // Try to create vec0 index; ignore if sqlite-vec not available (like Python fallback b33a0c1)
    // Dim from ANCHOR_EMBED_DIM (default 768 = nomic-embed-text) — matches backend/app/config.py:64
    let dim: usize = std::env::var("ANCHOR_EMBED_DIM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(768);
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(embedding float[{}] distance_metric=cosine)",
        dim
    );
    if let Err(e) = conn.execute(&sql, []) {
        let msg = e.to_string();
        if msg.contains("no such module: vec0") {
            // fallback when sqlite-vec is not statically linked (Rust bundled sqlite):
            // plain table + sync triggers so the scan-backed stack has a stable
            // shape (pinned by tests/api_smoke.rs::fts_and_vec_tables_exist_without_vec0)
            let _ = conn.execute("CREATE TABLE IF NOT EXISTS vec_chunks (rowid INTEGER PRIMARY KEY, embedding BLOB)", []);
            let fallbacks = [
                r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_ai AFTER INSERT ON chunks WHEN new.embedding IS NOT NULL BEGIN INSERT OR IGNORE INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding); END"#,
                r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_ad AFTER DELETE ON chunks BEGIN DELETE FROM vec_chunks WHERE rowid = old.id; END"#,
                r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_au AFTER UPDATE OF embedding ON chunks WHEN new.embedding IS NOT NULL BEGIN DELETE FROM vec_chunks WHERE rowid = old.id; INSERT OR IGNORE INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding); END"#,
            ];
            for trg in fallbacks {
                let _ = conn.execute(trg, []);
            }
            return;
        }
        if !msg.contains("already exists") {
            tracing::warn!("ensure_vec failed (sqlite-vec missing is ok): {}", msg);
        }
        return;
    }
    // Triggers like b33a0c1_add_vec_chunks.py
    let dim_bytes = dim * 4;
    let triggers = [
        format!(r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_ai AFTER INSERT ON chunks WHEN new.embedding IS NOT NULL AND length(new.embedding) = {} BEGIN INSERT INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding); END"#, dim_bytes),
        r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_ad AFTER DELETE ON chunks BEGIN DELETE FROM vec_chunks WHERE rowid = old.id; END"#.to_string(),
        format!(r#"CREATE TRIGGER IF NOT EXISTS vec_chunks_au AFTER UPDATE OF embedding ON chunks WHEN new.embedding IS NOT NULL AND length(new.embedding) = {} BEGIN DELETE FROM vec_chunks WHERE rowid = old.id; INSERT INTO vec_chunks(rowid, embedding) VALUES (new.id, new.embedding); END"#, dim_bytes),
    ];
    for trg in triggers {
        if let Err(e) = conn.execute(&trg, []) {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("no such table") {
                tracing::warn!("ensure_vec trigger failed: {}", msg);
            }
        }
    }
    // Backfill existing embeddings (like Python migration)
    let backfill = format!("INSERT OR IGNORE INTO vec_chunks(rowid, embedding) SELECT id, embedding FROM chunks WHERE embedding IS NOT NULL AND length(embedding) = {}", dim_bytes);
    let _ = conn.execute(&backfill, []);
}

fn vec_trigger_defs(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND sql LIKE '%vec_chunks%'")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<(String, String)>, _>>()
        })
        .unwrap_or_default()
}

/// One transaction per migration (R41.7) so the startup screen can name
/// each step; a failed step rolls back alone and the next boot resumes there.
fn run_migrations(conn: &mut Connection, plan: &mut UpgradePlan) -> Result<(), InitFailure> {
    // This build has no vec0 module linked, but Python-managed DBs contain a
    // real vec0 table plus vec_chunks_* triggers on chunks. ALTER TABLE ...
    // RENAME re-validates every trigger body, which fails with "no such
    // module: vec0". Drop those triggers for the migration run and restore
    // them byte-identical afterwards (even on failure, so a rolled-back run
    // leaves the schema untouched). Fresh DBs have no such triggers: no-op.
    let saved = if plan.to > plan.from { vec_trigger_defs(conn) } else { Vec::new() };
    for (name, _) in &saved {
        let _ = conn.execute(&format!("DROP TRIGGER IF EXISTS \"{}\"", name.replace('"', "\"\"")), []);
    }
    let result = apply_pending(conn, plan);
    // Migration 21 removes an orphaned vec0 table; its triggers then point at
    // nothing, and `ensure_vec` recreates the plain table + fallback triggers.
    if table_exists(conn, "vec_chunks") {
        for (_, sql) in &saved {
            if let Err(e) = conn.execute(sql, []) {
                tracing::warn!("failed to restore vec trigger after migrate: {}", e);
            }
        }
    }
    result
}

fn apply_pending(conn: &mut Connection, plan: &mut UpgradePlan) -> Result<(), InitFailure> {
    let migrations = migrations();
    for version in plan.from + 1..=plan.to {
        let name = MIGRATIONS[version - 1].2;
        plan.next("migrations", &format!("Migration {version}: {name}"));
        test_slow_step();
        let Err(e) = migrations.to_version(conn, version) else { continue };
        // "already exists" is ok because Python/Alembic already created
        // tables before the rusqlite_migration tracking existed: log, stop
        // migrating, continue with `ensure_columns` (pre-R41.7 behaviour).
        let msg = e.to_string();
        if msg.contains("already exists") || msg.contains("duplicate column") {
            tracing::warn!("migration ignored (idempotent rerun on Python DB): {}", msg);
            return Ok(());
        }
        return Err(InitFailure::new(format!("migration {version} ({name}) failed: {msg}"), migration_data_state(plan, version)));
    }
    Ok(())
}

/// What the error screen says about the data after migration `failed`
/// rolled back (backed by `api_startup` black-box tests).
fn migration_data_state(plan: &UpgradePlan, failed: usize) -> String {
    let mut s = format!(
        "Migration {failed} was rolled back as one transaction, so it changed nothing: your library is at schema version {}.",
        failed - 1
    );
    if failed - 1 > plan.from {
        s.push_str(&format!(" The steps before it (versions {}–{}) finished and were kept.", plan.from + 1, failed - 1));
    }
    if let Some(file) = &plan.backup_file {
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        s.push_str(&format!(" A full safety backup from before this upgrade is in the exports folder ({name})."));
    }
    s.push_str(" Restarting AnchorCore retries from this step.");
    s
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// R45.0 hook for migration 21: drop a vec0 `vec_chunks` this binary cannot
/// load. DROP TABLE needs the module, so the schema row goes via
/// writable_schema; the shadow tables are plain and drop normally. A build
/// that does link vec0 (R43.10) keeps its index untouched.
fn drop_orphan_vec0(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    let sql: Option<String> = tx
        .query_row("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'", [], |r| r.get(0))
        .ok();
    let is_vec0 = sql.is_some_and(|s| s.to_ascii_lowercase().contains("using vec0"));
    if !is_vec0 {
        return Ok(());
    }
    match tx.query_row("SELECT COUNT(*) FROM vec_chunks", [], |r| r.get::<_, i64>(0)) {
        Ok(_) => return Ok(()),
        Err(e) if !e.to_string().contains("no such module: vec0") => return Err(e.into()),
        Err(_) => {}
    }
    for (name, _) in vec_trigger_defs(tx) {
        tx.execute(&format!("DROP TRIGGER IF EXISTS \"{}\"", name.replace('"', "\"\"")), [])?;
    }
    tx.pragma_update(None, "writable_schema", "ON")?;
    let deleted = tx.execute("DELETE FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'", []);
    tx.pragma_update(None, "writable_schema", "RESET")?;
    deleted?;
    let shadows: Vec<String> = tx
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name GLOB 'vec_chunks_*'")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for name in &shadows {
        tx.execute(&format!("DROP TABLE IF EXISTS \"{}\"", name.replace('"', "\"\"")), [])?;
    }
    tracing::warn!("R45.0: removed orphaned vec0 index (vec_chunks + {} shadow tables)", shadows.len());
    Ok(())
}

/// A migration hook (runs inside the migration's transaction).
type Hook = fn(&rusqlite::Transaction) -> rusqlite_migration::HookResult;

/// R41.7: one row per schema version (index + 1): SQL, optional hook, and
/// what the startup screen says while it runs. Append only.
const MIGRATIONS: &[(&str, Option<Hook>, &str)] = &[
    // 23bca0413318_initial_schema — sources, ingested_items, entities, chunks, merge_actions, relationships
    (include_str!("../migrations/01_initial.sql"), None, "creating the library tables"),
    // 7c4a2b9e3d81_add_system_events_table
    (include_str!("../migrations/02_system_events.sql"), None, "adding the event log"),
    // 9d3f1b2c4a51_add_jobs_table
    (include_str!("../migrations/03_jobs.sql"), None, "adding the job queue"),
    // b4a00c1_add_app_settings
    (include_str!("../migrations/04_app_settings.sql"), None, "adding settings"),
    // b3a0c1_disputes + b15a0d1_projects + b18 distilled + b26 review_context (collapsed)
    (include_str!("../migrations/05_misc.sql"), None, "adding projects and review data"),
    // b12f7c0_add_chunks_fts
    (include_str!("../migrations/06_fts.sql"), None, "building the keyword search index"),
    // b39a0c1_add_source_label
    (include_str!("../migrations/07_source_label.sql"), None, "adding source labels"),
    // b33a0c1_add_vec_chunks
    (include_str!("../migrations/08_vec.sql"), None, "adding the vector table"),
    // b30a0c1_add_pii_flags
    (include_str!("../migrations/09_pii.sql"), None, "adding privacy flags"),
    // b40a0c1_add_users
    (include_str!("../migrations/10_users.sql"), None, "adding user accounts"),
    // R14.1 hierarchical TOC
    (include_str!("../migrations/11_sections.sql"), None, "adding document outlines"),
    // R14.4 dynamic tags
    (include_str!("../migrations/12_tags.sql"), None, "adding tags"),
    // Backfill created_at default on system_events (older chains lack it -> audit writes fail)
    (include_str!("../migrations/13_system_events_default.sql"), None, "fixing event timestamps"),
    // R25.1: stable uids (UUIDv7) for backup/export + device migration
    (include_str!("../migrations/14_uids.sql"), None, "adding stable ids for backups"),
    // R27.4: free-text reason on conflict links
    (include_str!("../migrations/15_conflict_note.sql"), None, "adding conflict notes"),
    // R35.1: manual PII mark at whole-document level
    (include_str!("../migrations/16_item_pii.sql"), None, "adding document-level privacy marks"),
    // Incremental folder sync: stat fingerprint so unchanged files skip extraction
    (include_str!("../migrations/17_file_fingerprint.sql"), None, "adding file fingerprints"),
    // F-03: per-item completion marker (processed_hash) with one-shot backfill
    (include_str!("../migrations/18_processed_hash.sql"), None, "adding completion markers"),
    // R38.1: markdown line ranges + section kind/meta
    (include_str!("../migrations/19_markdown_ranges.sql"), None, "adding markdown line ranges"),
    // R40.7: corpus vocabulary table (R45.13: glossary + initialism rules)
    (include_str!("../migrations/20_vocabulary.sql"), None, "adding the vocabulary table"),
    // R45.0: drop a Python-era vec0 index this binary cannot load
    (include_str!("../migrations/21_drop_orphan_vec0.sql"), Some(drop_orphan_vec0), "removing an old vector index"),
    // R45.5: vector provenance (chunks.embed_fp + embed_state) with one-shot backfill
    (include_str!("../migrations/22_vector_provenance.sql"), None, "recording vector provenance"),
    // R45.8: record fields on entities (decision id/status, action due/done)
    (include_str!("../migrations/23_entity_records.sql"), None, "adding decision and action fields"),
    // R45.13: glossary marks + drop vocabulary no person or initialism backs
    (include_str!("../migrations/24_glossary.sql"), Some(prune_vocabulary), "adding glossary marks"),
    // R45.18: drop distilled chunks (distillation deleted)
    (include_str!("../migrations/25_drop_distilled.sql"), Some(drop_distilled_chunks), "dropping distilled chunks"),
    // R46.7: image assets — deduped blobs + per-reference rows
    (include_str!("../migrations/26_assets.sql"), None, "adding image storage"),
    // R45.10: legacy entity cleanup (backup first, see `init_db`)
    (include_str!("../migrations/27_legacy_entities.sql"), Some(cleanup_legacy_entities), "cleaning up legacy entities"),
    // R36.2: chunker generation per item (re-chunk runs in `init_db`)
    (include_str!("../migrations/28_chunker_version.sql"), None, "adding chunker generations"),
    (include_str!("../migrations/29_sensitive_bundle.sql"), Some(bundle_sensitive), "rewriting the search index for Sensitive labels (can take several minutes on large libraries)"),
    // R47.2: dismissable system events (column added by the hook)
    (include_str!("../migrations/30_event_dismiss.sql"), Some(add_event_dismissed_at), "making errors dismissable"),
    // R41.10: per-item chunk lookups (re-chunk check, chunk deletes) use an index
    (include_str!("../migrations/31_chunk_item_index.sql"), None, "indexing chunks by document (can take a minute on large libraries)"),
    // R41.8: entity cascades use indexes (the migration 27 hook runs it first)
    (ENTITY_FK_INDEXES, None, "indexing records"),
];

const ENTITY_FK_INDEXES: &str = include_str!("../migrations/32_entity_fk_indexes.sql");

fn migrations() -> Migrations<'static> {
    let fail_at = test_fail_migration();
    Migrations::new(
        MIGRATIONS
            .iter()
            .enumerate()
            .map(|(i, &(sql, hook, _))| match (hook, fail_at == Some(i + 1)) {
                (_, true) => M::up_with_hook(sql, move |tx: &rusqlite::Transaction| failing_hook(tx, hook)),
                (Some(h), false) => M::up_with_hook(sql, h),
                (None, false) => M::up(sql),
            })
            .collect(),
    )
}

/// Debug builds only: `ANCHOR_TEST_FAIL_MIGRATION=<version>` makes that
/// migration fail after its SQL ran (so the black-box suite can prove the
/// rollback); `ANCHOR_TEST_SLOW_MIGRATION_MS` slows every step.
fn test_fail_migration() -> Option<usize> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("ANCHOR_TEST_FAIL_MIGRATION").ok()?.parse().ok()
}

fn test_slow_step() {
    if !cfg!(debug_assertions) {
        return;
    }
    if let Some(ms) = std::env::var("ANCHOR_TEST_SLOW_MIGRATION_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

fn failing_hook(tx: &rusqlite::Transaction, inner: Option<Hook>) -> rusqlite_migration::HookResult {
    if let Some(h) = inner {
        h(tx)?;
    }
    Err(rusqlite_migration::HookError::Hook("test hook: simulated migration failure".to_string()))
}

/// R45.13 hook for migration 24 (the initialism check is Rust, not SQL).
fn prune_vocabulary(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    let n = crate::vocab::prune_to_trusted(tx)?;
    if n > 0 {
        tracing::warn!("R45.13: dropped {} vocabulary rows (LLM proposals + non-initialism rules)", n);
    }
    Ok(())
}

/// Legacy = not written by the R45.8 record parser.
const LEGACY_ENTITY: &str = "reasoning <> ?1";

/// R45.10 hook for migration 27: delete legacy entities on non-record items,
/// demote the rest of the legacy `verified` rows, record the counts.
fn cleanup_legacy_entities(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    let marker = crate::records::RECORD_REASONING;
    let before: i64 = tx.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))?;
    let legacy: i64 =
        tx.query_row(&format!("SELECT COUNT(*) FROM entities WHERE {LEGACY_ENTITY}"), [marker], |r| r.get(0))?;
    if legacy == 0 {
        return Ok(());
    }
    // R41.8: without these each deleted row scans `chunks` for its cascade.
    tx.execute_batch(ENTITY_FK_INDEXES)?;
    // No doc_type column yet → no item is in the record bucket.
    let records = if column_exists(tx, "ingested_items", "doc_type") {
        "SELECT id FROM ingested_items WHERE doc_type = 'record'"
    } else {
        "SELECT id FROM ingested_items WHERE 0"
    };
    let deleted = tx.execute(&format!("DELETE FROM entities WHERE {LEGACY_ENTITY} AND item_id NOT IN ({records})"), [marker])?;
    let demoted = tx.execute(
        &format!("UPDATE entities SET status = 'unverified', updated_at = datetime('now') WHERE {LEGACY_ENTITY} AND status = 'verified'"),
        [marker],
    )?;
    let after = before - deleted as i64;
    let detail = format!(r#"{{"before":{before},"legacy":{legacy},"deleted":{deleted},"demoted":{demoted},"after":{after}}}"#);
    tracing::warn!("R45.10: legacy entities — {}", detail);
    tx.execute(
        "INSERT INTO system_events (component, level, message, detail) VALUES ('migration', 'info', 'R45.10 legacy entity cleanup', ?1)",
        [detail],
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        [table, column],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// R45.10: whether migration 27 is pending on a DB with legacy entities
/// (only then is the safety backup taken).
fn legacy_cleanup_needs_backup(conn: &Connection) -> Result<bool> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 || version >= 27 || !table_exists(conn, "entities") {
        return Ok(false);
    }
    let legacy: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM entities WHERE {LEGACY_ENTITY}"),
        [crate::records::RECORD_REASONING],
        |r| r.get(0),
    )?;
    Ok(legacy > 0)
}

/// R45.10: before migration 27 deletes/demotes legacy entities, copy the
/// whole DB to `exports/safety-backup-pre-r45.10-<utc>.db` (never swept).
/// A failed backup fails the boot rather than changing unbacked-up data.
/// R41.7: the startup screen shows bytes written against the DB size.
fn backup_before_legacy_cleanup(conn: &Connection, db_path: &Path) -> Result<Option<PathBuf>> {
    if !legacy_cleanup_needs_backup(conn)? {
        return Ok(None);
    }
    let dir = db_path.parent().unwrap_or(Path::new(".")).join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| backup_error(&dir, e))?;
    let file = dir.join(format!("safety-backup-pre-r45.10-{}.db", chrono::Utc::now().format("%Y%m%dT%H%M%SZ")));
    let total = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    let watcher = BackupProgress::spawn(file.clone(), total);
    let res = conn.execute("VACUUM INTO ?1", [file.to_string_lossy()]);
    watcher.stop();
    res?;
    tracing::warn!("R45.10: legacy entities to clean up — backed up the database to {}", file.display());
    Ok(Some(file))
}

/// Polls the growing backup file so `/health` can show bytes copied.
struct BackupProgress {
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BackupProgress {
    fn spawn(file: PathBuf, total: u64) -> Self {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        let _ = std::thread::Builder::new().name("backup-progress".to_string()).spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                let written = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                crate::startup::progress(written.min(total), total, "bytes");
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
        Self { done }
    }

    fn stop(self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn backup_error(dir: &Path, e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(1),
        Some(format!("R45.10 safety backup: cannot create {}: {e}", dir.display())),
    )
}

/// R45.18 hook for migration 25: drop distilled chunks when the schema has
/// them (fresh DBs gain `chunks.kind` / `chunk_tags` only after migrations).
fn drop_distilled_chunks(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    let has = |sql: &str| tx.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if !has("SELECT COUNT(*) FROM pragma_table_info('chunks') WHERE name = 'kind'") {
        return Ok(());
    }
    if has("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunk_tags'") {
        tx.execute("DELETE FROM chunk_tags WHERE chunk_id IN (SELECT id FROM chunks WHERE kind = 'distilled')", [])?;
    }
    let n = tx.execute("DELETE FROM chunks WHERE kind = 'distilled'", [])?;
    if n > 0 {
        tracing::info!("R45.18: dropped {} distilled chunks", n);
    }
    Ok(())
}

/// R35.4 hook for migration 29: bundle PII into one Sensitive label.
/// Legacy `pii` source labels become `sensitive`; `ingested_items.is_pii`
/// becomes the tri-state `sensitive_override` (explicit marks survive as 1,
/// unmarked rows become NULL = inherit); dead chunk PII columns are dropped.
/// Every step guards on column existence: straggler DBs may miss
/// `try_add_column` columns until `ensure_columns` runs after migrations.
fn bundle_sensitive(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    if column_exists(tx, "sources", "label") {
        tx.execute("UPDATE sources SET label = 'sensitive' WHERE label = 'pii'", [])?;
    }
    // `is_pii` was added `NOT NULL DEFAULT 0`, so a rename would carry the
    // constraint and reject the NULL (inherit) state: add the nullable
    // column, copy explicit marks, drop the old one.
    if !column_exists(tx, "ingested_items", "sensitive_override") {
        tx.execute("ALTER TABLE ingested_items ADD COLUMN sensitive_override INTEGER", [])?;
    }
    if column_exists(tx, "ingested_items", "is_pii") {
        tx.execute("UPDATE ingested_items SET sensitive_override = 1 WHERE is_pii = 1", [])?;
        tx.execute("ALTER TABLE ingested_items DROP COLUMN is_pii", [])?;
    }
    tx.execute("UPDATE ingested_items SET sensitive_override = NULL WHERE COALESCE(sensitive_override, 0) = 0", [])?;
    for col in ["is_pii", "pii_categories"] {
        if column_exists(tx, "chunks", col) {
            tx.execute(&format!("ALTER TABLE chunks DROP COLUMN {col}"), [])?;
        }
    }
    Ok(())
}

/// R47.2 hook for migration 30: `system_events.dismissed_at` (NULL = shown).
fn add_event_dismissed_at(tx: &rusqlite::Transaction) -> rusqlite_migration::HookResult {
    if !column_exists(tx, "system_events", "dismissed_at") {
        tx.execute("ALTER TABLE system_events ADD COLUMN dismissed_at DATETIME", [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn migrate_all(conn: &mut Connection) {
        let mut plan = UpgradePlan::for_db(conn).unwrap();
        run_migrations(conn, &mut plan).unwrap();
    }

    #[test]
    fn every_migration_has_a_name_for_the_startup_screen() {
        assert!(MIGRATIONS.iter().all(|(_, _, name)| !name.is_empty()));
    }

    #[test]
    fn opens_and_migrates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = init_db(&path).unwrap();
        // check tables exist
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
        let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert!(tables.contains(&"sources".to_string()));
        assert!(tables.contains(&"chunks".to_string()));
        // pragmas
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn migration_19_markdown_ranges() {
        // R38.1: sections carry kind/meta + 1-based line ranges, chunks carry
        // line ranges (prose/code/config paths write the 0/0 default).
        let dir = tempdir().unwrap();
        let path = dir.path().join("md19.db");
        let conn = init_db(&path).unwrap();
        for (table, col) in [
            ("sections", "line_start"),
            ("sections", "line_end"),
            ("sections", "kind"),
            ("sections", "meta"),
            ("chunks", "line_start"),
            ("chunks", "line_end"),
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{col}'"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "{table}.{col} must exist after migrate");
        }
    }

    /// Undo migration 23 (R45.8) so a re-run has real work to do.
    fn drop_entity_record_columns(conn: &Connection) {
        conn.execute("DROP INDEX IF EXISTS ix_entities_owner", []).unwrap();
        for col in ["record_id", "record_status", "due", "done", "supersedes", "superseded_by"] {
            conn.execute(&format!("ALTER TABLE entities DROP COLUMN {col}"), [])
                .unwrap_or_else(|e| panic!("drop entities.{col}: {e}"));
        }
    }

    /// Undo migration 24 (R45.13) so a re-run has real work to do.
    fn drop_glossary_artifacts(conn: &Connection) {
        conn.execute("DROP INDEX IF EXISTS ix_vocabulary_origin", []).unwrap();
        conn.execute("ALTER TABLE ingested_items DROP COLUMN is_glossary", []).unwrap();
    }

    /// Undo migration 32 (R41.8): a real pre-v27 DB has none of these.
    fn drop_entity_fk_indexes(conn: &Connection) {
        for name in [
            "ix_chunks_entity_id",
            "ix_relationships_from_entity_id",
            "ix_relationships_to_entity_id",
            "ix_merge_actions_entity_a_id",
            "ix_merge_actions_entity_b_id",
            "ix_disputes_entity_id",
            "ix_entities_item_id",
        ] {
            conn.execute(&format!("DROP INDEX {name}"), []).unwrap();
        }
    }

    #[test]
    fn legacy_cleanup_indexes_before_deleting() {
        let dir = tempdir().unwrap();
        let mut conn = init_db(&dir.path().join("anchorcore.db")).unwrap();
        drop_entity_fk_indexes(&conn);
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        conn.execute("INSERT INTO ingested_items (source_id, content_hash) VALUES (1,'h')", []).unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning) VALUES (1,'decision','x','llm')", []).unwrap();
        conn.execute("INSERT INTO chunks (item_id, entity_id, content) VALUES (1, 1, 'entity chunk')", []).unwrap();
        let tx = conn.transaction().unwrap();
        cleanup_legacy_entities(&tx).unwrap();
        let plan: String = tx
            .query_row("EXPLAIN QUERY PLAN DELETE FROM chunks WHERE entity_id = 1", [], |r| r.get(3))
            .unwrap();
        assert!(plan.contains("ix_chunks_entity_id"), "the hook indexes before its delete: {plan}");
        let left: i64 = tx.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "the entity chunk still cascades");
    }

    fn vec_trigger_sql(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT sql FROM sqlite_master WHERE type = 'trigger' AND sql LIKE '%vec_chunks%' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn migrate_pending_with_vec_triggers_and_no_vec0() {
        // Live-DB repro: schema at v10 + vec0 triggers (created by a vec0-capable
        // build), migrating with a binary that has no vec0 module linked.
        // ALTER TABLE ... RENAME re-validates every trigger, so the pending
        // migration must not die with "no such module: vec0" — and the
        // triggers must survive byte-identical.
        let dir = tempdir().unwrap();
        let path = dir.path().join("live.db");
        init_db(&path).unwrap();
        // Recreate the live-DB shape: a vec0 VIRTUAL TABLE (Python-created)
        // with vec triggers on chunks. This build has no vec0 module, so fake
        // the schema entry directly — exactly what RENAME chokes on.
        {
            let conn = open_db(&path).unwrap();
            // One item whose completion marker migration 18 must adopt.
            conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
            let sid = conn.last_insert_rowid();
            conn.execute("INSERT INTO ingested_items (source_id, content_hash) VALUES (?1,'h')", [sid]).unwrap();
            // A real v10 DB predates R25.1 — drop the uid columns/indexes so
            // migration 14 has real work to do (re-running it on a DB that
            // already has them hits "duplicate column name" by design).
            // Do this before removing vec_chunks: DROP COLUMN re-validates every
            // trigger, and the vec triggers need their table to exist.
            for table in UID_TABLES {
                conn.execute(&format!("DROP INDEX IF EXISTS ix_{}_uid", table), [])
                    .unwrap_or_else(|e| panic!("drop index {table}: {e}"));
                conn.execute(&format!("ALTER TABLE {} DROP COLUMN uid", table), [])
                    .unwrap_or_else(|e| panic!("drop column {table}: {e}"));
            }
            // A real v10 DB also predates R27.4 — drop `note` so migration 15
            // has real work to do (same "duplicate column by design" rule).
            conn.execute("ALTER TABLE relationships DROP COLUMN note", [])
                .unwrap_or_else(|e| panic!("drop note: {e}"));
            // A real v10 DB predates F-03 too — drop `processed_hash` so
            // migration 18 has real work to do (ALTER + one-shot backfill).
            conn.execute("ALTER TABLE ingested_items DROP COLUMN processed_hash", [])
                .unwrap_or_else(|e| panic!("drop processed_hash: {e}"));
            // A real v10 DB also predates R38.1 — drop the markdown ranges
            // so migration 19 has real work to do (same rule).
            for (table, col) in [
                ("sections", "line_start"),
                ("sections", "line_end"),
                ("sections", "kind"),
                ("sections", "meta"),
                ("chunks", "line_start"),
                ("chunks", "line_end"),
            ] {
                conn.execute(&format!("ALTER TABLE {} DROP COLUMN {}", table, col), [])
                    .unwrap_or_else(|e| panic!("drop {table}.{col}: {e}"));
            }
            // A real v10 DB also predates R40.7 — drop the vocabulary table
            // so migration 20 has real work to do (same rule).
            conn.execute("DROP TABLE vocabulary", [])
                .unwrap_or_else(|e| panic!("drop vocabulary: {e}"));
            // A real v10 DB also predates R45.5 — drop the provenance column,
            // index and state table so migration 22 has real work to do, and
            // seed one legacy vector (2 floats) for its one-shot backfill.
            // Before removing vec_chunks: DROP COLUMN re-validates triggers.
            conn.execute("DROP INDEX ix_chunks_embed_fp", []).unwrap();
            conn.execute("ALTER TABLE chunks DROP COLUMN embed_fp", []).unwrap();
            conn.execute("DROP TABLE embed_state", []).unwrap();
            let item: i64 = conn.query_row("SELECT id FROM ingested_items", [], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO chunks (item_id, content, embedding) VALUES (?1, 'legacy', zeroblob(8))", [item])
                .unwrap();
            // R45.18: one distilled chunk for migration 25 to drop.
            conn.execute("INSERT INTO chunks (item_id, kind, content) VALUES (?1, 'distilled', 'Q: a\nA: b')", [item])
                .unwrap();
            // R45.10: legacy rows for migration 27 — one on the unbucketed
            // item (deleted, with its entity chunk), one on a record item
            // (demoted), and a person-verified parser row (kept).
            conn.execute("INSERT INTO ingested_items (source_id, content_hash, doc_type) VALUES (?1,'r','record')", [sid])
                .unwrap();
            let rec = conn.last_insert_rowid();
            for (iid, summary, reasoning) in [
                (item, "legacy on reference", "rule-based classifier"),
                (rec, "legacy on record", "model said so"),
                (rec, "parser row", "record parser"),
            ] {
                conn.execute(
                    "INSERT INTO entities (item_id, kind, summary, reasoning, confidence, status) VALUES (?1,'decision',?2,?3,0.9,'verified')",
                    rusqlite::params![iid, summary, reasoning],
                )
                .unwrap();
            }
            let legacy_ref: i64 = conn.query_row("SELECT id FROM entities WHERE summary='legacy on reference'", [], |r| r.get(0)).unwrap();
            conn.execute("INSERT INTO chunks (item_id, entity_id, content) VALUES (?1, ?2, 'entity chunk')", [item, legacy_ref])
                .unwrap();
            drop_entity_record_columns(&conn);
            drop_glossary_artifacts(&conn);
            drop_entity_fk_indexes(&conn);
            // A real v10 DB also predates R36.2 — drop the chunker
            // generation so migration 28 has real work to do.
            conn.execute("ALTER TABLE ingested_items DROP COLUMN chunker_version", []).unwrap();
            // A real v10 DB also predates R35.4 — the per-document mark is
            // still `is_pii`, the chunk PII columns exist, and a source may
            // carry the `pii` label, so migration 29 has real work to do.
            // Real DBs got `is_pii` from `ensure_columns` as NOT NULL — the
            // shape that broke the first 1.6.0 migration on upgrade.
            conn.execute("ALTER TABLE ingested_items DROP COLUMN sensitive_override", []).unwrap();
            conn.execute("ALTER TABLE ingested_items ADD COLUMN is_pii INTEGER NOT NULL DEFAULT 0", []).unwrap();
            conn.execute("ALTER TABLE chunks ADD COLUMN is_pii BOOLEAN NOT NULL DEFAULT 0", []).unwrap();
            conn.execute("ALTER TABLE chunks ADD COLUMN pii_categories TEXT NOT NULL DEFAULT '[]'", []).unwrap();
            conn.execute("UPDATE sources SET label = 'pii'", []).unwrap();
            conn.execute("UPDATE ingested_items SET is_pii = 1 WHERE content_hash = 'h'", []).unwrap();
            // A real v10 DB also predates R46.7 — drop the asset tables (their
            // triggers go with them) so migration 26 has real work to do.
            conn.execute("DROP TABLE assets", []).unwrap();
            conn.execute("DROP TABLE asset_blobs", []).unwrap();
            conn.execute("DROP TABLE vec_chunks", []).unwrap();
            conn.pragma_update(None, "writable_schema", "ON").unwrap();
            conn.execute(
                "INSERT INTO sqlite_master(type, name, tbl_name, rootpage, sql) VALUES \
                 ('table', 'vec_chunks', 'vec_chunks', 0, \
                 'CREATE VIRTUAL TABLE vec_chunks USING vec0(embedding float[768] distance_metric=cosine)')",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "writable_schema", "OFF").unwrap();
            // vec0's shadow tables are plain tables (live-DB names).
            for shadow in ["vec_chunks_info", "vec_chunks_chunks", "vec_chunks_rowids", "vec_chunks_vector_chunks00"] {
                conn.execute(&format!("CREATE TABLE \"{shadow}\" (x)"), []).unwrap();
            }
            conn.pragma_update(None, "user_version", 10).unwrap();
        }
        let before = vec_trigger_sql(&open_db(&path).unwrap());
        assert!(!before.is_empty(), "test setup must have vec triggers");
        let mut conn = open_db(&path).unwrap();
        migrate_all(&mut conn);
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 32);
        // R41.10 (migration 31): per-item chunk lookups are indexed.
        // R41.8 (migration 32, also run by the 27 hook): entity cascades are.
        for (table, index) in [
            ("chunks", "ix_chunks_item_id"),
            ("assets", "ix_assets_chunk_id"),
            ("chunks", "ix_chunks_entity_id"),
            ("relationships", "ix_relationships_from_entity_id"),
            ("relationships", "ix_relationships_to_entity_id"),
            ("merge_actions", "ix_merge_actions_entity_a_id"),
            ("merge_actions", "ix_merge_actions_entity_b_id"),
            ("disputes", "ix_disputes_entity_id"),
            ("entities", "ix_entities_item_id"),
        ] {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name=?1 AND name=?2", [table, index], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "migrations 31/32 must create {index}");
        }
        let plan: String = conn
            .query_row("EXPLAIN QUERY PLAN DELETE FROM chunks WHERE entity_id = 1", [], |r| r.get(3))
            .unwrap();
        assert!(plan.contains("ix_chunks_entity_id"), "entity cascade must use the index: {plan}");
        // R47.2 (migration 30): migration 13 rebuilt system_events without
        // the column, so the hook must have added it.
        assert!(column_exists(&conn, "system_events", "dismissed_at"), "migration 30 adds dismissed_at");
        // R35.4 (migration 29): legacy `pii` labels bundle into `sensitive`,
        // `is_pii` becomes the tri-state override, chunk PII columns go.
        let pii_labels: i64 = conn.query_row("SELECT COUNT(*) FROM sources WHERE label = 'pii'", [], |r| r.get(0)).unwrap();
        assert_eq!(pii_labels, 0, "migration 29 must bundle pii labels into sensitive");
        assert_eq!(
            conn.query_row("SELECT label FROM sources WHERE name = 's'", [], |r| r.get::<_, String>(0)).unwrap(),
            "sensitive"
        );
        let is_pii_col: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('ingested_items') WHERE name = 'is_pii'", [], |r| r.get(0)).unwrap();
        assert_eq!(is_pii_col, 0, "is_pii must be renamed to sensitive_override");
        let kept: Option<i64> =
            conn.query_row("SELECT sensitive_override FROM ingested_items WHERE content_hash = 'h'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, Some(1), "an explicit mark survives as the override");
        let cleared: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items WHERE sensitive_override IS NULL", [], |r| r.get(0)).unwrap();
        assert!(cleared >= 1, "unmarked rows become NULL (inherit)");
        let chunk_pii: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('chunks') WHERE name IN ('is_pii','pii_categories')", [], |r| r.get(0)).unwrap();
        assert_eq!(chunk_pii, 0, "migration 29 must drop the chunk PII columns");
        let generations: Vec<i64> = conn
            .prepare("SELECT chunker_version FROM ingested_items")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!generations.is_empty() && generations.iter().all(|&g| g == 1), "migration 28 (R36.2) starts existing items at generation 1: {generations:?}");
        let asset_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('assets','asset_blobs','assets_blob_gc_delete','assets_blob_gc_update')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(asset_tables, 4, "migration 26 (R46.7) must create the asset tables + GC triggers");
        let distilled: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind = 'distilled'", [], |r| r.get(0)).unwrap();
        assert_eq!(distilled, 0, "migration 25 (R45.18) must drop distilled chunks");
        let glossary_col: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('ingested_items') WHERE name = 'is_glossary'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(glossary_col, 1, "migration 24 (R45.13) must add is_glossary");
        // R45.5 (migration 22): legacy vectors are stamped with the default
        // HTTP-embedder fingerprint; the state table exists, empty.
        let fp: String = conn
            .query_row("SELECT embed_fp FROM chunks WHERE content = 'legacy'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fp, "ollama:nomic-embed-text:2");
        let states: i64 = conn.query_row("SELECT COUNT(*) FROM embed_state", [], |r| r.get(0)).unwrap();
        assert_eq!(states, 0);
        let record_cols: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('entities') WHERE name IN ('record_id','due','done')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(record_cols, 3, "migration 23 (R45.8) must add the record columns");
        let vocab: String = conn
            .query_row("SELECT name FROM sqlite_master WHERE type='table' AND name='vocabulary'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vocab, "vocabulary", "migration 20 must create the table");
        // R45.0 (migration 21): the orphaned vec0 index and its triggers are
        // gone instead of restored — restoring them broke every chunk write.
        assert!(vec_trigger_sql(&conn).is_empty(), "vec0 triggers must not be restored");
        let leftovers: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name GLOB 'vec_chunks*'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftovers, 0, "vec0 table + shadow tables must be dropped");
        // R45.10 (migration 27): legacy rows deleted off-record, demoted on
        // record items; parser rows untouched; counts recorded.
        let status = |summary: &str| -> Option<String> {
            conn.query_row("SELECT status FROM entities WHERE summary = ?1", [summary], |r| r.get(0)).ok()
        };
        assert_eq!(status("legacy on reference"), None, "legacy row on a non-record item is deleted");
        assert_eq!(status("legacy on record").as_deref(), Some("unverified"), "legacy verified is demoted");
        assert_eq!(status("parser row").as_deref(), Some("verified"), "a person-verified parser row stays");
        let entity_chunks: i64 =
            conn.query_row("SELECT COUNT(*) FROM chunks WHERE content = 'entity chunk'", [], |r| r.get(0)).unwrap();
        assert_eq!(entity_chunks, 0, "the deleted entity's chunk cascades");
        let detail: String = conn
            .query_row("SELECT detail FROM system_events WHERE component = 'migration'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(detail, r#"{"before":3,"legacy":2,"deleted":1,"demoted":1,"after":2}"#);
        let adopted: String =
            conn.query_row("SELECT processed_hash FROM ingested_items WHERE content_hash = 'h'", [], |r| r.get(0)).unwrap();
        assert_eq!(adopted, "h", "migration 18 must backfill processed_hash=content_hash");
        let sys_sql: String = conn
            .query_row("SELECT sql FROM sqlite_master WHERE name = 'system_events'", [], |r| r.get(0))
            .unwrap();
        assert!(sys_sql.contains("DEFAULT CURRENT_TIMESTAMP"), "migration 13 must apply: {sys_sql}");
        // The boot path then rebuilds the plain fallback index, and chunk
        // writes (insert, embedding update, delete) work again.
        ensure_vec(&conn);
        let item: i64 = conn.query_row("SELECT id FROM ingested_items", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO chunks (item_id, content, embedding) VALUES (?1, 'body', x'00')", [item])
            .unwrap();
        let chunk = conn.last_insert_rowid();
        conn.execute("UPDATE chunks SET embedding = x'01' WHERE id = ?1", [chunk]).unwrap();
        conn.execute("DELETE FROM chunks WHERE id = ?1", [chunk]).unwrap();
    }

    #[test]
    fn legacy_cleanup_backs_up_first_only_when_needed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("anchorcore.db");
        let conn = init_db(&path).unwrap();
        let exports = dir.path().join("exports");
        backup_before_legacy_cleanup(&conn, &path).unwrap();
        assert!(!exports.exists(), "at version 27 nothing is backed up");
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        conn.execute("INSERT INTO ingested_items (source_id, content_hash) VALUES (1,'h')", []).unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning) VALUES (1,'decision','x','record parser')", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 26).unwrap();
        backup_before_legacy_cleanup(&conn, &path).unwrap();
        assert!(!exports.exists(), "parser rows alone need no backup");
        conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning) VALUES (1,'decision','y','llm')", []).unwrap();
        backup_before_legacy_cleanup(&conn, &path).unwrap();
        let files: Vec<_> = std::fs::read_dir(&exports).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(files.len(), 1);
        let name = files[0].to_string_lossy().to_string();
        assert!(name.starts_with("safety-backup-pre-r45.10-") && name.ends_with(".db"), "{name}");
        let copy = Connection::open(exports.join(&name)).unwrap();
        let n: i64 = copy.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "the backup holds the pre-cleanup rows");
    }

    #[test]
    fn migration_21_keeps_plain_vec_chunks() {
        // Fresh/Rust-made DBs have a plain vec_chunks table: migration 21
        // must leave it and its fallback triggers alone.
        let dir = tempdir().unwrap();
        let path = dir.path().join("plain.db");
        init_db(&path).unwrap();
        let mut conn = open_db(&path).unwrap();
        let before = vec_trigger_sql(&conn);
        assert!(!before.is_empty(), "ensure_vec must create fallback triggers");
        // Rewinding to 20 re-runs migration 22 too: drop its artifacts first.
        conn.execute("DROP INDEX ix_chunks_embed_fp", []).unwrap();
        conn.execute("ALTER TABLE chunks DROP COLUMN embed_fp", []).unwrap();
        conn.execute("DROP TABLE embed_state", []).unwrap();
        conn.pragma_update(None, "user_version", 20).unwrap();
        drop_entity_record_columns(&conn);
        drop_glossary_artifacts(&conn);
        conn.execute("ALTER TABLE ingested_items DROP COLUMN chunker_version", []).unwrap();
        migrate_all(&mut conn);
        assert!(table_exists(&conn, "vec_chunks"));
        assert_eq!(vec_trigger_sql(&conn), before);
    }

    #[test]
    fn uids_backfill_and_stay_stable() {
        // R25.1: rows written without a uid (older builds, Python backend, raw
        // SQL in tests) get a UUIDv7 on the first boot/export and keep it.
        let dir = tempdir().unwrap();
        let path = dir.path().join("uids.db");
        let conn = init_db(&path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s1','folder')", []).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s2','folder')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='s1'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO ingested_items (source_id, content_hash) VALUES (?1,'h')", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO entities (item_id, kind) VALUES (?1,'note')", [iid]).unwrap();
        conn.execute("INSERT INTO chunks (item_id, content) VALUES (?1,'body')", [iid]).unwrap();
        conn.execute("INSERT INTO sections (item_id, level, title, path) VALUES (?1,1,'S','S')", [iid]).unwrap();
        conn.execute("INSERT INTO tags (name) VALUES ('t')", []).unwrap();
        conn.execute("INSERT INTO projects (name) VALUES ('p')", []).unwrap();

        assert_eq!(backfill_uids(&conn).unwrap(), 8, "2 sources + 1 row in each other table");
        for table in UID_TABLES {
            let (rows, distinct, invalid): (i64, i64, i64) = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*), COUNT(DISTINCT uid), SUM(uid IS NULL OR uid = '' OR length(uid) != 36) FROM {}",
                        table
                    ),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert!(rows > 0, "{table} must have test rows");
            assert_eq!(distinct, rows, "{table} uids must be unique");
            assert_eq!(invalid, 0, "{table} uids must be filled UUID strings");
        }
        // second pass is a no-op: uids are stable
        assert_eq!(backfill_uids(&conn).unwrap(), 0);
        // the unique index enforces uniqueness for direct writes too
        let other: String = conn.query_row("SELECT uid FROM sources WHERE name='s2'", [], |r| r.get(0)).unwrap();
        let dup = conn.execute("UPDATE sources SET uid = ?1 WHERE name='s1'", [other]);
        assert!(dup.is_err(), "duplicate uid must be rejected");
    }
}
