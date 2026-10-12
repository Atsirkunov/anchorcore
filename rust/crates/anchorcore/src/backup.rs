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

use std::collections::BTreeMap;
use std::io::{BufRead, BufWriter, Read, Write};
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
    secret_refs: Vec<SecretRef>,
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
#[derive(Clone)]
struct SecretRef {
    id: i64,
    uid: String,
    name: String,
    field: String,
}

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
            secret_refs.push(SecretRef { id, uid: uid.clone(), name: name.clone(), field: field.to_string() });
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
            'author',i.author,'updated_at',i.updated_at,'stale',{stale},'is_pii',{is_pii},'created_at',i.created_at) \
         FROM ingested_items i JOIN sources s ON s.id = i.source_id WHERE 1=1{} ORDER BY i.id",
        scope,
        stale = json_bool("i.stale"),
        is_pii = json_bool("i.is_pii")
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
            'confidence',r.confidence,'source_ref',r.source_ref,'note',r.note,'created_at',r.created_at) \
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
        .filter_map(|s| {
            store
                .get(&crate::secrets::source_secret_key(s.id, &s.field))
                .map(|value| (format!("source:{}:{}", s.uid, s.field), value))
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
            .map(|s| json!({"source_uid": s.uid, "source_name": s.name, "field": s.field}))
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
        // A pre-replace safety backup is never swept — it is the undo button.
        if entry.file_name().to_string_lossy().starts_with("safety-backup") {
            continue;
        }
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
// R25.3 — import
// ---------------------------------------------------------------------------

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DATA_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
/// A zip entry whose uncompressed size dwarfs its compressed size is a bomb.
const MAX_COMPRESSION_RATIO: u64 = 2000;
const MAX_CONFLICTS_REPORTED: usize = 50;
/// The typed confirmation `replace` requires before it wipes local data.
pub const REPLACE_CONFIRM: &str = "replace";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    #[default]
    Merge,
    Replace,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportOptions {
    #[serde(default)]
    pub mode: ImportMode,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
    /// source uid → new folder path for this machine.
    #[serde(default)]
    pub remap: BTreeMap<String, String>,
}

impl ImportOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.mode == ImportMode::Replace && !self.dry_run && self.confirm.as_deref() != Some(REPLACE_CONFIRM) {
            return Err(format!(
                "replace requires confirm=\"{}\" — it deletes local data (after an automatic safety backup)",
                REPLACE_CONFIRM
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub dry_run: bool,
    pub mode: String,
    pub format_version: u32,
    pub app_version: String,
    pub created_at: String,
    /// Records counted in the archive, per type.
    pub counts: BTreeMap<String, u64>,
    /// Records written (insert + update) per type; on a dry run, would-write.
    pub imported: BTreeMap<String, u64>,
    /// Existing records kept (uid present, incoming not newer).
    pub skipped: BTreeMap<String, u64>,
    pub conflicts: Vec<Value>,
    pub needs_secret: Vec<Value>,
    pub folder_paths: Vec<Value>,
    pub remap_applied: Vec<Value>,
    pub secrets_imported: u64,
    pub secrets_note: Option<String>,
    pub safety_backup: Option<String>,
    pub reembed_source_ids: Vec<i64>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Insert,
    Update,
    Skip,
}

fn decide(exists: bool, incoming: Option<&str>, existing: Option<&str>) -> Action {
    if !exists {
        return Action::Insert;
    }
    if is_newer(incoming, existing) {
        Action::Update
    } else {
        Action::Skip
    }
}

/// Timestamps are SQLite `datetime('now')` strings; lexicographic comparison is
/// the Phase A LWW rule (Phase B replaces it with a hybrid logical clock).
fn is_newer(incoming: Option<&str>, existing: Option<&str>) -> bool {
    match (incoming, existing) {
        (Some(i), Some(e)) => i > e,
        (Some(_), None) => true,
        _ => false,
    }
}

struct ImportCtx<'a> {
    conn: &'a Connection,
    write: bool,
    caches: BTreeMap<&'static str, BTreeMap<String, i64>>,
    imported: BTreeMap<String, u64>,
    skipped: BTreeMap<String, u64>,
    conflicts: Vec<Value>,
    warnings: Vec<String>,
    remap: BTreeMap<String, String>,
    remapped: Vec<Value>,
}

impl<'a> ImportCtx<'a> {
    fn new(conn: &'a Connection, write: bool, remap: BTreeMap<String, String>) -> Self {
        Self {
            conn,
            write,
            caches: BTreeMap::new(),
            imported: BTreeMap::new(),
            skipped: BTreeMap::new(),
            conflicts: Vec::new(),
            warnings: Vec::new(),
            remap,
            remapped: Vec::new(),
        }
    }

    /// Local id for a uid, cached. `table` is an internal constant.
    fn lookup(&mut self, table: &'static str, uid: &str) -> Result<Option<i64>, String> {
        if uid.is_empty() {
            return Ok(None);
        }
        if let Some(id) = self.caches.get(table).and_then(|m| m.get(uid)) {
            return Ok(Some(*id));
        }
        let sql = format!("SELECT id FROM {} WHERE uid = ?1", table);
        let found: Option<i64> = self.conn.query_row(&sql, [uid], |r| r.get(0)).ok();
        if let Some(id) = found {
            self.caches.entry(table).or_default().insert(uid.to_string(), id);
        }
        Ok(found)
    }

    fn require(&mut self, table: &'static str, uid: &str, kind: &str) -> Result<i64, String> {
        self.lookup(table, uid)?
            .ok_or_else(|| format!("archive references an unknown {kind} uid: {uid}"))
    }

    fn remember(&mut self, table: &'static str, uid: &str, id: i64) {
        self.caches.entry(table).or_default().insert(uid.to_string(), id);
    }

    /// Count the record and, on a merge conflict, note how it was resolved.
    fn record(&mut self, kind: &str, uid: &str, action: Action) {
        match action {
            Action::Skip => *self.skipped.entry(kind.to_string()).or_default() += 1,
            _ => *self.imported.entry(kind.to_string()).or_default() += 1,
        }
        if action != Action::Insert && self.conflicts.len() < MAX_CONFLICTS_REPORTED {
            self.conflicts.push(json!({
                "type": kind,
                "uid": uid,
                "action": if action == Action::Skip { "kept-local" } else { "imported" },
            }));
        }
    }

    fn stamp_of(&self, sql: &str, id: i64) -> Option<String> {
        self.conn
            .query_row(sql, [id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
    }

    /// Apply a folder-path remap (plan §2: paths are machine-local).
    fn remap_config(&mut self, uid: &str, mut config: Value) -> Value {
        let Some(new_path) = self.remap.get(uid).cloned() else {
            return config;
        };
        let Some(obj) = config.as_object_mut() else {
            return config;
        };
        let old = obj.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        obj.insert("path".to_string(), json!(new_path));
        self.remapped.push(json!({"source_uid": uid, "from": old, "to": new_path}));
        config
    }
}

fn str_field(rec: &Value, key: &str) -> String {
    rec.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn opt_str_field(rec: &Value, key: &str) -> Option<String> {
    rec.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn bool_field(rec: &Value, key: &str) -> bool {
    rec.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn bool_int(rec: &Value, key: &str) -> i64 {
    if bool_field(rec, key) {
        1
    } else {
        0
    }
}

fn i64_field(rec: &Value, key: &str) -> Option<i64> {
    rec.get(key).and_then(|v| v.as_i64())
}

fn f64_field(rec: &Value, key: &str) -> f64 {
    rec.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn embed_dim_bytes() -> usize {
    embed_dim() as usize * 4
}

/// Decode an `embedding_b64` field, dropping vectors from another embed model
/// (they would silently poison the vec0 index — the manifest records
/// `embed_model`/`embed_dim` so a mismatch is visible, not silent).
fn embedding_field(ctx: &mut ImportCtx, rec: &Value, key: &str, kind: &str) -> Option<Vec<u8>> {
    let encoded = rec.get(key).and_then(|v| v.as_str())?;
    match b64_decode(encoded) {
        Ok(blob) if blob.len() == embed_dim_bytes() => Some(blob),
        Ok(blob) => {
            ctx.warnings.push(format!(
                "{kind} {}: embedding is {} bytes, this app uses {} — dropped, will be recomputed",
                str_field(rec, "uid"),
                blob.len(),
                embed_dim_bytes()
            ));
            None
        }
        Err(_) => {
            ctx.warnings.push(format!("{kind} {}: embedding_b64 is not valid base64 — dropped", str_field(rec, "uid")));
            None
        }
    }
}

// --- per-record apply functions -------------------------------------------

fn apply_source(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let existing = ctx.lookup("sources", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT created_at FROM sources WHERE id=?1", id));
    let action = decide(existing.is_some(), opt_str_field(rec, "created_at").as_deref(), existing_stamp.as_deref());
    ctx.record("source", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let name = str_field(rec, "name");
    let connector = str_field(rec, "connector");
    let label = {
        let l = str_field(rec, "label");
        if l.is_empty() { "internal".to_string() } else { l }
    };
    let enabled = bool_int(rec, "enabled");
    let config = ctx.remap_config(&uid, rec.get("config").cloned().unwrap_or_else(|| json!({})));
    let config_str = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO sources (name, connector, config, label, enabled, last_sync_cursor, error_count, uid, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, '', 0, ?6, COALESCE(?7, datetime('now')))",
                rusqlite::params![name, connector, config_str, label, enabled, uid, opt_str_field(rec, "created_at")],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("sources", &uid, id);
        return Ok(());
    }
    let id = existing.unwrap();
    ctx.conn
        .execute(
            "UPDATE sources SET name=?1, connector=?2, config=?3, label=?4, enabled=?5 WHERE id=?6",
            rusqlite::params![name, connector, config_str, label, enabled, id],
        )
        .map_err(|e| e.to_string())?;
    ctx.remember("sources", &uid, id);
    Ok(())
}

fn apply_item(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let source_id = ctx.require("sources", &str_field(rec, "source_uid"), "source")?;
    let existing = ctx.lookup("ingested_items", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT COALESCE(updated_at, created_at) FROM ingested_items WHERE id=?1", id));
    let stamp = opt_str_field(rec, "updated_at").or_else(|| opt_str_field(rec, "created_at"));
    let action = decide(existing.is_some(), stamp.as_deref(), existing_stamp.as_deref());
    ctx.record("item", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let external_id = str_field(rec, "external_id");
    let title = str_field(rec, "title");
    let text = str_field(rec, "text");
    let hash = str_field(rec, "content_hash");
    let author = str_field(rec, "author");
    let doc_type = str_field(rec, "doc_type");
    let stale = bool_int(rec, "stale");
    // R35.1: manual PII mark round-trips with the item (missing in old
    // archives reads as unmarked).
    let is_pii = bool_int(rec, "is_pii");
    if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, doc_type, stale, is_pii, uid, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
                rusqlite::params![source_id, external_id, title, text, hash, author, doc_type, stale, is_pii, uid],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("ingested_items", &uid, id);
        return Ok(());
    }
    let id = existing.unwrap();
    ctx.conn
        .execute(
            "UPDATE ingested_items SET external_id=?2, title=?3, text=?4, content_hash=?5, author=?6, doc_type=?7, stale=?8, is_pii=?9, updated_at=datetime('now') WHERE id=?1",
            rusqlite::params![id, external_id, title, text, hash, author, doc_type, stale, is_pii],
        )
        .map_err(|e| e.to_string())?;
    ctx.remember("ingested_items", &uid, id);
    Ok(())
}

fn apply_section(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let item_id = ctx.require("ingested_items", &str_field(rec, "item_uid"), "item")?;
    let parent_id = {
        let parent_uid = str_field(rec, "parent_uid");
        if parent_uid.is_empty() {
            None
        } else {
            Some(ctx.require("sections", &parent_uid, "section")?)
        }
    };
    let existing = ctx.lookup("sections", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT created_at FROM sections WHERE id=?1", id));
    let action = decide(existing.is_some(), opt_str_field(rec, "created_at").as_deref(), existing_stamp.as_deref());
    ctx.record("section", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let level = i64_field(rec, "level").unwrap_or(0);
    let title = str_field(rec, "title");
    let path = str_field(rec, "path");
    let chunk_range = str_field(rec, "chunk_range");
    let summary = str_field(rec, "summary");
    let summary_hash = str_field(rec, "summary_hash");
    if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO sections (item_id, parent_id, level, title, path, chunk_range, summary, summary_hash, uid, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
                rusqlite::params![item_id, parent_id, level, title, path, chunk_range, summary, summary_hash, uid],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("sections", &uid, id);
        return Ok(());
    }
    let id = existing.unwrap();
    ctx.conn
        .execute(
            "UPDATE sections SET parent_id=?2, level=?3, title=?4, path=?5, chunk_range=?6, summary=?7, summary_hash=?8 WHERE id=?1",
            rusqlite::params![id, parent_id, level, title, path, chunk_range, summary, summary_hash],
        )
        .map_err(|e| e.to_string())?;
    ctx.remember("sections", &uid, id);
    Ok(())
}

fn apply_entity(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let item_id = ctx.require("ingested_items", &str_field(rec, "item_uid"), "item")?;
    let existing = ctx.lookup("entities", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT COALESCE(updated_at, created_at) FROM entities WHERE id=?1", id));
    let stamp = opt_str_field(rec, "updated_at").or_else(|| opt_str_field(rec, "created_at"));
    let action = decide(existing.is_some(), stamp.as_deref(), existing_stamp.as_deref());
    ctx.record("entity", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let kind = str_field(rec, "kind");
    let summary = str_field(rec, "summary");
    let reasoning = str_field(rec, "reasoning");
    let confidence = f64_field(rec, "confidence");
    let author = str_field(rec, "author");
    let source_ref = str_field(rec, "source_ref");
    let window_text = str_field(rec, "window_text");
    let window_index = i64_field(rec, "window_index");
    let status = {
        let s = str_field(rec, "status");
        if s.is_empty() { "unverified".to_string() } else { s }
    };
    let owner = str_field(rec, "owner");
    let dispute_count = i64_field(rec, "dispute_count").unwrap_or(0);
    if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, dispute_count, uid, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'), datetime('now'))",
                rusqlite::params![item_id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, dispute_count, uid],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("entities", &uid, id);
        return Ok(());
    }
    let id = existing.unwrap();
    ctx.conn
        .execute(
            "UPDATE entities SET kind=?2, summary=?3, reasoning=?4, confidence=?5, author=?6, source_ref=?7, window_text=?8, window_index=?9, status=?10, owner=?11, dispute_count=?12, updated_at=datetime('now') WHERE id=?1",
            rusqlite::params![id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, dispute_count],
        )
        .map_err(|e| e.to_string())?;
    ctx.remember("entities", &uid, id);
    Ok(())
}

type ChunkRefs = (Option<i64>, Option<i64>, Option<i64>);

fn chunk_refs(ctx: &mut ImportCtx, rec: &Value) -> Result<ChunkRefs, String> {
    let item_uid = str_field(rec, "item_uid");
    let entity_uid = str_field(rec, "entity_uid");
    let section_uid = str_field(rec, "section_uid");
    let item_id = if item_uid.is_empty() { None } else { ctx.lookup("ingested_items", &item_uid)? };
    let entity_id = if entity_uid.is_empty() { None } else { ctx.lookup("entities", &entity_uid)? };
    let section_id = if section_uid.is_empty() { None } else { ctx.lookup("sections", &section_uid)? };
    Ok((item_id, entity_id, section_id))
}

fn apply_chunk(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let (item_id, entity_id, section_id) = chunk_refs(ctx, rec)?;
    let existing = ctx.lookup("chunks", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT created_at FROM chunks WHERE id=?1", id));
    let action = decide(existing.is_some(), opt_str_field(rec, "created_at").as_deref(), existing_stamp.as_deref());
    ctx.record("chunk", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let kind = str_field(rec, "kind");
    let source_ref = str_field(rec, "source_ref");
    let content = str_field(rec, "content");
    let level = i64_field(rec, "level").unwrap_or(0);
    let path = str_field(rec, "path");
    let is_pii = bool_int(rec, "is_pii");
    let pii_categories = rec.get("pii_categories").map(|v| v.to_string()).unwrap_or_else(|| "[]".to_string());
    let embedding = embedding_field(ctx, rec, "embedding_b64", "chunk");
    if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO chunks (item_id, entity_id, section_id, kind, source_ref, content, level, path, is_pii, pii_categories, embedding, uid, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))",
                rusqlite::params![item_id, entity_id, section_id, kind, source_ref, content, level, path, is_pii, pii_categories, embedding, uid],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("chunks", &uid, id);
        return Ok(());
    }
    let id = existing.unwrap();
    ctx.conn
        .execute(
            "UPDATE chunks SET item_id=?2, entity_id=?3, section_id=?4, kind=?5, source_ref=?6, content=?7, level=?8, path=?9, is_pii=?10, pii_categories=?11 WHERE id=?1",
            rusqlite::params![id, item_id, entity_id, section_id, kind, source_ref, content, level, path, is_pii, pii_categories],
        )
        .map_err(|e| e.to_string())?;
    if let Some(blob) = embedding {
        ctx.conn
            .execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, id])
            .map_err(|e| e.to_string())?;
    }
    ctx.remember("chunks", &uid, id);
    Ok(())
}

fn apply_relationship(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let from_uid = str_field(rec, "from_uid");
    let to_uid = str_field(rec, "to_uid");
    let kind = str_field(rec, "kind");
    let existing: Option<i64> = ctx
        .conn
        .query_row(
            "SELECT r.id FROM relationships r \
             JOIN entities a ON a.id = r.from_entity_id \
             JOIN entities b ON b.id = r.to_entity_id \
             WHERE a.uid = ?1 AND b.uid = ?2 AND r.kind = ?3",
            rusqlite::params![from_uid, to_uid, kind],
            |r| r.get(0),
        )
        .ok();
    let action = if existing.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("relationship", &format!("{from_uid}->{to_uid}"), action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let from_id = ctx.require("entities", &from_uid, "entity")?;
    let to_id = ctx.require("entities", &to_uid, "entity")?;
    ctx.conn
        .execute(
            "INSERT INTO relationships (from_entity_id, to_entity_id, kind, confidence, source_ref, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
            rusqlite::params![from_id, to_id, kind, f64_field(rec, "confidence"), str_field(rec, "source_ref"), str_field(rec, "note")],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_tag(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let name = str_field(rec, "name");
    if name.is_empty() {
        return Err("tag record without a name".to_string());
    }
    // tags are global: merge by uid first, then by the unique name.
    let mut existing = ctx.lookup("tags", &uid)?;
    if existing.is_none() {
        existing = ctx
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [&name], |r| r.get(0))
            .ok();
        if let Some(id) = existing {
            ctx.remember("tags", &uid, id);
        }
    }
    let action = if existing.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("tag", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let embedding = embedding_field(ctx, rec, "embedding_b64", "tag");
    ctx.conn
        .execute(
            "INSERT INTO tags (name, description, count, embedding, uid, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            rusqlite::params![name, str_field(rec, "description"), i64_field(rec, "count").unwrap_or(0), embedding, uid],
        )
        .map_err(|e| e.to_string())?;
    let id = ctx.conn.last_insert_rowid();
    ctx.remember("tags", &uid, id);
    Ok(())
}

fn apply_chunk_tag(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let chunk_uid = str_field(rec, "chunk_uid");
    let tag_uid = str_field(rec, "tag_uid");
    let exists: Option<i64> = ctx
        .conn
        .query_row(
            "SELECT ct.chunk_id FROM chunk_tags ct JOIN chunks c ON c.id = ct.chunk_id JOIN tags t ON t.id = ct.tag_id WHERE c.uid = ?1 AND t.uid = ?2",
            rusqlite::params![chunk_uid, tag_uid],
            |r| r.get(0),
        )
        .ok();
    let action = if exists.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("chunk_tag", &format!("{chunk_uid}:{tag_uid}"), action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let chunk_id = ctx.require("chunks", &chunk_uid, "chunk")?;
    let tag_id = ctx.require("tags", &tag_uid, "tag")?;
    ctx.conn
        .execute("INSERT OR IGNORE INTO chunk_tags (chunk_id, tag_id) VALUES (?1, ?2)", rusqlite::params![chunk_id, tag_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_project(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let existing = ctx.lookup("projects", &uid)?;
    let existing_stamp = existing.and_then(|id| ctx.stamp_of("SELECT created_at FROM projects WHERE id=?1", id));
    let action = decide(existing.is_some(), opt_str_field(rec, "created_at").as_deref(), existing_stamp.as_deref());
    ctx.record("project", &uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let name = str_field(rec, "name");
    let is_default = bool_int(rec, "is_default");
    let id = if action == Action::Insert {
        ctx.conn
            .execute(
                "INSERT INTO projects (name, is_default, uid, created_at) VALUES (?1, ?2, ?3, datetime('now'))",
                rusqlite::params![name, is_default, uid],
            )
            .map_err(|e| e.to_string())?;
        let id = ctx.conn.last_insert_rowid();
        ctx.remember("projects", &uid, id);
        id
    } else {
        let id = existing.unwrap();
        ctx.conn
            .execute("UPDATE projects SET name=?1, is_default=?2 WHERE id=?3", rusqlite::params![name, is_default, id])
            .map_err(|e| e.to_string())?;
        ctx.remember("projects", &uid, id);
        id
    };
    let source_uids: Vec<String> = rec
        .get("source_uids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    for source_uid in source_uids {
        let Some(source_id) = ctx.lookup("sources", &source_uid)? else {
            ctx.warnings.push(format!("project {name}: source {source_uid} not in archive — link skipped"));
            continue;
        };
        ctx.conn
            .execute("INSERT OR IGNORE INTO project_sources (project_id, source_id) VALUES (?1, ?2)", rusqlite::params![id, source_id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_dispute(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let entity_uid = str_field(rec, "entity_uid");
    let reason = str_field(rec, "reason");
    let user = str_field(rec, "user");
    let exists: Option<i64> = ctx
        .conn
        .query_row(
            "SELECT d.id FROM disputes d JOIN entities e ON e.id = d.entity_id WHERE e.uid = ?1 AND d.reason = ?2 AND d.user = ?3",
            rusqlite::params![entity_uid, reason, user],
            |r| r.get(0),
        )
        .ok();
    let action = if exists.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("dispute", &entity_uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let entity_id = ctx.require("entities", &entity_uid, "entity")?;
    ctx.conn
        .execute(
            "INSERT INTO disputes (entity_id, reason, user, created_at) VALUES (?1, ?2, ?3, datetime('now'))",
            rusqlite::params![entity_id, reason, user],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_merge_action(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let a_uid = str_field(rec, "entity_a_uid");
    let b_uid = str_field(rec, "entity_b_uid");
    let status = str_field(rec, "status");
    let reason = str_field(rec, "reason");
    let exists: Option<i64> = ctx
        .conn
        .query_row(
            "SELECT m.id FROM merge_actions m JOIN entities a ON a.id = m.entity_a_id WHERE a.uid = ?1 AND m.status = ?2 AND m.reason = ?3",
            rusqlite::params![a_uid, status, reason],
            |r| r.get(0),
        )
        .ok();
    let action = if exists.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("merge_action", &a_uid, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    let a_id = ctx.require("entities", &a_uid, "entity")?;
    let b_id = if b_uid.is_empty() { None } else { ctx.lookup("entities", &b_uid)? };
    ctx.conn
        .execute(
            "INSERT INTO merge_actions (entity_a_id, entity_b_id, status, reason, user, created_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
            rusqlite::params![a_id, b_id, status, reason, str_field(rec, "user")],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Settings are machine-local (plan §2): a key that already exists locally wins.
fn apply_setting(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    let key = str_field(rec, "key");
    let value = str_field(rec, "value");
    if key.is_empty() || crate::settings::SECRET_KEYS.contains(&key.as_str()) {
        return Ok(());
    }
    let exists: Option<String> = ctx
        .conn
        .query_row("SELECT value FROM app_settings WHERE key = ?1", [&key], |r| r.get(0))
        .ok();
    let action = if exists.is_some() { Action::Skip } else { Action::Insert };
    ctx.record("setting", &key, action);
    if !ctx.write || action == Action::Skip {
        return Ok(());
    }
    ctx.conn
        .execute("INSERT INTO app_settings (key, value) VALUES (?1, ?2)", rusqlite::params![key, value])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn apply_embedding(ctx: &mut ImportCtx, rec: &Value, table: &'static str, column: &str, kind: &str) -> Result<(), String> {
    let uid = str_field(rec, "uid");
    let Some(existing) = ctx.lookup(table, &uid)? else {
        ctx.warnings.push(format!("{kind}: no local {table} row for uid {uid} — skipped"));
        return Ok(());
    };
    ctx.record(kind, &uid, Action::Update);
    if !ctx.write {
        return Ok(());
    }
    let Some(blob) = embedding_field(ctx, rec, "embedding_b64", kind) else {
        return Ok(());
    };
    let sql = format!("UPDATE {} SET {} = ?1 WHERE id = ?2", table, column);
    ctx.conn
        .execute(&sql, rusqlite::params![blob, existing])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// --- dispatcher + stream ---------------------------------------------------

fn is_state_record(kind: &str) -> bool {
    matches!(
        kind,
        "dispute" | "merge_action" | "setting" | "chunk_embedding" | "section_embedding" | "tag_embedding"
    )
}

fn apply_core_record(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    match str_field(rec, "type").as_str() {
        "source" => apply_source(ctx, rec),
        "item" => apply_item(ctx, rec),
        "section" => apply_section(ctx, rec),
        "entity" => apply_entity(ctx, rec),
        "relationship" => apply_relationship(ctx, rec),
        "chunk" => apply_chunk(ctx, rec),
        "tag" => apply_tag(ctx, rec),
        "chunk_tag" => apply_chunk_tag(ctx, rec),
        "project" => apply_project(ctx, rec),
        other => Err(format!("unknown record type: {other}")),
    }
}

fn apply_state_record(ctx: &mut ImportCtx, rec: &Value) -> Result<(), String> {
    match str_field(rec, "type").as_str() {
        "dispute" => apply_dispute(ctx, rec),
        "merge_action" => apply_merge_action(ctx, rec),
        "setting" => apply_setting(ctx, rec),
        "chunk_embedding" => apply_embedding(ctx, rec, "chunks", "embedding", "chunk_embedding"),
        "section_embedding" => apply_embedding(ctx, rec, "sections", "summary_embedding", "section_embedding"),
        _ => apply_embedding(ctx, rec, "tags", "embedding", "tag_embedding"),
    }
}

fn apply_record(ctx: &mut ImportCtx, rec: &Value, counts: &mut BTreeMap<String, u64>) -> Result<(), String> {
    let kind = str_field(rec, "type");
    if kind.is_empty() {
        return Err("record without a type".to_string());
    }
    *counts.entry(kind.clone()).or_default() += 1;
    if kind.starts_with("ops_") {
        // ops records are informational only (jobs/events/users stay local)
        return Ok(());
    }
    if is_state_record(&kind) {
        return apply_state_record(ctx, rec);
    }
    apply_core_record(ctx, rec)
}

/// Stream `data.jsonl`, validating and applying each record in file order (the
/// exporter writes parents before children).
fn import_stream<R: std::io::Read>(
    reader: R,
    ctx: &mut ImportCtx,
    counts: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    for (idx, line) in std::io::BufReader::new(reader).lines().enumerate() {
        let line = line.map_err(|e| format!("data.jsonl: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let line_no = idx + 1;
        let rec: Value = serde_json::from_str(&line).map_err(|e| format!("data.jsonl line {line_no}: {e}"))?;
        apply_record(ctx, &rec, counts).map_err(|e| format!("data.jsonl line {line_no}: {e}"))?;
    }
    Ok(())
}

// --- archive plumbing ------------------------------------------------------

fn entry_within_caps(size: u64, compressed: u64, max: u64, name: &str) -> Result<(), String> {
    if size > max {
        return Err(format!("{name} is too large ({} bytes)", size));
    }
    if compressed > 0 && size / compressed.max(1) > MAX_COMPRESSION_RATIO {
        return Err(format!("{name} looks like a zip bomb (ratio {})", size / compressed.max(1)));
    }
    Ok(())
}

fn read_entry_bounded<R: std::io::Read>(entry: &mut R, max: u64, name: &str) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    entry
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("reading {name}: {e}"))?;
    if buf.len() as u64 > max {
        return Err(format!("{name} is too large"));
    }
    Ok(buf)
}

fn open_archive(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open archive: {e}"))?;
    let zip = zip::ZipArchive::new(file).map_err(|e| format!("not a valid .anchorcore-backup zip: {e}"))?;
    // We never extract entries to disk, but reject unsafe names loudly anyway.
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok()).map(|s| s.to_string()).collect();
    for name in &names {
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') || name.contains(':') {
            return Err(format!("archive contains an unsafe entry name: {name}"));
        }
    }
    if !names.iter().any(|n| n == "data.jsonl") {
        return Err("archive has no data.jsonl".to_string());
    }
    Ok(zip)
}

fn read_manifest(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<ImportReport, String> {
    let mut entry = zip
        .by_name("manifest.json")
        .map_err(|_| "archive has no manifest.json".to_string())?;
    entry_within_caps(entry.size(), entry.compressed_size(), MAX_MANIFEST_BYTES, "manifest.json")?;
    let buf = read_entry_bounded(&mut entry, MAX_MANIFEST_BYTES, "manifest.json")?;
    let manifest: Value = serde_json::from_slice(&buf).map_err(|e| format!("manifest.json: {e}"))?;
    let version = manifest.get("format_version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version == 0 {
        return Err("manifest.json has no format_version".to_string());
    }
    if version > FORMAT_VERSION {
        return Err(format!(
            "archive format version {version} is newer than this app supports ({FORMAT_VERSION}) — upgrade the app"
        ));
    }
    Ok(ImportReport {
        format_version: version,
        app_version: str_field(&manifest, "app_version"),
        created_at: str_field(&manifest, "created_at"),
        needs_secret: manifest.get("needs_secret").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        folder_paths: manifest.get("folder_paths").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        ..Default::default()
    })
}

/// `replace` wipes every data record (settings, ops and users stay local).
fn clear_data(conn: &Connection) -> Result<(), String> {
    let stmts = [
        "DELETE FROM chunk_tags",
        "DELETE FROM chunks",
        "DELETE FROM sections",
        "DELETE FROM disputes",
        "DELETE FROM merge_actions",
        "DELETE FROM relationships",
        "DELETE FROM entities",
        "DELETE FROM project_sources",
        "DELETE FROM projects",
        "DELETE FROM ingested_items",
        "DELETE FROM sources",
        "DELETE FROM tags",
    ];
    for sql in stmts {
        conn.execute(sql, []).map_err(|e| format!("{sql}: {e}"))?;
    }
    Ok(())
}

/// A pre-replace copy that is never swept (documents + review state, no secrets).
fn safety_backup(data_dir: &Path) -> Result<String, String> {
    let res = export_to_file(data_dir, &ExportOptions::default())?;
    let name = res.filename.replacen("anchorcore-backup", "safety-backup", 1);
    let path = res.path.with_file_name(&name);
    std::fs::rename(&res.path, &path).map_err(|e| e.to_string())?;
    Ok(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(name))
}

fn parse_secret_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("source:")?;
    let (uid, field) = rest.rsplit_once(':')?;
    if uid.is_empty() || field.is_empty() {
        return None;
    }
    Some((uid, field))
}

fn apply_secrets_bundle(
    data_dir: &Path,
    entries: Vec<(String, String)>,
    ctx: &mut ImportCtx,
    report: &mut ImportReport,
) -> Result<(), String> {
    let store = SecretStore::new(data_dir.join("secrets.enc"));
    for (key, value) in entries {
        let Some((uid, field)) = parse_secret_key(&key) else {
            ctx.warnings.push(format!("secret {key} has an unexpected key shape — skipped"));
            continue;
        };
        let uid = uid.to_string();
        let field = field.to_string();
        let Some(source_id) = ctx.lookup("sources", &uid)? else {
            ctx.warnings.push(format!("secret {key}: source is not in this archive — skipped"));
            continue;
        };
        store.set(&crate::secrets::source_secret_key(source_id, &field), &value);
        report.secrets_imported += 1;
    }
    Ok(())
}

/// Read + decrypt `secrets.bundle` **before** anything is written: a wrong
/// passphrase must fail the import cleanly, not after the records landed.
fn read_bundle_entries(
    zip: &mut zip::ZipArchive<std::fs::File>,
    opts: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Option<Vec<(String, String)>>, String> {
    let Ok(mut entry) = zip.by_name("secrets.bundle") else {
        return Ok(None);
    };
    entry_within_caps(entry.size(), entry.compressed_size(), MAX_BUNDLE_BYTES, "secrets.bundle")?;
    let buf = read_entry_bounded(&mut entry, MAX_BUNDLE_BYTES, "secrets.bundle")?;
    drop(entry);
    let passphrase = match opts.passphrase.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            report.secrets_note = Some(
                "archive contains secrets.bundle — skipped (no passphrase given); re-enter credentials".to_string(),
            );
            return Ok(None);
        }
    };
    let entries = open_bundle(&passphrase, &buf)?;
    if opts.dry_run {
        report.secrets_imported = entries.len() as u64;
        return Ok(None);
    }
    Ok(Some(entries))
}

fn sources_missing_embeddings(conn: &Connection) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.id FROM sources s \
             JOIN ingested_items i ON i.source_id = s.id \
             JOIN chunks c ON c.item_id = i.id \
             WHERE c.embedding IS NULL ORDER BY s.id",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

fn import_data(
    zip: &mut zip::ZipArchive<std::fs::File>,
    conn: &Connection,
    opts: &ImportOptions,
    ctx: &mut ImportCtx,
    counts: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    let mut entry = zip.by_name("data.jsonl").map_err(|_| "archive has no data.jsonl".to_string())?;
    entry_within_caps(entry.size(), entry.compressed_size(), MAX_DATA_BYTES, "data.jsonl")?;
    if opts.mode == ImportMode::Replace && ctx.write {
        clear_data(conn)?;
    }
    import_stream(&mut entry, ctx, counts)}

/// Always apply inside one transaction: a dry run is "apply, then roll back",
/// which keeps conflict detection and counts identical to a real import while
/// leaving nothing behind.
fn apply_in_transaction(
    conn: &Connection,
    zip: &mut zip::ZipArchive<std::fs::File>,
    opts: &ImportOptions,
    ctx: &mut ImportCtx,
    counts: &mut BTreeMap<String, u64>,
) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    let applied = import_data(zip, conn, opts, ctx, counts);
    let _ = match (&applied, opts.dry_run) {
        (Ok(_), false) => conn.execute_batch("COMMIT"),
        _ => conn.execute_batch("ROLLBACK"),
    };
    applied
}

fn finalize_report(
    conn: &Connection,
    opts: &ImportOptions,
    ctx: &mut ImportCtx,
    counts: BTreeMap<String, u64>,
    report: &mut ImportReport,
) -> Result<(), String> {
    report.counts = counts;
    report.imported = std::mem::take(&mut ctx.imported);
    report.skipped = std::mem::take(&mut ctx.skipped);
    report.conflicts = std::mem::take(&mut ctx.conflicts);
    report.warnings.extend(std::mem::take(&mut ctx.warnings));
    report.remap_applied = std::mem::take(&mut ctx.remapped);
    if opts.dry_run {
        return Ok(());
    }
    // tag vectors are local; rebuild any that arrived without one (R25.3)
    let _ = crate::tags::ensure_embeddings(conn);
    report.reembed_source_ids = sources_missing_embeddings(conn)?;
    Ok(())
}

fn mode_label(opts: &ImportOptions) -> String {
    if opts.dry_run {
        return "dry_run".to_string();
    }
    match opts.mode {
        ImportMode::Merge => "merge".to_string(),
        ImportMode::Replace => "replace".to_string(),
    }
}

fn record_import_event(conn: &Connection, report: &ImportReport) {
    record_export_event(
        conn,
        "info",
        &serde_json::to_string(&json!({
            "mode": report.mode,
            "dry_run": report.dry_run,
            "counts": report.counts,
            "imported": report.imported,
            "skipped": report.skipped,
            "secrets_imported": report.secrets_imported,
            "safety_backup": report.safety_backup,
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    );
}

/// Import an archive (merge or replace). Returns the dry-run report too, so the
/// same call powers "show me what would happen" and the real import.
pub fn import_archive(data_dir: &Path, archive_path: &Path, opts: &ImportOptions) -> Result<ImportReport, String> {
    opts.validate()?;
    let db_path = crate::db::resolve_db_path(&data_dir.to_string_lossy());
    let conn = crate::db::init_db(&db_path).map_err(|e| e.to_string())?;
    let _ = crate::db::backfill_uids(&conn);
    let mut zip = open_archive(archive_path)?;
    let mut report = read_manifest(&mut zip)?;
    report.dry_run = opts.dry_run;
    report.mode = mode_label(opts);
    let mut counts = BTreeMap::new();
    let mut ctx = ImportCtx::new(&conn, true, opts.remap.clone());

    // Decrypt the bundle first: a wrong passphrase must abort before any write.
    let bundle_entries = read_bundle_entries(&mut zip, opts, &mut report)?;

    if opts.mode == ImportMode::Replace && !opts.dry_run {
        let backup = safety_backup(data_dir)?;
        report.safety_backup = Some(backup);
    }
    if let Err(e) = apply_in_transaction(&conn, &mut zip, opts, &mut ctx, &mut counts) {
        record_export_event(&conn, "error", &format!("import failed: {e}"));
        return Err(e);
    }
    if let Some(entries) = bundle_entries {
        apply_secrets_bundle(data_dir, entries, &mut ctx, &mut report)?;
    }
    finalize_report(&conn, opts, &mut ctx, counts, &mut report)?;
    if !opts.dry_run {
        record_import_event(&conn, &report);
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

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

use axum::extract::Multipart;
use tokio::io::AsyncWriteExt as _;

fn bad_request(msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({"detail": msg.to_string()})))
}

fn internal_error(msg: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": msg.to_string()})))
}

/// Queue an `embed` job per imported source that has chunks without vectors;
/// the job embeds them and retrieval stops reporting keyword-only.
fn enqueue_reembed(state: &AppState, source_ids: &[i64]) -> Vec<i64> {
    let mut job_ids = Vec::new();
    for &source_id in source_ids {
        let conn = crate::common::open_data_db(&state.data_dir);
        let Ok(job_id) = state.jobs.create_job(&conn, source_id, "embed") else {
            continue;
        };
        drop(conn);
        let settings = state.settings.clone();
        let jobs = state.jobs.clone();
        let embedder = state.embedder.clone();
        let data_dir = state.data_dir.clone();
        tokio::spawn(Box::pin(async move {
            let classifier = std::sync::Arc::new(crate::classifier::Classifier::new(settings.clone()));
            let pipeline = crate::pipeline::Pipeline::new(classifier, embedder, settings, data_dir, jobs);
            Box::pin(pipeline.embed_missing_for_source(source_id, job_id)).await;
        }));
        job_ids.push(job_id);
    }
    job_ids
}

async fn parse_options_field(field: axum::extract::multipart::Field<'_>) -> Result<ImportOptions, (StatusCode, Json<Value>)> {
    let text = field.text().await.map_err(bad_request)?;
    if text.trim().is_empty() {
        return Ok(ImportOptions::default());
    }
    serde_json::from_str(&text).map_err(|e| bad_request(format!("options: {e}")))
}

async fn save_upload(data_dir: &Path, mut field: axum::extract::multipart::Field<'_>) -> Result<PathBuf, (StatusCode, Json<Value>)> {
    let path = data_dir
        .join(EXPORT_DIR)
        .join(format!("import-{:08x}{}", rand::random::<u32>(), ARCHIVE_SUFFIX));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(internal_error)?;
    }
    let mut file = tokio::fs::File::create(&path).await.map_err(internal_error)?;
    while let Some(chunk) = field.chunk().await.map_err(bad_request)? {
        file.write_all(&chunk).await.map_err(internal_error)?;
    }
    file.flush().await.map_err(internal_error)?;
    Ok(path)
}

/// Read `options` (JSON, optional) and `file` (the archive) out of the request.
async fn collect_import_parts(
    data_dir: &Path,
    mut multipart: Multipart,
) -> Result<(ImportOptions, Option<PathBuf>), (StatusCode, Json<Value>)> {
    let mut opts = ImportOptions::default();
    let mut archive: Option<PathBuf> = None;
    while let Some(field) = multipart.next_field().await.map_err(bad_request)? {
        match field.name() {
            Some("options") => opts = parse_options_field(field).await?,
            Some("file") => archive = Some(save_upload(data_dir, field).await?),
            _ => {}
        }
    }
    Ok((opts, archive))
}

/// `POST /backup/import` — multipart `file` (the archive) + optional `options`
/// (JSON `ImportOptions`; default is a merge dry-run). A dry run reports what
/// would happen; a real import is transactional.
pub async fn import_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = PathBuf::from(&state.data_dir);
    let (opts, archive) = collect_import_parts(&data_dir, multipart).await?;
    let Some(archive_path) = archive else {
        return Err(bad_request("multipart field `file` is required"));
    };
    let data_dir_clone = data_dir.clone();
    let path_for_task = archive_path.clone();
    let result = tokio::task::spawn_blocking(move || import_archive(&data_dir_clone, &path_for_task, &opts)).await;
    let _ = std::fs::remove_file(&archive_path);
    let report = match result {
        Ok(Ok(report)) => report,
        Ok(Err(msg)) => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": msg})))),
        Err(e) => return Err(internal_error(e)),
    };
    let job_ids = if report.dry_run {
        Vec::new()
    } else {
        enqueue_reembed(&state, &report.reembed_source_ids)
    };
    let mut value = serde_json::to_value(&report).map_err(internal_error)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("reembed_job_ids".to_string(), json!(job_ids));
    }
    Ok(Json(value))
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
            "INSERT INTO entities (item_id, kind, summary, status, owner) VALUES (?1,'decision','Nightly ledger payout','verified','ops')",
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
        assert!(needs.iter().all(|n| n["source_name"] == "payments"), "{needs:?}");
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

    // --- R25.3 import ------------------------------------------------------

    fn db(dir: &Path) -> rusqlite::Connection {
        let db_path = crate::db::resolve_db_path(&dir.to_string_lossy());
        crate::db::init_db(&db_path).unwrap()
    }

    fn one<T: rusqlite::types::FromSql>(conn: &rusqlite::Connection, sql: &str) -> T {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn exported(dir: &Path, opts: ExportOptions) -> ExportResult {
        export_to_file(dir, &opts).unwrap()
    }

    fn rewrite_manifest_version(src: &Path, dst: &Path, version: u32) {
        let raw = std::fs::read(src).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(raw)).unwrap();
        let mut writer = zip::ZipWriter::new(std::fs::File::create(dst).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            drop(entry);
            if name == "manifest.json" {
                let mut manifest: Value = serde_json::from_slice(&buf).unwrap();
                manifest["format_version"] = json!(version);
                buf = serde_json::to_vec(&manifest).unwrap();
            }
            writer.start_file(name, options).unwrap();
            writer.write_all(&buf).unwrap();
        }
        writer.finish().unwrap();
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn import_round_trip_restores_memory_review_state_and_secrets() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        let sid = seed(src.path());
        let archive = exported(
            src.path(),
            ExportOptions {
                include_secrets: true,
                passphrase: Some("correct horse battery".to_string()),
                ..Default::default()
            },
        );
        // the export backfills uids; read it after so the value is final
        let uid = one::<String>(&db(src.path()), &format!("SELECT uid FROM sources WHERE id = {sid}"));

        let dst = tempdir().unwrap();
        let report = import_archive(
            dst.path(),
            &archive.path,
            &ImportOptions {
                passphrase: Some("correct horse battery".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.mode, "merge");
        assert!(!report.dry_run);
        assert_eq!(report.imported.get("source"), Some(&1u64));
        assert_eq!(report.imported.get("item"), Some(&1u64));
        assert_eq!(report.imported.get("entity"), Some(&1u64));
        assert_eq!(report.imported.get("chunk"), Some(&2u64));
        assert_eq!(report.imported.get("section"), Some(&1u64));
        assert_eq!(report.imported.get("tag"), Some(&1u64));
        assert_eq!(report.imported.get("chunk_tag"), Some(&1u64));
        assert_eq!(report.imported.get("project"), Some(&1u64));
        assert_eq!(report.imported.get("dispute"), Some(&1u64));
        assert_eq!(report.imported.get("merge_action"), Some(&1u64));
        assert_eq!(report.needs_secret.len(), 2, "manifest carries the credential list");
        assert_eq!(report.secrets_imported, 2);

        let conn = db(dst.path());
        assert_eq!(one::<String>(&conn, "SELECT uid FROM sources"), uid, "uids survive a move");
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM ingested_items"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM entities WHERE status='verified' AND owner='ops'"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM disputes"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM merge_actions"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM chunk_tags"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM project_sources"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM relationships"), 0);
        // non-secret settings land on a fresh machine; secret ones never travel
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM app_settings WHERE key='answer_model'"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM app_settings WHERE key='answer_api_key'"), 0);
        // imported chunks have no vectors yet — the report asks for a re-embed job
        let new_sid: i64 = one(&conn, "SELECT id FROM sources");
        assert_eq!(report.reembed_source_ids, vec![new_sid]);
        // the bundle landed in the local secret store
        let store = SecretStore::new(dst.path().join("secrets.enc"));
        assert_eq!(
            store.get(&crate::secrets::source_secret_key(new_sid, "token")),
            Some(MARKER_TOKEN.to_string())
        );
        assert_eq!(
            store.get(&crate::secrets::source_secret_key(new_sid, "password")),
            Some(MARKER_PHRASE.to_string())
        );

        // a second import is idempotent: nothing new, everything kept local
        let again = import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        assert!(again.imported.get("source").is_none(), "{:?}", again.imported);
        assert_eq!(again.skipped.get("entity"), Some(&1u64));
        assert_eq!(one::<i64>(&db(dst.path()), "SELECT COUNT(*) FROM sources"), 1);
    }

    #[test]
    fn conflict_note_survives_export_import() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        let sid = seed(src.path());
        let conn = db(src.path());
        let iid: i64 = one(&conn, &format!("SELECT id FROM ingested_items WHERE source_id = {sid}"));
        let a: i64 = one(&conn, "SELECT id FROM entities");
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','code choice')", [iid]).unwrap();
        let b = conn.last_insert_rowid();
        conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind, note) VALUES (?1,?2,'conflicts_with','doc says JWT, code uses sessions')", rusqlite::params![a, b]).unwrap();
        drop(conn);
        let archive = exported(src.path(), ExportOptions::default());
        let dst = tempdir().unwrap();
        import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        let conn = db(dst.path());
        assert_eq!(one::<String>(&conn, "SELECT note FROM relationships WHERE kind='conflicts_with'"), "doc says JWT, code uses sessions");
    }

    #[test]
    fn item_pii_mark_survives_export_import() {
        // R35.1: the manual whole-doc mark travels with the item.
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        let sid = seed(src.path());
        let conn = db(src.path());
        conn.execute("UPDATE ingested_items SET is_pii = 1 WHERE source_id = ?1", [sid]).unwrap();
        drop(conn);
        let archive = exported(src.path(), ExportOptions::default());
        let dst = tempdir().unwrap();
        import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        let conn = db(dst.path());
        assert_eq!(one::<i64>(&conn, "SELECT is_pii FROM ingested_items"), 1);
    }

    #[test]
    fn import_with_wrong_passphrase_leaves_nothing_behind() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        let archive = exported(
            src.path(),
            ExportOptions {
                include_secrets: true,
                passphrase: Some("right".to_string()),
                ..Default::default()
            },
        );
        let dst = tempdir().unwrap();
        let err = import_archive(
            dst.path(),
            &archive.path,
            &ImportOptions {
                passphrase: Some("wrong".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("wrong passphrase"), "{err}");
        // the failure happens before the transaction: no partial import
        assert_eq!(one::<i64>(&db(dst.path()), "SELECT COUNT(*) FROM sources"), 0);
        assert_eq!(one::<i64>(&db(dst.path()), "SELECT COUNT(*) FROM ingested_items"), 0);
    }

    #[test]
    fn import_dry_run_reports_without_writing() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        let archive = exported(src.path(), ExportOptions::default());
        let dst = tempdir().unwrap();
        let report = import_archive(
            dst.path(),
            &archive.path,
            &ImportOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.dry_run);
        assert_eq!(report.mode, "dry_run");
        assert_eq!(report.imported.get("source"), Some(&1), "reports what would happen");
        assert!(!report.folder_paths.is_empty(), "folder remap hints come from the manifest");
        assert_eq!(one::<i64>(&db(dst.path()), "SELECT COUNT(*) FROM sources"), 0);
    }

    #[test]
    fn import_replace_needs_confirmation_and_takes_safety_backup() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        let archive = exported(src.path(), ExportOptions::default());

        // destination has its own, unrelated source
        let dst = tempdir().unwrap();
        let conn = db(dst.path());
        conn.execute("INSERT INTO sources (name, connector) VALUES ('local','folder')", []).unwrap();

        let opts = ImportOptions {
            mode: ImportMode::Replace,
            ..Default::default()
        };
        assert!(import_archive(dst.path(), &archive.path, &opts).is_err(), "replace without confirm");

        let opts = ImportOptions {
            mode: ImportMode::Replace,
            confirm: Some(REPLACE_CONFIRM.to_string()),
            ..Default::default()
        };
        let report = import_archive(dst.path(), &archive.path, &opts).unwrap();
        assert_eq!(report.mode, "replace");
        let backup = report.safety_backup.clone().expect("safety backup name");
        assert!(dst.path().join(EXPORT_DIR).join(&backup).exists());
        let names: Vec<String> = {
            let conn = db(dst.path());
            let mut stmt = conn.prepare("SELECT name FROM sources ORDER BY name").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(|r| r.ok()).collect()
        };
        assert_eq!(names, vec!["payments".to_string()], "local source replaced");
    }

    #[test]
    fn import_keeps_local_state_that_is_newer() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        let archive = exported(src.path(), ExportOptions::default());
        let dst = tempdir().unwrap();
        import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        // local review state moves forward after the export…
        let conn = db(dst.path());
        conn.execute("UPDATE entities SET status='disputed', owner='me', updated_at='2999-01-01 00:00:00'", []).unwrap();
        // …and a re-import must not clobber it (LWW)
        let report = import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        assert_eq!(report.skipped.get("entity"), Some(&1u64));
        assert_eq!(one::<String>(&conn, "SELECT status FROM entities"), "disputed");
        assert!(report
            .conflicts
            .iter()
            .any(|c| c["type"] == "entity" && c["action"] == "kept-local"));
    }

    #[test]
    fn import_refuses_newer_format_unsafe_names_and_bad_zips() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        let archive = exported(src.path(), ExportOptions::default());
        let dst = tempdir().unwrap();

        let newer = dst.path().join("newer.anchorcore-backup");
        rewrite_manifest_version(&archive.path, &newer, FORMAT_VERSION + 1);
        let err = import_archive(dst.path(), &newer, &ImportOptions::default()).unwrap_err();
        assert!(err.contains("upgrade the app"), "{err}");

        let slip = dst.path().join("slip.anchorcore-backup");
        std::fs::write(&slip, zip_of(&[("../evil.txt", b"no".as_slice()), ("data.jsonl", b"".as_slice())])).unwrap();
        assert!(import_archive(dst.path(), &slip, &ImportOptions::default()).unwrap_err().contains("unsafe entry name"));

        let notzip = dst.path().join("notzip.anchorcore-backup");
        std::fs::write(&notzip, b"definitely not a zip").unwrap();
        assert!(import_archive(dst.path(), &notzip, &ImportOptions::default()).is_err());
    }

    #[test]
    fn import_drops_foreign_embeddings_and_remaps_folder_paths() {
        let _env = EnvGuard::file_only();
        let src = tempdir().unwrap();
        seed(src.path());
        // a vector from another embed model (wrong dim) must not be trusted
        db(src.path())
            .execute("UPDATE chunks SET embedding = ?1 WHERE kind='document'", [vec![9u8; 8]])
            .unwrap();
        let archive = exported(
            src.path(),
            ExportOptions {
                include_embeddings: true,
                ..Default::default()
            },
        );
        let dst = tempdir().unwrap();
        let report = import_archive(dst.path(), &archive.path, &ImportOptions::default()).unwrap();
        assert!(
            report.warnings.iter().any(|w| w.contains("this app uses")),
            "{:?}",
            report.warnings
        );
        assert_eq!(one::<i64>(&db(dst.path()), "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL"), 0);

        // folder remap: the dry run lists the old path, the import applies the new one
        let report = import_archive(
            dst.path(),
            &archive.path,
            &ImportOptions {
                mode: ImportMode::Replace,
                confirm: Some(REPLACE_CONFIRM.to_string()),
                remap: BTreeMap::from([(
                    one::<String>(&db(dst.path()), "SELECT uid FROM sources"),
                    "D:\\Docs\\payments".to_string(),
                )]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.remap_applied.len(), 1);
        let cfg: String = one(&db(dst.path()), "SELECT config FROM sources");
        assert!(cfg.contains("D:\\\\Docs\\\\payments"), "{cfg}");
    }
}
