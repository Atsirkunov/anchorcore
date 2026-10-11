//! R25.6 secrets bundle + R25.2 export engine — Phase A of `docs/plan-backup-sync.md`.
//!
//! `.anchorcore-backup` is a zip:
//! - `manifest.json` — format_version, app_version, platform, created_at, device,
//!   counts, embed_model/embed_dim, options used, `needs_secret`, folder paths
//! - `data.jsonl` — one typed JSON record per line (plan §3.1)
//! - `secrets.bundle` — optional, only with `include_secrets` + passphrase
//! - `README.txt` — what this is + the keep-the-passphrase warning
//!
//! Secrets are never plaintext in an export: the default carries only the
//! non-secret config plus `needs_secret` markers, and the opt-in bundle is
//! Argon2id (m=64 MiB, t=3, p=1) + XChaCha20-Poly1305 with the header bound as
//! AEAD associated data (R25.6). Records never carry local integer ids — uids
//! only — so an import can remap them.
//!
//! Records are rendered by SQLite's JSON functions (`json_object`) and streamed
//! straight into the zip entry, so nothing accumulates in memory and the record
//! shape stays declarative (no 15-field row mappers).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::secrets::{SecretStore, SECRET_SOURCE_FIELDS};

/// Export schema version — versioned independently of SQLite. A *newer*
/// archive than the running app is refused on import with "upgrade the app".
pub const FORMAT_VERSION: u32 = 1;
pub const ARCHIVE_SUFFIX: &str = ".anchorcore-backup";
const BUNDLE_MAGIC: &[u8; 6] = b"ACSB1\n";
const BUNDLE_VERSION: u32 = 1;
const ARGON2_M_COST: u32 = 65_536; // KiB → 64 MiB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const EXPORT_DIR: &str = "exports";
/// Finished archives in `exports/` are request-scoped; sweep stragglers.
const EXPORT_TTL_SECS: u64 = 3600;

type ArchiveWriter = zip::ZipWriter<BufWriter<std::fs::File>>;

// ---------------------------------------------------------------------------
// R25.6 — encrypted secrets bundle
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct BundleHeader {
    v: u32,
    kdf: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: String,
    nonce: String,
}

fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| "bundle header is not valid base64".to_string())
}

fn argon2_key(passphrase: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    let params = argon2::Params::new(m, t, p, Some(KEY_LEN)).map_err(|e| format!("argon2 params: {e}"))?;
    let kdf = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = zeroize::Zeroizing::new(vec![0u8; KEY_LEN]);
    kdf.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| format!("argon2: {e}"))?;
    Ok(key)
}

/// Seal `entries` (`key` → secret value) into a `secrets.bundle` byte string.
/// The key material and the plaintext are zeroized after use.
pub fn seal_bundle(passphrase: &str, entries: &[(String, String)]) -> Result<Vec<u8>, String> {
    if passphrase.trim().is_empty() {
        return Err("passphrase must not be empty".to_string());
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let header = BundleHeader {
        v: BUNDLE_VERSION,
        kdf: "argon2id".to_string(),
        m_cost: ARGON2_M_COST,
        t_cost: ARGON2_T_COST,
        p_cost: ARGON2_P_COST,
        salt: b64_encode(&salt),
        nonce: b64_encode(&nonce),
    };
    let header_bytes = serde_json::to_vec(&header).map_err(|e| e.to_string())?;
    let mut prefix = Vec::with_capacity(BUNDLE_MAGIC.len() + 4 + header_bytes.len());
    prefix.extend_from_slice(BUNDLE_MAGIC);
    prefix.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    prefix.extend_from_slice(&header_bytes);

    let payload: Vec<Value> = entries.iter().map(|(k, v)| json!({"k": k, "v": v})).collect();
    let bundle = json!({"v": BUNDLE_VERSION, "entries": payload});
    let plain = zeroize::Zeroizing::new(serde_json::to_vec(&bundle).map_err(|e| e.to_string())?);
    let key = argon2_key(passphrase, &salt, ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("cipher init: {e}"))?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain[..],
                aad: &prefix,
            },
        )
        .map_err(|_| "failed to encrypt secrets bundle".to_string())?;
    let mut out = prefix;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Validate magic + header and return it with the ciphertext offset.
fn parse_bundle_prefix(bytes: &[u8]) -> Result<(BundleHeader, usize), String> {
    if bytes.len() < BUNDLE_MAGIC.len() + 4 || !bytes.starts_with(BUNDLE_MAGIC) {
        return Err("not an AnchorCore secrets bundle".to_string());
    }
    let len_at = BUNDLE_MAGIC.len();
    let hlen = u32::from_le_bytes(bytes[len_at..len_at + 4].try_into().unwrap()) as usize;
    let header_start = len_at + 4;
    let header_end = header_start
        .checked_add(hlen)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "secrets bundle header is truncated".to_string())?;
    let header: BundleHeader =
        serde_json::from_slice(&bytes[header_start..header_end]).map_err(|e| format!("bundle header: {e}"))?;
    if header.v != BUNDLE_VERSION || header.kdf != "argon2id" {
        return Err(format!(
            "unsupported secrets bundle (version {}, kdf {}) — upgrade the app",
            header.v, header.kdf
        ));
    }
    // A hostile bundle must not be able to pick our Argon2 memory cost.
    if header.m_cost > ARGON2_M_COST || header.t_cost > 16 || header.p_cost > 8 {
        return Err("secrets bundle KDF parameters are out of range".to_string());
    }
    Ok((header, header_end))
}

