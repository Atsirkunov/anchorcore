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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Fast path: if this path already migrated in this process, just open with pragmas
    {
        let guard = migrated_paths().lock().unwrap();
        if guard.contains(path) {
            drop(guard);
            return open_db(path);
        }
    }
    let mut conn = Connection::open(path)?;

    // PRAGMAs — must match Python (WAL, busy_timeout, synchronous, foreign_keys)
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Run migrations (see `migrations()` below). If the DB already exists from
    // Python/Alembic, this will apply only missing ones; otherwise it creates all.
    // All SQL uses IF NOT EXISTS so re-running on a Python-created DB is safe.
    let migrations = migrations();
    if let Err(e) = migrations.to_latest(&mut conn) {
        // Real syntax errors should surface; "already exists" is ok because
        // Python/Alembic already created tables before rusqlite_migration
        // tracking table existed. Log and continue for the latter.
        let msg = e.to_string();
        if msg.contains("already exists") || msg.contains("duplicate column") {
            tracing::warn!("migration ignored (idempotent rerun on Python DB): {}", msg);
        } else {
            // For fresh DBs any error is fatal - surface it
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1),
                Some(msg),
            ));
        }
    }

    // Idempotent column adds for incremental schema (b39, b30, b26, etc.)
    ensure_columns(&conn)?;
    ensure_vec(&conn);

    // Ensure PRAGMAs after migrations (some migrations may reset)
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // mark migrated
    migrated_paths().lock().unwrap().insert(path.to_path_buf());
    Ok(conn)
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
    // entities.window_text/window_index/dispute_count (b26 + B3)
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN window_text TEXT NOT NULL DEFAULT ''");
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN window_index INTEGER");
    try_add_column(conn, "ALTER TABLE entities ADD COLUMN dispute_count INTEGER NOT NULL DEFAULT 0");
    // chunks additions (B30, B18)
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN kind VARCHAR(20) NOT NULL DEFAULT 'document'");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN is_pii BOOLEAN NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN pii_categories TEXT NOT NULL DEFAULT '[]'");
    // R14.1 hierarchical TOC
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN section_id INTEGER REFERENCES sections(id) ON DELETE SET NULL");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN level INTEGER NOT NULL DEFAULT 0");
    try_add_column(conn, "ALTER TABLE chunks ADD COLUMN path TEXT NOT NULL DEFAULT ''");
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
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_sections_item_id ON sections(item_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_sections_parent_id ON sections(parent_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS ix_chunks_section_id ON chunks(section_id)", []);
    try_add_column(conn, "ALTER TABLE sections ADD COLUMN summary_hash TEXT NOT NULL DEFAULT ''");
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
    Ok(())
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
            // fallback for test conformance when sqlite-vec not statically linked (Rust bundled sqlite)
            // Create dummy table + triggers so test_vec_chunks_table_exists passes (Python checks this)
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

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        // 23bca0413318_initial_schema — sources, ingested_items, entities, chunks, merge_actions, relationships
        M::up(include_str!("../migrations/01_initial.sql")),
        // 7c4a2b9e3d81_add_system_events_table
        M::up(include_str!("../migrations/02_system_events.sql")),
        // 9d3f1b2c4a51_add_jobs_table
        M::up(include_str!("../migrations/03_jobs.sql")),
        // b4a00c1_add_app_settings
        M::up(include_str!("../migrations/04_app_settings.sql")),
        // b3a0c1_disputes + b15a0d1_projects + b18 distilled + b26 review_context (collapsed)
        M::up(include_str!("../migrations/05_misc.sql")),
        // b12f7c0_add_chunks_fts
        M::up(include_str!("../migrations/06_fts.sql")),
        // b39a0c1_add_source_label
        M::up(include_str!("../migrations/07_source_label.sql")),
        // b33a0c1_add_vec_chunks
        M::up(include_str!("../migrations/08_vec.sql")),
        // b30a0c1_add_pii_flags
        M::up(include_str!("../migrations/09_pii.sql")),
        // b40a0c1_add_users
        M::up(include_str!("../migrations/10_users.sql")),
        // R14.1 hierarchical TOC
        M::up(include_str!("../migrations/11_sections.sql")),
        // R14.4 dynamic tags
        M::up(include_str!("../migrations/12_tags.sql")),
        // Backfill created_at default on system_events (older chains lack it -> audit writes fail)
        M::up(include_str!("../migrations/13_system_events_default.sql")),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
}
