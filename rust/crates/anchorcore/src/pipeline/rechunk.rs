//! R36.2: chunker generations per item.
//!
//! Every chunk write stamps `ingested_items.chunker_version` with the
//! generation `chunking::chunker_version` reports for the item's filename
//! (migration 28 starts every existing row at 1, the pre-R36.2 chunkers).
//! `rechunk_outdated_items` runs from `db::init_db` once the schema is
//! complete: it rebuilds every item whose stored generation is older than
//! its family's current one from the stored item text — no network, no
//! re-extraction, no chat call — so an upgrade never leaves old- and
//! new-granularity chunks side by side in one index.
//!
//! Each item is one transaction (chunks + sections + stamp + caption
//! re-pointing + summaries + tags). Vectors carry over through the usual
//! `(kind, path, content)` reuse key; chunks whose bytes or breadcrumb
//! changed are left without a vector for the embedding backfill (startup +
//! every 15 min), which honours the label gate. Items that host images are
//! left to their next sync (asset placement needs the document package).

use rusqlite::Connection;

use super::{create_summaries_inner, create_tags_inner, insert_doc_chunks, insert_sections, old_vectors, plan_doc_chunks, VectorReuse};

/// Stamp the generation that just chunked `item_id` (inside the caller's
/// chunk-write transaction).
pub(super) fn stamp_chunker_version(conn: &Connection, item_id: i64, filename: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE ingested_items SET chunker_version=?1 WHERE id=?2",
        rusqlite::params![crate::chunking::chunker_version(filename), item_id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

struct Outdated {
    id: i64,
    external_id: String,
    text: String,
    doc_type: String,
}

/// Rebuild the chunks of every item an older chunker produced. Returns how
/// many items were re-chunked; one failing item is logged and skipped (its
/// stamp stays old, so the next boot retries it).
pub fn rechunk_outdated_items(conn: &Connection) -> Result<usize, String> {
    let items = outdated_items(conn)?;
    let mut done = 0usize;
    // R41.7: the startup screen shows items done / total (no-op after boot).
    if !items.is_empty() {
        crate::startup::step_detail(&format!("Rebuilding search chunks for {} item(s)", items.len()));
    }
    for (i, item) in items.iter().enumerate() {
        crate::startup::progress(i as u64, items.len() as u64, "items");
        match rechunk_item(conn, item) {
            Ok(()) => done += 1,
            Err(e) => tracing::warn!("R36.2: re-chunk of item {} ({}) failed: {}", item.id, item.external_id, e),
        }
    }
    if done > 0 {
        tracing::info!("R36.2: re-chunked {} item(s) to the current chunker generation", done);
        let detail = format!(r#"{{"rechunked":{done},"outdated":{}}}"#, items.len());
        let _ = conn.execute(
            "INSERT INTO system_events (component, level, message, detail) VALUES ('migration', 'info', 'R36.2 chunker upgrade', ?1)",
            [detail],
        );
    }
    Ok(done)
}

/// Chunked items (they have document chunks) whose stored generation is
/// older than what their filename's family produces now.
fn outdated_items(conn: &Connection) -> Result<Vec<Outdated>, String> {
    let newest = crate::chunking::CONFIG_CHUNKER_VERSION.max(crate::chunking::BASE_CHUNKER_VERSION);
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.external_id, COALESCE(i.text, ''), COALESCE(i.doc_type, ''), i.chunker_version FROM ingested_items i \
             WHERE i.chunker_version < ?1 AND COALESCE(i.text, '') <> '' \
             AND EXISTS (SELECT 1 FROM chunks c WHERE c.item_id = i.id AND c.kind = 'document') ORDER BY i.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([newest], |r| {
            Ok((Outdated { id: r.get(0)?, external_id: r.get(1)?, text: r.get(2)?, doc_type: r.get(3)? }, r.get::<_, i64>(4)?))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|(it, stored)| crate::chunking::chunker_version(&it.external_id) > *stored && !crate::assets::hosts_images(&it.external_id))
        .map(|(it, _)| it)
        .collect())
}

/// One item, one transaction. Clean + chunk + the vector read run before
/// `BEGIN IMMEDIATE` (same rule as `store_doc_chunks`).
fn rechunk_item(conn: &Connection, item: &Outdated) -> Result<(), String> {
    let skip_clean = item.doc_type == "code"
        || crate::extract::is_config_filename(&item.external_id)
        || crate::markdown::is_markdown_filename(&item.external_id);
    let (metas, chunk_metas) = plan_doc_chunks(&item.doc_type, &item.external_id, &item.text, skip_clean);
    let reuse = old_vectors(conn, item.id)?;
    let base_ref = base_ref_of(conn, item.id)?.unwrap_or_else(|| item.external_id.clone());
    let plan = Plan { base_ref: &base_ref, metas: &metas, chunk_metas: &chunk_metas, reuse: &reuse };
    conn.execute("BEGIN IMMEDIATE", []).map_err(|e| e.to_string())?;
    let res = replace_item_chunks(conn, item, &plan);
    // `init_db` hands this connection to its caller: never leave a
    // transaction open on it.
    conn.execute(if res.is_ok() { "COMMIT" } else { "ROLLBACK" }, []).map_err(|e| e.to_string())?;
    res
}

struct Plan<'a> {
    base_ref: &'a str,
    metas: &'a [crate::chunking::SectionMeta],
    chunk_metas: &'a [crate::chunking::ChunkMeta],
    reuse: &'a VectorReuse,
}

/// The transaction body: the `store_doc_chunks` rebuild, then the finalize
/// phases that read the new sections (captions, summaries, tags).
fn replace_item_chunks(conn: &Connection, item: &Outdated, plan: &Plan) -> Result<(), String> {
    let _ = conn.execute("DELETE FROM chunks WHERE item_id=?1 AND entity_id IS NULL AND kind <> 'caption'", [item.id]);
    let _ = conn.execute("DELETE FROM sections WHERE item_id=?1", [item.id]);
    let section_ids = insert_sections(conn, item.id, plan.metas)?;
    insert_doc_chunks(conn, item.id, plan.base_ref, plan.chunk_metas, &section_ids, plan.reuse)?;
    stamp_chunker_version(conn, item.id, &item.external_id)?;
    crate::captions::refresh_for_item(conn, item.id)?;
    create_summaries_inner(conn, item.id, &item.text, plan.base_ref, false)?;
    restore_summary_vectors(conn, item.id, plan.reuse)?;
    create_tags_inner(conn, item.id, &item.text)
}

/// The item's citation base (`chunks.source_ref` minus the ` §N` suffix the
/// chunk writer appends), so rebuilt chunks cite the same place.
fn base_ref_of(conn: &Connection, item_id: i64) -> Result<Option<String>, String> {
    let first: Option<(String, i64)> = conn
        .query_row(
            "SELECT source_ref, (SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND kind='document') FROM chunks \
             WHERE item_id=?1 AND kind='document' ORDER BY id LIMIT 1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    Ok(first.map(|(r, n)| strip_ordinal(&r, n > 1)))
}

fn strip_ordinal(source_ref: &str, numbered: bool) -> String {
    if !numbered {
        return source_ref.to_string();
    }
    match source_ref.rsplit_once(" §") {
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => base.to_string(),
        _ => source_ref.to_string(),
    }
}

/// Summary chunks are rebuilt after the document chunks; unchanged ones get
/// their old vector back (sync twin of `restore_vectors`).
fn restore_summary_vectors(conn: &Connection, item_id: i64, reuse: &VectorReuse) -> Result<(), String> {
    let pending: Vec<(i64, (String, String, String))> = conn
        .prepare("SELECT id, kind, path, content FROM chunks WHERE item_id=?1 AND embedding IS NULL AND kind IN ('section_summary','doc_summary')")
        .and_then(|mut st| st.query_map([item_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?.collect())
        .map_err(|e| e.to_string())?;
    for (id, key) in pending {
        let Some((blob, fp)) = reuse.get(&key) else { continue };
        conn.execute("UPDATE chunks SET embedding=?1, embed_fp=?2 WHERE id=?3", rusqlite::params![blob, fp, id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const OLD_YAML: &str = "routes:\n  - item_type: payout\n    settle_code: STL-0042\n    target: ledger-eu-1\n  - item_type: refund\n    settle_code: STL-0107\n    target: ledger-eu-2\n";

    /// A config item chunked the pre-R36.2 way (every key a section),
    /// stamped generation 1 like migration 28 leaves it, plus a prose item.
    fn seed_old_index(conn: &Connection) -> (i64, i64) {
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, processed_hash) VALUES (?1,'cfg/routing.yaml','routing.yaml',?2,'h','h')", rusqlite::params![sid, OLD_YAML]).unwrap();
        let cfg = conn.last_insert_rowid();
        for (i, (path, content)) in [("routes", "routes:"), ("routes > item_type", "  - item_type: payout"), ("routes > settle_code", "    settle_code: STL-0042")].iter().enumerate() {
            conn.execute("INSERT INTO sections (item_id, level, title, path, created_at) VALUES (?1, 1, ?2, ?2, datetime('now'))", rusqlite::params![cfg, path]).unwrap();
            let sec = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO chunks (item_id, kind, source_ref, content, section_id, path, embedding, embed_fp) VALUES (?1,'document',?2,?3,?4,?5,x'0000803f','fam')",
                rusqlite::params![cfg, format!("/data/cfg/routing.yaml §{}", i + 1), content, sec, path],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, processed_hash) VALUES (?1,'notes.txt','notes.txt','plain notes','p','p')", [sid]).unwrap();
        let prose = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, path) VALUES (?1,'document','/data/notes.txt','plain notes','')", [prose]).unwrap();
        conn.execute("UPDATE ingested_items SET chunker_version = 1", []).unwrap();
        (cfg, prose)
    }

    #[test]
    fn outdated_config_items_rechunk_once_and_prose_is_untouched() {
        let dir = tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("rc.db")).unwrap();
        let (cfg, prose) = seed_old_index(&conn);
        assert_eq!(rechunk_outdated_items(&conn).unwrap(), 1, "only the config item is outdated");
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM sections WHERE item_id=?1 ORDER BY id")
            .unwrap()
            .query_map([cfg], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(paths, vec!["routes", "routes > item_type: payout, settle_code: STL-0042", "routes > item_type: refund, settle_code: STL-0107"]);
        let refs: Vec<String> = conn
            .prepare("SELECT source_ref FROM chunks WHERE item_id=?1 AND kind='document' ORDER BY id")
            .unwrap()
            .query_map([cfg], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(refs, vec!["/data/cfg/routing.yaml §1", "/data/cfg/routing.yaml §2"], "citation base kept");
        let versions: Vec<i64> = conn
            .prepare("SELECT chunker_version FROM ingested_items WHERE id IN (?1, ?2) ORDER BY id")
            .unwrap()
            .query_map([cfg, prose], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(versions, vec![crate::chunking::CONFIG_CHUNKER_VERSION, 1]);
        let old_left: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND path LIKE 'routes > settle_code%'", [cfg], |r| r.get(0)).unwrap();
        assert_eq!(old_left, 0, "no mixed granularity: every old chunk is gone");
        let summaries: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND kind='section_summary'", [cfg], |r| r.get(0)).unwrap();
        assert!(summaries > 0, "summaries rebuilt");
        let prose_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1", [prose], |r| r.get(0)).unwrap();
        assert_eq!(prose_chunks, 1);
        assert_eq!(rechunk_outdated_items(&conn).unwrap(), 0, "idempotent");
        let in_tx: bool = conn.is_autocommit();
        assert!(in_tx, "no transaction left open");
    }

    #[test]
    fn strip_ordinal_only_for_numbered_refs() {
        assert_eq!(strip_ordinal("/a/b.yaml §12", true), "/a/b.yaml");
        assert_eq!(strip_ordinal("/a/b.yaml", false), "/a/b.yaml");
        assert_eq!(strip_ordinal("/a/§x §y", true), "/a/§x §y");
    }
}