/// Open a `secrets.bundle`. Wrong passphrase, tampering (header or ciphertext)
/// and unknown versions all fail without returning partial data.
pub fn open_bundle(passphrase: &str, bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let (header, header_end) = parse_bundle_prefix(bytes)?;
    let salt = b64_decode(&header.salt)?;
    let nonce = b64_decode(&header.nonce)?;
    if nonce.len() != NONCE_LEN {
        return Err("secrets bundle nonce has the wrong length".to_string());
    }
    let key = argon2_key(passphrase, &salt, header.m_cost, header.t_cost, header.p_cost)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| format!("cipher init: {e}"))?;
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &bytes[header_end..],
                aad: &bytes[..header_end],
            },
        )
        .map_err(|_| "cannot decrypt secrets bundle (wrong passphrase or damaged file)".to_string())?;
    let plain = zeroize::Zeroizing::new(plain);
    let doc: Value = serde_json::from_slice(&plain).map_err(|e| format!("bundle payload: {e}"))?;
    let entries = doc
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "bundle payload has no entries".to_string())?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let (Some(k), Some(v)) = (entry.get("k").and_then(|x| x.as_str()), entry.get("v").and_then(|x| x.as_str())) else {
            return Err("bundle entry is malformed".to_string());
        };
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// R25.2 — export
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportOptions {
    #[serde(default)]
    pub include_secrets: bool,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub include_embeddings: bool,
    #[serde(default)]
    pub include_ops: bool,
    #[serde(default)]
    pub project_id: Option<i64>,
}

impl ExportOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.include_secrets && self.passphrase.as_deref().unwrap_or("").trim().is_empty() {
            return Err("include_secrets requires a non-empty passphrase".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Counts {
    pub sources: u64,
    pub items: u64,
    pub entities: u64,
    pub relationships: u64,
    pub chunks: u64,
    pub sections: u64,
    pub tags: u64,
    pub chunk_tags: u64,
    pub projects: u64,
    pub disputes: u64,
    pub merge_actions: u64,
    pub settings: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub jobs: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub system_events: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub users: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Debug)]
pub struct ExportResult {
    pub path: PathBuf,
    pub filename: String,
    pub counts: Counts,
    pub needs_secret: u64,
}

struct ExportSummary {
    counts: Counts,
    /// (local source id, source uid, field) for `include_secrets`.
    secret_refs: Vec<(i64, String, String)>,
    /// `{source_uid, name, path}` for folder sources — remap hints on import.
    folder_paths: Vec<Value>,
    embed_model: Option<String>,
}

/// Optional project filter. `expr` is a source-id column expression and `pid`
/// is a parsed integer — this is never built from user-supplied text.
fn source_scope(pid: Option<i64>, expr: &str) -> String {
    match pid {
        Some(pid) => format!(
            " AND {} IN (SELECT source_id FROM project_sources WHERE project_id = {})",
            expr, pid
        ),
        None => String::new(),
    }
}

fn json_bool(col: &str) -> String {
    format!("CASE WHEN {col} THEN json('true') ELSE json('false') END")
}

fn write_str(zip: &mut ArchiveWriter, text: &str) -> Result<(), String> {
    zip.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

fn write_record_line(zip: &mut ArchiveWriter, record: &str) -> Result<(), String> {
    write_str(zip, record)?;
    write_str(zip, "\n")
}

/// Stream `SELECT json_object(...)` rows straight into the current zip entry.
/// One prepared statement, no per-row allocations beyond SQLite's own string.
fn write_jsonl_query(conn: &Connection, zip: &mut ArchiveWriter, sql: &str) -> Result<u64, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut n = 0u64;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let record: String = row.get(0).map_err(|e| e.to_string())?;
        write_record_line(zip, &record)?;
        n += 1;
    }
    Ok(n)
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

/// A secret to carry into the bundle: (local source id, source uid, field).
type SecretRef = (i64, String, String);

struct SourceExport {
    count: u64,
    secret_refs: Vec<SecretRef>,
    folder_paths: Vec<Value>,
}

struct SourceRecord {
    record: Value,
    secret_refs: Vec<SecretRef>,
    folder_path: Option<Value>,
}

/// (id, uid, connector, name, label, enabled, config, created_at)
type RawSource = (i64, Option<String>, String, String, String, bool, String, Option<String>);

fn write_json_record_line(zip: &mut ArchiveWriter, value: &Value) -> Result<(), String> {
    let line = serde_json::to_string(value).map_err(|e| e.to_string())?;
    write_record_line(zip, &line)
}

fn load_sources(conn: &Connection, opts: &ExportOptions) -> Result<Vec<RawSource>, String> {
    let scope = source_scope(opts.project_id, "s.id");
    let sql = format!(
        "SELECT s.id, s.uid, s.connector, s.name, s.label, s.enabled, s.config, s.created_at \
         FROM sources s WHERE 1=1{} ORDER BY s.id",
        scope
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn source_record(store: &SecretStore, raw: RawSource) -> SourceRecord {
    let (id, uid, connector, name, label, enabled, config_str, created_at) = raw;
    let uid = uid.unwrap_or_default();
    let config: Value = serde_json::from_str(&config_str).unwrap_or_else(|_| json!({}));
    let mut fields: Vec<&str> = Vec::new();
    let mut secret_refs = Vec::new();
    for &field in SECRET_SOURCE_FIELDS {
        if store.get(&crate::secrets::source_secret_key(id, field)).is_some() {
            fields.push(field);
            secret_refs.push((id, uid.clone(), field.to_string()));
        }
    }
    let folder_path = if connector == "folder" {
        config
            .get("path")
            .and_then(|v| v.as_str())
            .map(|path| json!({"source_uid": uid, "name": name, "path": path}))
    } else {
        None
    };
    SourceRecord {
        record: json!({
            "type": "source",
            "uid": uid,
            "connector": connector,
            "name": name,
            "label": label,
            "enabled": enabled,
            "config": config,
            "needs_secret": fields,
            "created_at": created_at,
        }),
        secret_refs,
        folder_path,
    }
}

fn export_sources(
    conn: &Connection,
    zip: &mut ArchiveWriter,
    opts: &ExportOptions,
    store: &SecretStore,
) -> Result<SourceExport, String> {
    let mut out = SourceExport {
        count: 0,
        secret_refs: Vec::new(),
        folder_paths: Vec::new(),
    };
    for raw in load_sources(conn, opts)? {
        let parsed = source_record(store, raw);
        write_json_record_line(zip, &parsed.record)?;
        out.secret_refs.extend(parsed.secret_refs);
        if let Some(folder) = parsed.folder_path {
            out.folder_paths.push(folder);
        }
        out.count += 1;
    }
    Ok(out)
}

/// Core records; each table is one JSON-rendering query.
fn export_core_counts(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions, sources: u64) -> Result<Counts, String> {
    Ok(Counts {
        sources,
        items: export_items(conn, zip, opts)?,
        entities: export_entities(conn, zip, opts)?,
        relationships: export_relationships(conn, zip, opts)?,
        chunks: export_chunks(conn, zip, opts)?,
        sections: export_sections(conn, zip, opts)?,
        tags: export_tags(conn, zip)?,
        chunk_tags: export_chunk_tags(conn, zip)?,
        projects: export_projects(conn, zip, opts)?,
        ..Default::default()
    })
}

fn export_items(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = source_scope(opts.project_id, "i.source_id");
    let sql = format!(
        "SELECT json_object('type','item','uid',i.uid,'source_uid',s.uid,'external_id',i.external_id, \
            'title',i.title,'doc_type',i.doc_type,'text',i.text,'content_hash',i.content_hash, \
            'author',i.author,'updated_at',i.updated_at,'stale',{stale},'created_at',i.created_at) \
         FROM ingested_items i JOIN sources s ON s.id = i.source_id WHERE 1=1{} ORDER BY i.id",
        scope,
        stale = json_bool("i.stale")
    );
    write_jsonl_query(conn, zip, &sql)
}

fn export_entities(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = source_scope(opts.project_id, "i.source_id");
    let sql = format!(
        "SELECT json_object('type','entity','uid',e.uid,'item_uid',i.uid,'kind',e.kind, \
            'summary',e.summary,'reasoning',e.reasoning,'confidence',e.confidence,'author',e.author, \
            'source_ref',e.source_ref,'window_text',e.window_text,'window_index',e.window_index, \
            'status',e.status,'owner',e.owner,'dispute_count',e.dispute_count, \
            'created_at',e.created_at,'updated_at',e.updated_at) \
         FROM entities e JOIN ingested_items i ON i.id = e.item_id WHERE 1=1{} ORDER BY e.id",
        scope
    );
    write_jsonl_query(conn, zip, &sql)
}

fn export_relationships(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = source_scope(opts.project_id, "i.source_id");
    let sql = format!(
        "SELECT json_object('type','relationship','from_uid',fr.uid,'to_uid',to_e.uid,'kind',r.kind, \
            'confidence',r.confidence,'source_ref',r.source_ref,'created_at',r.created_at) \
         FROM relationships r \
         JOIN entities fr ON fr.id = r.from_entity_id \
         JOIN entities to_e ON to_e.id = r.to_entity_id \
         JOIN ingested_items i ON i.id = fr.item_id WHERE 1=1{} ORDER BY r.id",
        scope
    );
    write_jsonl_query(conn, zip, &sql)
}

fn chunk_scope(opts: &ExportOptions) -> String {
    let Some(pid) = opts.project_id else {
        return String::new();
    };
    format!(
        " AND (i.source_id IN (SELECT source_id FROM project_sources WHERE project_id = {pid}) \
         OR en.item_id IN (SELECT id FROM ingested_items WHERE source_id IN \
         (SELECT source_id FROM project_sources WHERE project_id = {pid})))"
    )
}

fn export_chunks(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = chunk_scope(opts);
    let sql = format!(
        "SELECT json_object('type','chunk','uid',c.uid,'item_uid',i.uid,'entity_uid',en.uid, \
            'section_uid',sec.uid,'kind',c.kind,'source_ref',c.source_ref,'content',c.content, \
            'level',c.level,'path',c.path,'is_pii',{pii},'pii_categories',json(c.pii_categories), \
            'created_at',c.created_at) \
         FROM chunks c \
         LEFT JOIN ingested_items i ON i.id = c.item_id \
         LEFT JOIN entities en ON en.id = c.entity_id \
         LEFT JOIN sections sec ON sec.id = c.section_id \
         WHERE 1=1{} ORDER BY c.id",
        scope,
        pii = json_bool("c.is_pii")
    );
    write_jsonl_query(conn, zip, &sql)
}

fn export_sections(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = source_scope(opts.project_id, "i.source_id");
    let sql = format!(
        "SELECT json_object('type','section','uid',s.uid,'item_uid',i.uid,'parent_uid',p.uid, \
            'level',s.level,'title',s.title,'path',s.path,'chunk_range',s.chunk_range, \
            'summary',s.summary,'summary_hash',s.summary_hash,'created_at',s.created_at) \
         FROM sections s \
         JOIN ingested_items i ON i.id = s.item_id \
         LEFT JOIN sections p ON p.id = s.parent_id WHERE 1=1{} ORDER BY s.id",
        scope
    );
    write_jsonl_query(conn, zip, &sql)
}

fn export_tags(conn: &Connection, zip: &mut ArchiveWriter) -> Result<u64, String> {
    let sql = "SELECT json_object('type','tag','uid',uid,'name',name,'description',description, \
               'count',count,'created_at',created_at) FROM tags ORDER BY id";
    write_jsonl_query(conn, zip, sql)
}

fn export_chunk_tags(conn: &Connection, zip: &mut ArchiveWriter) -> Result<u64, String> {
    let sql = "SELECT json_object('type','chunk_tag','chunk_uid',c.uid,'tag_uid',t.uid) \
               FROM chunk_tags ct \
               JOIN chunks c ON c.id = ct.chunk_id \
               JOIN tags t ON t.id = ct.tag_id \
               WHERE c.uid IS NOT NULL AND t.uid IS NOT NULL ORDER BY ct.chunk_id";
    write_jsonl_query(conn, zip, sql)
}

fn export_projects(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<u64, String> {
    let scope = match opts.project_id {
        Some(pid) => format!(" WHERE p.id = {pid}"),
        None => String::new(),
    };
    let sql = format!(
        "SELECT json_object('type','project','uid',p.uid,'name',p.name,'is_default',{def}, \
            'source_uids',(SELECT json_group_array(s.uid) FROM project_sources ps \
                JOIN sources s ON s.id = ps.source_id WHERE ps.project_id = p.id), \
            'created_at',p.created_at) \
         FROM projects p{} ORDER BY p.id",
        scope,
        def = json_bool("p.is_default")
    );
    write_jsonl_query(conn, zip, &sql)
}

fn export_review_state(conn: &Connection, zip: &mut ArchiveWriter, opts: &ExportOptions) -> Result<(u64, u64), String> {
    let scope = source_scope(opts.project_id, "i.source_id");
    let disputes_sql = format!(
        "SELECT json_object('type','dispute','entity_uid',e.uid,'reason',d.reason,'user',d.user, \
            'created_at',d.created_at) \
         FROM disputes d JOIN entities e ON e.id = d.entity_id \
         JOIN ingested_items i ON i.id = e.item_id WHERE 1=1{} ORDER BY d.id",
        scope
    );
    let disputes = write_jsonl_query(conn, zip, &disputes_sql)?;
    let merges_sql = format!(
        "SELECT json_object('type','merge_action','entity_a_uid',a.uid,'entity_b_uid',b.uid, \
            'status',m.status,'reason',m.reason,'user',m.user,'created_at',m.created_at) \
         FROM merge_actions m \
         JOIN entities a ON a.id = m.entity_a_id \
         LEFT JOIN entities b ON b.id = m.entity_b_id \
         JOIN ingested_items i ON i.id = a.item_id WHERE 1=1{} ORDER BY m.id",
        scope
    );
    let merges = write_jsonl_query(conn, zip, &merges_sql)?;
    Ok((disputes, merges))
}

fn export_settings(conn: &Connection, zip: &mut ArchiveWriter) -> Result<u64, String> {
    let sql = "SELECT json_object('type','setting','key',key,'value',value) FROM app_settings \
               WHERE value != '' ORDER BY key";
    let secretish: Vec<String> = crate::settings::SECRET_KEYS.iter().map(|s| s.to_string()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut n = 0u64;
    for row in rows {
        let record = row.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&record).map_err(|e| e.to_string())?;
        let is_secret = value
            .get("key")
            .and_then(|k| k.as_str())
            .map(|k| secretish.iter().any(|s| s == k))
            .unwrap_or(false);
        if is_secret {
            continue;
        }
        write_record_line(zip, &record)?;
        n += 1;
    }
    Ok(n)
}

fn sql_value_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    match value {
        rusqlite::types::ValueRef::Null => Value::Null,
        rusqlite::types::ValueRef::Integer(i) => json!(i),
        rusqlite::types::ValueRef::Real(f) => json!(f),
        rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        rusqlite::types::ValueRef::Blob(b) => json!(b64_encode(b)),
    }
}

fn export_ops(conn: &Connection, zip: &mut ArchiveWriter, counts: &mut Counts) -> Result<(), String> {
    for table in ["jobs", "system_events", "users"] {
        if !table_exists(conn, table) {
            continue;
        }
        let n = export_ops_table(conn, zip, table)?;
        match table {
            "jobs" => counts.jobs = n,
            "system_events" => counts.system_events = n,
            _ => counts.users = n,
        }
    }
    Ok(())
}

fn export_ops_table(conn: &Connection, zip: &mut ArchiveWriter, table: &str) -> Result<u64, String> {
    let sql = format!("SELECT * FROM {} ORDER BY id", table);
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut n = 0u64;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut obj = serde_json::Map::new();
        for (idx, name) in cols.iter().enumerate() {
            let value = sql_value_json(row.get_ref(idx).map_err(|e| e.to_string())?);
            obj.insert(name.clone(), value);
        }
        obj.insert("type".to_string(), json!(format!("ops_{}", table)));
        let record = serde_json::to_string(&Value::Object(obj)).map_err(|e| e.to_string())?;
        write_record_line(zip, &record)?;
        n += 1;
    }
    Ok(n)
}

/// Embeddings ride in their own opt-in records keyed by uid (`embedding` /
/// `summary_embedding` columns have no JSON mapping in SQLite).
fn export_embeddings(conn: &Connection, zip: &mut ArchiveWriter, table: &str, column: &str, kind: &str) -> Result<u64, String> {
    let sql = format!("SELECT uid, {} FROM {} WHERE {} IS NOT NULL ORDER BY id", column, table, column);
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut n = 0u64;
    for row in rows {
        let (uid, blob) = row.map_err(|e| e.to_string())?;
        let (Some(uid), Some(blob)) = (uid, blob) else { continue };
        write_record_line(
            zip,
            &serde_json::to_string(&json!({"type": kind, "uid": uid, "embedding_b64": b64_encode(&blob)}))
                .map_err(|e| e.to_string())?,
        )?;
        n += 1;
    }
    Ok(n)
}

fn embed_model(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT value FROM app_settings WHERE key = 'embed_model'", [], |r| r.get(0))
        .ok()
        .filter(|v: &String| !v.is_empty())
}

fn embed_dim() -> u64 {
    std::env::var("ANCHOR_EMBED_DIM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(768)
}

fn export_optional(
    conn: &Connection,
    zip: &mut ArchiveWriter,
    opts: &ExportOptions,
    counts: &mut Counts,
    secret_refs: &[SecretRef],
    store: &SecretStore,
) -> Result<(), String> {
    if opts.include_embeddings {
        export_embeddings(conn, zip, "chunks", "embedding", "chunk_embedding")?;
        export_embeddings(conn, zip, "sections", "summary_embedding", "section_embedding")?;
        export_embeddings(conn, zip, "tags", "embedding", "tag_embedding")?;
    }
    if opts.include_ops {
        export_ops(conn, zip, counts)?;
    }
    if opts.include_secrets && !secret_refs.is_empty() {
        write_secrets_bundle(zip, opts, secret_refs, store)?;
    }
    Ok(())
}

fn write_secrets_bundle(
    zip: &mut ArchiveWriter,
    opts: &ExportOptions,
    secret_refs: &[SecretRef],
    store: &SecretStore,
) -> Result<(), String> {
    let passphrase = opts.passphrase.as_deref().unwrap_or("");
    let entries: Vec<(String, String)> = secret_refs
        .iter()
        .filter_map(|(id, uid, field)| {
            store
                .get(&crate::secrets::source_secret_key(*id, field))
                .map(|value| (format!("source:{}:{}", uid, field), value))
        })
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    let bundle = seal_bundle(passphrase, &entries)?;
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("secrets.bundle", options).map_err(|e| e.to_string())?;
    zip.write_all(&bundle).map_err(|e| e.to_string())
}

fn export_inner(
    conn: &Connection,
    zip: &mut ArchiveWriter,
    opts: &ExportOptions,
    store: &SecretStore,
) -> Result<ExportSummary, String> {
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("data.jsonl", options).map_err(|e| e.to_string())?;

    let source_export = export_sources(conn, zip, opts, store)?;
    let mut counts = export_core_counts(conn, zip, opts, source_export.count)?;
    (counts.disputes, counts.merge_actions) = export_review_state(conn, zip, opts)?;
    counts.settings = export_settings(conn, zip)?;
    export_optional(conn, zip, opts, &mut counts, &source_export.secret_refs, store)?;

    let summary = ExportSummary {
        counts,
        secret_refs: source_export.secret_refs,
        folder_paths: source_export.folder_paths,
        embed_model: embed_model(conn),
    };
    write_archive_tail(zip, opts, &summary)?;
    Ok(summary)
}

/// `README.txt` + `manifest.json` — written last so counts are known.
fn write_archive_tail(zip: &mut ArchiveWriter, opts: &ExportOptions, summary: &ExportSummary) -> Result<(), String> {
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("README.txt", options).map_err(|e| e.to_string())?;
    write_str(zip, &readme_text())?;

    let manifest = json!({
        "format_version": FORMAT_VERSION,
        "app_version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "device": device_name(),
        "counts": summary.counts,
        "embed_model": summary.embed_model,
        "embed_dim": embed_dim(),
        "options": {
            "include_secrets": opts.include_secrets,
            "include_embeddings": opts.include_embeddings,
            "include_ops": opts.include_ops,
            "project_id": opts.project_id,
        },
        "needs_secret": summary.secret_refs.iter()
            .map(|(_, uid, field)| json!({"source_uid": uid, "field": field}))
            .collect::<Vec<_>>(),
        "folder_paths": summary.folder_paths,
    });
    zip.start_file("manifest.json", options).map_err(|e| e.to_string())?;
    write_str(zip, &serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?)?;
    Ok(())
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn readme_text() -> String {
    format!(
        "AnchorCore backup — {}\n\
         ============================\n\n\
         This file is a portable AnchorCore export (format version {}).\n\n\
         To restore it: open AnchorCore -> System tab -> Backup & restore -> Import,\n\
         or run: anchorcore import --file <this file>\n\n\
         What is inside:\n\
         - manifest.json  — version, counts, options, which sources need credentials\n\
         - data.jsonl     — your memory: sources, documents, entities, chunks, review state\n\
         - secrets.bundle — only if the export included secrets; it is encrypted with\n           your passphrase and CANNOT be recovered if you lose the passphrase.\n\n\
         Secrets are not stored in plaintext. After importing you may need to\n\
         re-enter credentials for the sources listed in manifest.json.\n\
         Embeddings are excluded by default and are recomputed on the new device;\n\
         until that finishes, answers fall back to keyword-only and say so.\n",
        env!("CARGO_PKG_VERSION"),
        FORMAT_VERSION
    )
}

fn sweep_old_exports(data_dir: &Path) {
    let dir = data_dir.join(EXPORT_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(EXPORT_TTL_SECS);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Build `.anchorcore-backup` into `data_dir/exports/` and return where it is.
/// The caller streams the file; the sweep removes stragglers after an hour.
pub fn export_to_file(data_dir: &Path, opts: &ExportOptions) -> Result<ExportResult, String> {
    opts.validate()?;
    let db_path = crate::db::resolve_db_path(&data_dir.to_string_lossy());
    let conn = crate::db::init_db(&db_path).map_err(|e| e.to_string())?;
    // No record may leave without its stable id (R25.1).
    let _ = crate::db::backfill_uids(&conn);
    sweep_old_exports(data_dir);
    let exports_dir = data_dir.join(EXPORT_DIR);
    std::fs::create_dir_all(&exports_dir).map_err(|e| e.to_string())?;
    let filename = format!(
        "anchorcore-backup-{}-{:08x}{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        rand::random::<u32>(),
        ARCHIVE_SUFFIX
    );
    let path = exports_dir.join(&filename);
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    let store = SecretStore::new(data_dir.join("secrets.enc"));

    // One WAL read snapshot: every table is read as of the same instant even
    // while ingestion keeps writing.
    conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let inner = export_inner(&conn, &mut zip, opts, &store);
    let _ = conn.execute_batch("COMMIT");
    let summary = match inner {
        Ok(summary) => summary,
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            record_export_event(&conn, "error", &format!("export failed: {e}"));
            return Err(e);
        }
    };
    zip.finish().map_err(|e| e.to_string())?.into_inner().map_err(|e| e.to_string())?;

    let needs_secret = summary.secret_refs.len() as u64;
    record_export_event(
        &conn,
        "info",
        &serde_json::to_string(&json!({
            "file": filename,
            "counts": summary.counts,
            "include_secrets": opts.include_secrets,
            "include_embeddings": opts.include_embeddings,
            "include_ops": opts.include_ops,
            "project_id": opts.project_id,
            "needs_secret": needs_secret,
            "embed_model": summary.embed_model,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    );
    Ok(ExportResult {
        path,
        filename,
        counts: summary.counts,
        needs_secret,
    })
}

fn record_export_event(conn: &Connection, level: &str, detail: &str) {
    let message = if level == "error" { "export failed" } else { "export completed" };
    let _ = conn.execute(
        "INSERT INTO system_events (component, level, message, detail) VALUES ('backup', ?1, ?2, ?3)",
        rusqlite::params![level, message, detail],
    );
}

// ---------------------------------------------------------------------------
// HTTP handler
// ---------------------------------------------------------------------------

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;

use crate::health::AppState;

pub async fn export_handler(
    State(state): State<AppState>,
    Json(opts): Json<ExportOptions>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    if let Err(msg) = opts.validate() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": msg}))));
    }
    let data_dir = PathBuf::from(&state.data_dir);
    let result = tokio::task::spawn_blocking(move || export_to_file(&data_dir, &opts))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))))?;
    let result = match result {
        Ok(r) => r,
        Err(msg) => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": msg})))),
    };
    let file = tokio::fs::File::open(&result.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))))?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", result.filename),
        )
        .header("x-anchorcore-format-version", FORMAT_VERSION.to_string())
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Read;
    use tempfile::tempdir;

    // Planted in *secret* values only: the plan §7 grep test asserts neither the
    // token nor the phrase is ever readable in an exported archive.
    const MARKER_TOKEN: &str = "sk-live-MOONVAULT-TOKEN-4f9a";
    const MARKER_PHRASE: &str = "MOONVAULT-MARKER-PHRASE";

    /// Serializes the process-global `ANCHOR_SECRETS_NO_KEYRING` mutation with
    /// the other test that touches it (secrets.rs), so a test never ends up
    /// writing planted markers into the OS keychain.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn file_only() -> Self {
            let lock = crate::secrets::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("ANCHOR_SECRETS_NO_KEYRING").ok();
            std::env::set_var("ANCHOR_SECRETS_NO_KEYRING", "1");
            Self { _lock: lock, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("ANCHOR_SECRETS_NO_KEYRING", v),
                None => std::env::remove_var("ANCHOR_SECRETS_NO_KEYRING"),
            }
        }
    }

    fn contains(hay: &[u8], needle: &str) -> bool {
        !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    fn read_archive(path: &Path) -> (Vec<u8>, BTreeMap<String, Vec<u8>>) {
        let raw = std::fs::read(path).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(raw.clone())).unwrap();
        let mut entries = BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            entries.insert(name, buf);
        }
        (raw, entries)
    }

    fn assert_no_marker(entries: &BTreeMap<String, Vec<u8>>, raw: &[u8]) {
        for (name, bytes) in entries {
            assert!(!contains(bytes, MARKER_TOKEN), "{name} leaked the secret token");
            assert!(!contains(bytes, MARKER_PHRASE), "{name} leaked the secret phrase");
        }
        assert!(!contains(raw, MARKER_TOKEN), "raw archive bytes leaked the secret token");
        assert!(!contains(raw, MARKER_PHRASE), "raw archive bytes leaked the secret phrase");
    }

    fn jsonl_records(entries: &BTreeMap<String, Vec<u8>>) -> Vec<Value> {
        let text = String::from_utf8(entries["data.jsonl"].clone()).unwrap();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    /// One source with secrets plus one record of every type, inserted the way
    /// older builds do (no uid) so the export also exercises the backfill.
    fn seed(dir: &Path) -> i64 {
        let db_path = crate::db::resolve_db_path(&dir.to_string_lossy());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute(
            "INSERT INTO sources (name, connector, config, label) VALUES ('payments','folder','{\"path\":\"C:\\\\Docs\\\\payments\"}','internal')",
            [],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'payments.md','Payments overview','Payments run on the ledger service every night.','hash1')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO entities (item_id, kind, summary, status, owner) VALUES (?1,'decision','Nightly ledger settlement','verified','ops')",
            [iid],
        )
        .unwrap();
        let eid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','payments.md','Payments run on the ledger service every night.')",
            [iid],
        )
        .unwrap();
        let cid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sections (item_id, level, title, path) VALUES (?1,1,'Payments','Payments')",
            [iid],
        )
        .unwrap();
        let secid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, source_ref, content, level, path) VALUES (?1,?2,'section_summary','payments.md','Payments summary',1,'Payments')",
            rusqlite::params![iid, secid],
        )
        .unwrap();
        conn.execute("INSERT INTO tags (name, description) VALUES ('payments','money movement')", []).unwrap();
        let tid = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![cid, tid]).unwrap();
        conn.execute("INSERT INTO projects (name, is_default) VALUES ('Payments project',1)", []).unwrap();
        let pid = conn.last_insert_rowid();
        conn.execute("INSERT INTO project_sources (project_id, source_id) VALUES (?1,?2)", rusqlite::params![pid, sid]).unwrap();
        conn.execute("INSERT INTO disputes (entity_id, reason, user) VALUES (?1,'needs evidence','qa')", [eid]).unwrap();
        conn.execute("INSERT INTO merge_actions (entity_a_id, entity_b_id, status, reason) VALUES (?1,?1,'proposed','duplicate-ish')", [eid]).unwrap();
        conn.execute("INSERT INTO app_settings (key, value) VALUES ('answer_model','llama3')", []).unwrap();
        conn.execute("INSERT INTO app_settings (key, value) VALUES ('answer_api_key','sk-live-SETTINGS-never-exported')", []).unwrap();
        let store = SecretStore::new(dir.join("secrets.enc"));
        store.set(&crate::secrets::source_secret_key(sid, "token"), MARKER_TOKEN);
        store.set(&crate::secrets::source_secret_key(sid, "password"), MARKER_PHRASE);
        sid
    }

    #[test]
    fn export_without_secrets_has_no_plaintext_markers() {
        let _env = EnvGuard::file_only();
        let dir = tempdir().unwrap();
        seed(dir.path());
        let res = export_to_file(dir.path(), &ExportOptions::default()).unwrap();
        let (raw, entries) = read_archive(&res.path);
        assert_no_marker(&entries, &raw);

        // manifest: format, counts, credentials to re-enter, folder remap hint
        let manifest: Value = serde_json::from_slice(&entries["manifest.json"]).unwrap();
        assert_eq!(manifest["format_version"], FORMAT_VERSION);
        assert_eq!(manifest["counts"]["sources"], 1);
        assert_eq!(manifest["counts"]["items"], 1);
        assert_eq!(manifest["counts"]["entities"], 1);
        assert_eq!(manifest["counts"]["chunks"], 2);
        assert_eq!(manifest["counts"]["sections"], 1);
        assert_eq!(manifest["counts"]["tags"], 1);
        assert_eq!(manifest["counts"]["chunk_tags"], 1);
        assert_eq!(manifest["counts"]["projects"], 1);
        assert_eq!(manifest["counts"]["disputes"], 1);
        assert_eq!(manifest["counts"]["merge_actions"], 1);
        let needs: Vec<Value> = manifest["needs_secret"].as_array().unwrap().clone();
        assert_eq!(needs.len(), 2, "{manifest}");
        assert_eq!(manifest["folder_paths"][0]["path"], "C:\\Docs\\payments");
        assert_eq!(manifest["options"]["include_secrets"], false);
        assert!(!entries.contains_key("secrets.bundle"));
        assert!(entries.contains_key("README.txt"));

        // data.jsonl: typed records with uids, no secret-bearing config keys
        let records = jsonl_records(&entries);
        let source = records.iter().find(|r| r["type"] == "source").unwrap();
        assert_eq!(source["uid"].as_str().unwrap().len(), 36);
        assert!(!source["config"].as_object().unwrap().contains_key("token"));
        assert_eq!(source["needs_secret"], json!(["token", "password"]));
        assert!(records.iter().any(|r| r["type"] == "item" && r["text"].as_str().unwrap().contains("ledger service")));
        assert!(records.iter().any(|r| r["type"] == "entity" && r["status"] == "verified"));
        assert!(records.iter().any(|r| r["type"] == "chunk" && r["kind"] == "document"));
        assert!(records.iter().any(|r| r["type"] == "section"));
        assert!(records.iter().any(|r| r["type"] == "tag"));
        assert!(records.iter().any(|r| r["type"] == "chunk_tag"));
        assert!(records.iter().any(|r| r["type"] == "project"));
        assert!(records.iter().any(|r| r["type"] == "dispute"));
        assert!(records.iter().any(|r| r["type"] == "merge_action"));
        assert!(records.iter().any(|r| r["type"] == "setting" && r["key"] == "answer_model"));
        // the API-key *setting* is a secret and must not be exported
        assert!(!records.iter().any(|r| r["key"] == "answer_api_key"));
        // entity/chunk records reference uids, never local ids
        assert!(!records.iter().any(|r| r.get("id").is_some()));

        // audit event recorded
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy());
        let conn = crate::db::init_db(&db_path).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM system_events WHERE component='backup' AND message='export completed'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn export_with_secrets_keeps_markers_only_encrypted() {
        let _env = EnvGuard::file_only();
        let dir = tempdir().unwrap();
        let sid = seed(dir.path());
        let opts = ExportOptions {
            include_secrets: true,
            passphrase: Some("correct horse battery".to_string()),
            ..Default::default()
        };
        let res = export_to_file(dir.path(), &opts).unwrap();
        let (raw, entries) = read_archive(&res.path);
        assert_no_marker(&entries, &raw);
        let bundle = entries.get("secrets.bundle").expect("bundle with include_secrets");

        let opened = open_bundle("correct horse battery", bundle).unwrap();
        let uid: String = {
            let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy());
            let conn = crate::db::init_db(&db_path).unwrap();
            conn.query_row("SELECT uid FROM sources WHERE id = ?1", [sid], |r| r.get(0)).unwrap()
        };
        assert!(opened
            .iter()
            .any(|(k, v)| k == &format!("source:{}:token", uid) && v == MARKER_TOKEN));
        assert!(opened
            .iter()
            .any(|(k, v)| k == &format!("source:{}:password", uid) && v == MARKER_PHRASE));

        // wrong passphrase and tampering fail without partial data
        assert!(open_bundle("wrong passphrase", bundle).is_err());
        let mut flip_tail = bundle.clone();
        let last = flip_tail.len() - 1;
        flip_tail[last] ^= 0x01;
        assert!(open_bundle("correct horse battery", &flip_tail).is_err());
        let mut flip_header = bundle.clone();
        flip_header[8] ^= 0x01;
        assert!(open_bundle("correct horse battery", &flip_header).is_err());
        assert!(open_bundle("correct horse battery", b"not a bundle at all").is_err());
    }

    #[test]
    fn include_secrets_requires_passphrase() {
        let _env = EnvGuard::file_only();
        let dir = tempdir().unwrap();
        seed(dir.path());
        let opts = ExportOptions {
            include_secrets: true,
            passphrase: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(opts.validate().is_err());
        assert!(export_to_file(dir.path(), &opts).is_err());
    }

    #[test]
    fn embeddings_are_excluded_by_default_and_included_on_flag() {
        let _env = EnvGuard::file_only();
        let dir = tempdir().unwrap();
        seed(dir.path());
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("UPDATE chunks SET embedding = ?1 WHERE kind = 'document'", [vec![1u8, 2, 3, 4]])
            .unwrap();

        let plain = export_to_file(dir.path(), &ExportOptions::default()).unwrap();
        let (_, entries) = read_archive(&plain.path);
        let records = jsonl_records(&entries);
        assert!(!records.iter().any(|r| r["type"] == "chunk_embedding"));

        let with = export_to_file(
            dir.path(),
            &ExportOptions {
                include_embeddings: true,
                ..Default::default()
            },
        )
        .unwrap();
        let (_, entries) = read_archive(&with.path);
        let records = jsonl_records(&entries);
        let embedded: Vec<&Value> = records.iter().filter(|r| r["type"] == "chunk_embedding").collect();
        assert_eq!(embedded.len(), 1);
        assert_eq!(embedded[0]["embedding_b64"], b64_encode(&[1u8, 2, 3, 4]));
    }

    #[test]
    fn bundle_round_trip() {
        let entries = vec![
            ("source:abc:token".to_string(), "tok-123".to_string()),
            ("source:def:api_key".to_string(), "key-456".to_string()),
        ];
        let sealed = seal_bundle("pw", &entries).unwrap();
        assert!(sealed.starts_with(BUNDLE_MAGIC));
        assert_eq!(open_bundle("pw", &sealed).unwrap(), entries);
        assert!(open_bundle("", &sealed).is_err());
    }
}
