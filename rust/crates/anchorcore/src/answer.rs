//! R2.2: Answer orchestration — port of `backend/app/answer_engine.py:169` `ask()`
//! - project scoping, public_only gate, planner, executor, graph, B30 gate, generation

use crate::embedder::Embedder;
use crate::retrieval;
use crate::settings::SettingsService;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct AskRequest {
    pub question: String,
    pub history: Option<Vec<AskTurn>>,
    pub project_id: Option<i64>,
    pub public_only: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AskTurn {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct AskResponse {
    pub answer: String,
    pub citations: Vec<Citation>,
    /// Machine-readable gate outcome (Phase-1 deterministic reads): None when
    /// content was returned, Some("gated_provider") when everything relevant
    /// was withheld from an untrusted answer provider.
    pub refusal: Option<String>,
}

#[derive(Serialize)]
pub struct Citation {
    pub entity_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub source_ref: String,
    pub path: String,
    pub tags: Vec<String>,
    pub score: f64,
    pub snippet: String,
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub k: Option<usize>,
    pub project_id: Option<i64>,
    pub public_only: Option<bool>,
    /// A3 compact-by-default selector: comma list from
    /// `summary,source_ref,score,content` (or `all`). Chunk `content` is
    /// omitted unless requested — harnesses walk cheap citable hits, then
    /// drill down verbatim via get_entity/context.
    pub fields: Option<String>,
    /// A3 cursor pagination: zero-based offset into the gated hit list.
    pub cursor: Option<usize>,
    /// A3 cursor pagination: page size (defaults to `k`).
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub chunk_id: Option<i64>,
    pub entity_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub content: String,
    pub source_ref: String,
    pub path: String,
    pub tags: Vec<String>,
    pub source_id: Option<i64>,
    pub item_id: Option<i64>,
    pub item_title: String,
    pub score: f64,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// Count of hits withheld by the provider-trust gate (Phase-1).
    pub blocked: usize,
    /// Some("gated_provider") when relevant hits existed but none survived
    /// gating — lets harnesses react deterministically instead of guessing.
    pub refusal: Option<String>,
    /// A3 cursor for the next page (offset into the gated hit list), None
    /// when this page exhausts what the caller may see.
    pub next_cursor: Option<usize>,
}

async fn rewrite_question(settings: &SettingsService, req: &AskRequest) -> String {
    // R10.1: follow-up rewrite — history + trusted gate (like Python ask: rewrite before retrieval)
    let original_question = req.question.clone();
    let history = req.history.clone().unwrap_or_default();
    let filtered_history: Vec<AskTurn> = history.into_iter().filter(|t| !t.content.trim().is_empty()).collect();
    let trusted_for_rewrite = is_answer_trusted(settings);
    if filtered_history.is_empty() || !trusted_for_rewrite {
        return original_question;
    }
    if let Some(rewritten) = rewrite_followup(settings, &original_question, &filtered_history).await {
        if !rewritten.trim().is_empty() && rewritten != original_question {
            return rewritten;
        }
    }
    original_question
}

async fn record_degraded(data_dir: &str) {
    // R10.5: only record qa warnings when retrieval actually degraded (hits empty) — not on every qa
    // Python records these only when embedding failed / no hits; previously Rust spammed 2 warnings per qa
    let data_dir = data_dir.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::init_db(&db_path) {
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning','retrieval degraded to keyword-only','qa fallback')", []);
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning','answer generation failed; returning matching context only','qa fallback')", []);
        }
    })
    .await;
}

async fn record_gate_event(data_dir: &str, blocked: &[String]) {
    let data_dir = data_dir.to_string();
    let blocked_len = blocked.len();
    let labels = blocked.to_vec();
    let _ = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::init_db(&db_path) {
            let msg = format!("answer gate: {} hit(s) excluded from cloud answer (sensitive/pii source or PII chunk)", blocked_len);
            let detail = format!("labels={:?}; set ANCHOR_CLOUD_TRUST=1 to allow", labels);
            let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('qa','warning',?1,?2)", rusqlite::params![msg, detail]);
        }
    })
    .await;
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn display_source_ref(h: &crate::retrieval::Hit) -> String {
    if h.source_ref.is_empty() { format!("chunk:{}", h.chunk_id) } else { h.source_ref.clone() }
}

fn hit_kind(h: &crate::retrieval::Hit) -> String {
    if h.entity_id.is_some() { "entity".to_string() } else { "document".to_string() }
}

fn rounded_score(score: f64) -> f64 {
    (score * 1000.0).round() / 1000.0
}

fn tags_for_chunk(conn: &Connection, chunk_id: i64) -> Vec<String> {
    tags_for_chunks(conn, &[chunk_id]).remove(&chunk_id).unwrap_or_default()
}

/// Single batched tags lookup — replaces per-hit `tags_for_chunk` N+1.
fn tags_for_chunks(conn: &Connection, chunk_ids: &[i64]) -> std::collections::HashMap<i64, Vec<String>> {
    let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    if chunk_ids.is_empty() {
        return out;
    }
    let sql = format!(
        "SELECT ct.chunk_id, t.name FROM tags t JOIN chunk_tags ct ON ct.tag_id=t.id WHERE ct.chunk_id IN ({}) ORDER BY t.name",
        crate::common::placeholders(chunk_ids.len())
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk_ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    }) {
        for row in rows.flatten() {
            out.entry(row.0).or_default().push(row.1);
        }
    }
    out
}

fn build_citations(hits: &[crate::retrieval::Hit], data_dir: &str) -> Vec<Citation> {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    let ids: Vec<i64> = hits.iter().take(5).map(|h| h.chunk_id).collect();
    let tag_map = tags_for_chunks(&conn, &ids);
    hits.iter().take(5).map(|h| {
        Citation {
            entity_id: h.entity_id,
            kind: hit_kind(h),
            summary: take_chars(&h.content, 200),
            source_ref: display_source_ref(h),
            path: h.path.clone(),
            tags: tag_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            score: rounded_score(h.score),
            snippet: take_chars(&h.content, 300),
        }
    }).collect()
}

pub async fn ask(
    settings: &SettingsService,
    embedder: &Arc<Embedder>,
    req: AskRequest,
    data_dir: &str,
) -> AskResponse {
    let question = rewrite_question(settings, &req).await;
    // R8.1: embed query for vector search (hybrid) — deterministic fallback when remote unavailable
    let query_embedding = embedder.embed_query(&question).await;
    // Do DB retrieval in blocking thread to avoid holding !Send Connection across await
    let data_dir_string = data_dir.to_string();
    let data_dir_for_hits = data_dir_string.clone();
    let project_id = req.project_id;
    let public_only = req.public_only.unwrap_or(false);
    let history_for_gen: Vec<AskTurn> = req.history.clone().unwrap_or_default().into_iter().filter(|t| !t.content.trim().is_empty()).collect();
    let settings_clone = settings_snapshot(settings);
    let q_for_block = question.clone();
    let hits = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir_for_hits);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        retrieve_sync(&conn, &q_for_block, project_id, public_only, &settings_clone, query_embedding)
    })
    .await
    .unwrap_or_default();

    if hits.is_empty() {
        record_degraded(&data_dir_string).await;
        return AskResponse {
            answer: "No relevant knowledge found yet. Ingest sources first.".to_string(),
            citations: vec![],
            refusal: None,
        };
    }

    let trusted = is_answer_trusted(settings);
    let mut hits = hits;
    if !trusted {
        let hits_for_gate = hits.clone();
        let data_dir_gate = data_dir_string.clone();
        let (filtered, blocked) = tokio::task::spawn_blocking(move || {
            let db_path = crate::db::resolve_db_path(&data_dir_gate);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            gate_answer_hits(&conn, &hits_for_gate)
        }).await.unwrap_or((vec![], vec![]));
        if !blocked.is_empty() {
            record_gate_event(&data_dir_string, &blocked).await;
            if filtered.is_empty() {
                return AskResponse {
                    answer: "Relevant knowledge was found but it is sensitive/PII and the answer provider is not approved for it. Enable a local provider, list it in ANCHOR_TRUSTED_PROVIDERS, or set ANCHOR_CLOUD_TRUST=1 to answer.".to_string(),
                    citations: vec![],
                    refusal: Some("gated_provider".to_string()),
                };
            }
        }
        hits = filtered;
    }

    // Build context and generate (no DB borrow across await) — R10.6 UTF-8 safe (was byte slice)
    let sections: Vec<String> = hits.iter().enumerate().map(|(i, h)| format!("[S{}] {}", i+1, h.content.chars().take(2000).collect::<String>())).collect();
    let context = sections.join("\n\n");
    let answer = generate_answer(settings, &question, &context, &history_for_gen).await;
    let citations = build_citations(&hits, &data_dir_string);
    AskResponse { answer, citations, refusal: None }
}

fn settings_snapshot(settings: &SettingsService) -> std::collections::HashMap<String, String> {
    // snapshot needed for blocking thread (SettingsService is Sync but we clone needed values)
    let mut m = std::collections::HashMap::new();
    for k in ["answer_base_url","answer_api_key","answer_model","ollama_base_url"] {
        if let Some(v) = settings.get(k, None) { m.insert(k.to_string(), v); }
    }
    m
}

// --- R15.2: single retrieval orchestration shared by ask + search ---

struct RetrievalConfig {
    top_k: usize,
    keyword_weight: f64,
    halflife: f64,
    max_per_source: usize,
    qa_exclude_disputed: bool,
}

impl RetrievalConfig {
    fn from_env_with_top_k(top_k: usize) -> Self {
        let qa_exclude_disputed: bool = std::env::var("ANCHOR_QA_EXCLUDE_DISPUTED")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);
        let keyword_weight: f64 = std::env::var("ANCHOR_RETRIEVAL_KEYWORD_WEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let halflife: f64 = std::env::var("ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(365.0);
        let max_per_source: usize = std::env::var("ANCHOR_RETRIEVAL_MAX_PER_SOURCE").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        Self { top_k, keyword_weight, halflife, max_per_source, qa_exclude_disputed }
    }

    fn from_env() -> Self {
        let top_k: usize = std::env::var("ANCHOR_TOP_K").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
        Self::from_env_with_top_k(top_k)
    }
}

fn project_scope(conn: &Connection, project_id: Option<i64>, public_only: bool) -> Option<HashSet<i64>> {
    let mut scope = project_source_ids(conn, project_id);
    if public_only {
        let public_ids = public_source_ids(conn);
        scope = match scope {
            None => Some(public_ids),
            Some(s) => Some(s.intersection(&public_ids).cloned().collect()),
        };
    }
    scope
}

fn apply_graph(
    conn: &Connection,
    hits: Vec<crate::retrieval::Hit>,
    scope: Option<&HashSet<i64>>,
    cfg: &RetrievalConfig,
) -> Vec<crate::retrieval::Hit> {
    let mut hits = hits;
    let graph_hits = retrieval::graph_expand(conn, &hits, scope, 2, 3, cfg.qa_exclude_disputed);
    if !graph_hits.is_empty() {
        let lists = vec![hits, graph_hits];
        hits = retrieval::fuse_evidence(lists, vec![1.0, 0.5]);
        hits.truncate(cfg.top_k);
    }
    hits
}

/// TOC coarse-to-fine first, hybrid vector+keyword fallback otherwise, then graph.
fn orchestrate(
    conn: &Connection,
    query: &str,
    query_embedding: Option<Vec<f32>>,
    scope: Option<HashSet<i64>>,
    cfg: &RetrievalConfig,
) -> Vec<crate::retrieval::Hit> {
    // R14.5: TOC coarse-to-fine — try toc_search first (prunes to ~200 candidates)
    let toc_opt = {
        let emb_opt = query_embedding.as_deref();
        let toc = retrieval::toc_search(conn, query, emb_opt, scope.as_ref(), cfg.top_k, cfg.qa_exclude_disputed);
        if toc.is_empty() { None } else { Some(toc) }
    };
    if let Some(toc) = toc_opt {
        let hits = retrieval::fuse_and_rank(
            conn,
            Some(toc),
            None,
            cfg.keyword_weight,
            cfg.halflife,
            cfg.max_per_source,
            cfg.top_k,
            cfg.qa_exclude_disputed,
        );
        return apply_graph(conn, hits, scope.as_ref(), cfg);
    }
    // R8.1: hybrid search — vector + keyword
    let vector_hits = query_embedding.as_ref().and_then(|emb| {
        let hits = retrieval::vector_search(conn, emb, scope.as_ref(), cfg.top_k, cfg.qa_exclude_disputed);
        if hits.is_empty() { None } else { Some(hits) }
    });
    let keyword_hits = retrieval::keyword_search(conn, query, scope.as_ref(), cfg.top_k, cfg.qa_exclude_disputed);
    let evidence = if keyword_hits.is_empty() {
        if vector_hits.is_none() {
            retrieval::keyword_fallback(conn, scope.as_ref(), cfg.top_k, cfg.qa_exclude_disputed)
        } else {
            vec![]
        }
    } else {
        keyword_hits
    };
    let keyword_opt = if evidence.is_empty() { None } else { Some(evidence) };
    let hits = retrieval::fuse_and_rank(
        conn,
        vector_hits,
        keyword_opt,
        cfg.keyword_weight,
        cfg.halflife,
        cfg.max_per_source,
        cfg.top_k,
        cfg.qa_exclude_disputed,
    );
    apply_graph(conn, hits, scope.as_ref(), cfg)
}

fn retrieve_sync(
    conn: &Connection,
    question: &str,
    project_id: Option<i64>,
    public_only: bool,
    _settings_map: &std::collections::HashMap<String, String>,
    query_embedding: Option<Vec<f32>>,
) -> Vec<crate::retrieval::Hit> {
    let scope = project_scope(conn, project_id, public_only);
    let cfg = RetrievalConfig::from_env();
    // B30 gate moved to ask() for recording + trusted-source hack (R6.4)
    orchestrate(conn, question, query_embedding, scope, &cfg)
}

fn project_source_ids(conn: &Connection, project_id: Option<i64>) -> Option<HashSet<i64>> {
    let pid = project_id?;
    let mut stmt = conn.prepare("SELECT source_id FROM project_sources WHERE project_id = ?1").ok()?;
    let rows: Vec<i64> = stmt.query_map([pid], |r| r.get(0)).ok()?.filter_map(|r| r.ok()).collect();
    if rows.is_empty() { Some(HashSet::new()) } else { Some(rows.into_iter().collect()) }
}

fn public_source_ids(conn: &Connection) -> HashSet<i64> {
    let mut stmt = conn.prepare("SELECT id FROM sources WHERE label = 'public'").unwrap();
    stmt.query_map([], |r| r.get::<_, i64>(0)).unwrap().filter_map(|r| r.ok()).collect()
}

fn gate_answer_hits(conn: &Connection, hits: &[retrieval::Hit]) -> (Vec<retrieval::Hit>, Vec<String>) {
    let sids: HashSet<i64> = hits.iter().filter_map(|h| h.source_id).collect();
    let mut labels: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if !sids.is_empty() {
        let sql = format!("SELECT id, label FROM sources WHERE id IN ({})", crate::common::placeholders(sids.len()));
        if let Ok(mut stmt) = conn.prepare(&sql) {
            for row in stmt.query_map(rusqlite::params_from_iter(sids.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
                labels.insert(row.0, row.1);
            }
        }
    }
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    for h in hits {
        let label = h.source_id.and_then(|id| labels.get(&id).cloned()).unwrap_or_else(|| "internal".to_string());
        let is_pii = h.is_pii;
        if label=="sensitive" || label=="pii" || is_pii {
            blocked.push(label);
        } else {
            allowed.push(h.clone());
        }
    }
    (allowed, blocked)
}

pub(crate) fn is_answer_trusted(settings: &SettingsService) -> bool {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    crate::common::provider_trusted(&base)
}

/// Phase-1 deterministic gating for single-entity reads (get/related/context).
/// Mirrors gate_answer_hits: source label sensitive/pii, or PII-flagged chunks
/// on the entity's item, means an untrusted provider may not read it.
/// Returns Some(reason) when blocked, None when readable.
pub(crate) fn gate_entity_content(conn: &Connection, entity_id: i64) -> Option<String> {
    let item_id: i64 = conn.query_row("SELECT item_id FROM entities WHERE id=?1", [entity_id], |r| r.get(0)).ok()?;
    let (source_id, label): (i64, String) = conn
        .query_row(
            "SELECT s.id, s.label FROM sources s JOIN ingested_items i ON i.source_id = s.id WHERE i.id = ?1",
            [item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let pii: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks WHERE item_id = ?1 AND is_pii = 1", [item_id], |r| r.get(0))
        .unwrap_or(0);
    if label == "sensitive" || label == "pii" || pii > 0 {
        Some(format!(
            "entity {} blocked: source {} label '{}'{}, untrusted answer provider",
            entity_id,
            source_id,
            label,
            if pii > 0 { " + PII-flagged chunks" } else { "" }
        ))
    } else {
        None
    }
}

pub async fn search(
    settings: &SettingsService,
    embedder: &Arc<Embedder>,
    req: SearchRequest,
    data_dir: &str,
) -> SearchResponse {
    let query_for_embed = req.query.clone();
    let query_embedding = embedder.embed_query(&query_for_embed).await;
    let data_dir_string = data_dir.to_string();
    let data_dir_for_block = data_dir_string.clone();
    let query = req.query.clone();
    let k = req.k.unwrap_or(8).clamp(1, 50);
    let project_id = req.project_id;
    let public_only = req.public_only.unwrap_or(false);
    // A3: compact-by-default — full chunk bytes only on explicit opt-in.
    let want_content = req
        .fields
        .as_deref()
        .map(|f| f.split(',').any(|t| { let t = t.trim(); t == "content" || t == "all" }))
        .unwrap_or(false);
    // A3: cursor pagination over the gated list (deterministic full walks).
    let limit = req.limit.unwrap_or(k).clamp(1, 50);
    let cursor = req.cursor.unwrap_or(0);
    let _settings_clone = settings_snapshot(settings);
    let hits = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir_for_block);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        search_sync_inner(&conn, &query, k, project_id, public_only, query_embedding)
    })
    .await
    .unwrap_or_default();
    // Phase-1: same provider-trust gate as ask — an untrusted provider must not
    // receive sensitive/PII chunk content through the deterministic read path.
    let mut hits = hits;
    let mut blocked_count = 0usize;
    let mut refusal: Option<String> = None;
    if !is_answer_trusted(settings) && !hits.is_empty() {
        let hits_for_gate = hits.clone();
        let data_dir_gate = data_dir_string.clone();
        let (filtered, blocked) = tokio::task::spawn_blocking(move || {
            let db_path = crate::db::resolve_db_path(&data_dir_gate);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            gate_answer_hits(&conn, &hits_for_gate)
        })
        .await
        .unwrap_or((vec![], vec![]));
        blocked_count = blocked.len();
        if !blocked.is_empty() {
            record_gate_event(&data_dir_string, &blocked).await;
            if filtered.is_empty() {
                refusal = Some("gated_provider".to_string());
            }
        }
        hits = filtered;
    }
    // Paginate after gating so a harness sees a stable walk of exactly what
    // it may read; blocked/refusal still describe the whole gated set.
    let total_allowed = hits.len();
    let page: Vec<crate::retrieval::Hit> = hits.into_iter().skip(cursor).take(limit).collect();
    let next_cursor = if cursor + page.len() < total_allowed { Some(cursor + page.len()) } else { None };
    let db_path = crate::db::resolve_db_path(&data_dir_string);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    let ids: Vec<i64> = page.iter().map(|h| h.chunk_id).collect();
    let tag_map = tags_for_chunks(&conn, &ids);
    let hits = page.into_iter().map(|h| {
        SearchHit {
            chunk_id: Some(h.chunk_id),
            entity_id: h.entity_id,
            kind: hit_kind(&h),
            summary: take_chars(&h.content, 200),
            content: if want_content { take_chars(&h.content, 4000) } else { String::new() },
            source_ref: display_source_ref(&h),
            path: h.path.clone(),
            tags: tag_map.get(&h.chunk_id).cloned().unwrap_or_default(),
            source_id: h.source_id,
            item_id: h.item_id,
            item_title: String::new(),
            score: rounded_score(h.score),
        }
    }).collect();
    SearchResponse { hits, blocked: blocked_count, refusal, next_cursor }
}

fn search_sync_inner(conn: &Connection, query: &str, k: usize, project_id: Option<i64>, public_only: bool, query_embedding: Option<Vec<f32>>) -> Vec<crate::retrieval::Hit> {
    let scope = project_scope(conn, project_id, public_only);
    let cfg = RetrievalConfig::from_env_with_top_k(k);
    orchestrate(conn, query, query_embedding, scope, &cfg)
}

fn turn_text(turn: &AskTurn) -> String {
    format!("{}: {}", turn.role, turn.content)
}

async fn llm_rewrite(settings: &SettingsService, transcript: &str, question: &str) -> Option<String> {
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().ok()?;
    let mut headers = reqwest::header::HeaderMap::new();
    if !key.is_empty() {
        if let Ok(h) = format!("Bearer {}", key).parse() {
            headers.insert(reqwest::header::AUTHORIZATION, h);
        }
    }
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You rewrite a user's follow-up question into a standalone question that includes all context from the conversation needed to answer it alone. Respond with ONLY the rewritten question, no preamble."},
            {"role": "user", "content": format!("Conversation:\n{}\n\nFollow-up: {}", transcript, question)}
        ],
        "temperature": 0.0,
        "max_tokens": 120
    });
    let resp = client.post(&url).headers(headers).json(&payload).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let j = resp.json::<serde_json::Value>().await.ok()?;
    let content = j.pointer("/choices/0/message/content").and_then(|v| v.as_str())?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(500).collect())
    }
}

fn heuristic_rewrite(question: &str, history: &[AskTurn]) -> Option<String> {
    // Fallback heuristic — only on pronoun/coref, not blanket len<40 (R11.4)
    let q_lower = question.to_lowercase();
    let is_followup = q_lower.split_whitespace().any(|w| matches!(w, "it" | "its" | "this" | "that" | "these" | "those" | "they" | "them" | "their" | "itself"))
        || q_lower.contains("its ") || q_lower.contains(" for it");
    if !is_followup {
        return None;
    }
    let last_user = history.iter().rev().find(|t| t.role == "user").map(|t| t.content.clone()).or_else(|| history.last().map(|t| t.content.clone()))?;
    Some(format!("{} {}", last_user, question).chars().take(500).collect())
}

async fn rewrite_followup(settings: &SettingsService, question: &str, history: &[AskTurn]) -> Option<String> {
    if history.is_empty() {
        return None;
    }
    // B30: don't rewrite to untrusted cloud — Python returns None when no key and non-local
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    // if no history or same as question, skip
    let transcript = history.iter().rev().take(6).rev().map(turn_text).collect::<Vec<_>>().join("\n");
    if transcript.is_empty() {
        return None;
    }
    // R11.4: Python returns None without a model — don't pollute new-topic short questions
    if !is_local && key.is_empty() {
        return None;
    }
    if base.starts_with("http") {
        if let Some(r) = llm_rewrite(settings, &transcript, question).await {
            return Some(r);
        }
    }
    heuristic_rewrite(question, history)
}

async fn generate_answer(settings: &SettingsService, question: &str, context: &str, history: &[AskTurn]) -> String {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    if !is_local && key.is_empty() {
        return format!("Answer for: {}\n\n[No model key configured. Matching context:]\n\n{}", question, context.chars().take(1500).collect::<String>());
    }
    // For R2.2, we do not yet call LLM; return context stub (will be wired in R3.4)
    // Try to call LLM if configured (best-effort, like Python fallback)
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(_) => return format!("Answer for: {}\n\n{}", question, context.chars().take(1500).collect::<String>()),
    };
    let mut headers = reqwest::header::HeaderMap::new();
    if !key.is_empty() {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", key).parse().unwrap());
    }
    let user_content = if history.is_empty() {
        format!("Question: {}\n\nContext:\n{}", question, context)
    } else {
        // R11.7: Python parity — only last 6 turns (like backend/app/answer_engine.py:_generate history[-6:])
        let start = history.len().saturating_sub(6);
        let transcript = history[start..].iter().map(turn_text).collect::<Vec<_>>().join("\n");
        format!("Conversation so far:\n{}\n\nQuestion: {}\n\nContext:\n{}", transcript, question, context)
    };
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are AnchorCore, answer using only context with citations [S1] etc."},
            {"role": "user", "content": user_content}
        ],
        "temperature": 0.2
    });
    match client.post(&url).headers(headers).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(j) = resp.json::<serde_json::Value>().await {
                if let Some(content) = j.pointer("/choices/0/message/content").and_then(|v| v.as_str()) {
                    return content.to_string();
                }
            }
            format!("Answer for: {}\n\n{}", question, context.chars().take(1500).collect::<String>())
        }
        _ => format!("Answer for: {}\n\n[Answer model unreachable; showing context]\n\n{}", question, context.chars().take(1500).collect::<String>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Seed a temp DB: one ordinary + one sensitive source, each with an item,
    /// a chunk with distinctive content, and an entity. Returns (data_dir, conn).
    fn seed_gate_db() -> (tempfile::TempDir, Connection, Arc<SettingsService>) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (connector, name) VALUES ('folder','ordinary')", [])
            .unwrap();
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','secret','sensitive')", [])
            .unwrap();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let sen: i64 = conn.query_row("SELECT id FROM sources WHERE name='secret'", [], |r| r.get(0)).unwrap();
        for (sid, word) in [(ord, "sunshine"), (sen, "moonvault")] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, content_hash, title, text) VALUES (?1,'h','t',?2)",
                rusqlite::params![sid, format!("quarterly {} report numbers", word)],
            )
            .unwrap();
            let item: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO chunks (item_id, source_ref, content) VALUES (?1,'f.md',?2)",
                rusqlite::params![item, format!("quarterly {} report numbers", word)],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision',?2)",
                rusqlite::params![item, format!("{} entity", word)],
            )
            .unwrap();
        }
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(dir.path());
        // Hermetic embeddings: closed port refuses instantly, so tests fall
        // back to deterministic_embed instead of stalling on DNS/connect.
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        (dir, conn, settings)
    }

    fn mk_hit(source_id: i64, pii: bool) -> crate::retrieval::Hit {
        crate::retrieval::Hit {
            chunk_id: 1,
            entity_id: None,
            item_id: None,
            source_id: Some(source_id),
            section_id: None,
            path: String::new(),
            score: 1.0,
            content: "x".to_string(),
            source_ref: "f.md".to_string(),
            is_pii: pii,
            status: None,
        }
    }

    #[test]
    fn gate_answer_hits_filters_sensitive_and_pii() {
        let (_dir, conn, _settings) = seed_gate_db();
        let ord: i64 = conn.query_row("SELECT id FROM sources WHERE name='ordinary'", [], |r| r.get(0)).unwrap();
        let sen: i64 = conn.query_row("SELECT id FROM sources WHERE name='secret'", [], |r| r.get(0)).unwrap();
        let (allowed, blocked) = gate_answer_hits(&conn, &[mk_hit(ord, false), mk_hit(sen, false), mk_hit(ord, true)]);
        assert_eq!(allowed.len(), 1);
        assert_eq!(blocked.len(), 2);
    }

    #[test]
    fn gate_entity_content_blocks_sensitive_source() {
        let (_dir, conn, _settings) = seed_gate_db();
        let eid_ord: i64 = conn.query_row("SELECT id FROM entities WHERE summary LIKE 'sunshine%'", [], |r| r.get(0)).unwrap();
        let eid_sen: i64 = conn.query_row("SELECT id FROM entities WHERE summary LIKE 'moonvault%'", [], |r| r.get(0)).unwrap();
        assert!(gate_entity_content(&conn, eid_ord).is_none());
        let reason = gate_entity_content(&conn, eid_sen).expect("sensitive entity must be gated");
        assert!(reason.contains("sensitive"), "reason: {}", reason);
        assert!(gate_entity_content(&conn, 999999).is_none(), "missing entity must not gate (404 path owns it)");
    }

    #[tokio::test]
    async fn search_gated_provider_refusal() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        // Default settings: example remote base is untrusted (no ANCHOR_CLOUD_TRUST here).
        settings.set("answer_base_url", "https://untrusted.example.invalid/v1", &conn).unwrap();
        let req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert!(res.hits.is_empty(), "sensitive hit must be withheld, got {:?}", res.hits.len());
        assert_eq!(res.refusal.as_deref(), Some("gated_provider"));
        assert!(res.blocked >= 1);
    }

    #[tokio::test]
    async fn search_trusted_provider_returns_hits() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        // Localhost is trusted in every environment — deterministic trust.
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        assert!(is_answer_trusted(&settings));
        let req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None };
        let res = search(&settings, &embedder, req, &data_dir).await;
        assert!(!res.hits.is_empty(), "trusted provider must see the sensitive hit");
        assert!(res.refusal.is_none());
        assert_eq!(res.blocked, 0);
    }

    #[tokio::test]
    async fn search_compact_by_default_full_on_opt_in() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        let base = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: None, cursor: None, limit: None };
        let compact = search(&settings, &embedder, base, &data_dir).await;
        assert!(!compact.hits.is_empty());
        assert!(compact.hits.iter().all(|h| h.content.is_empty()), "default must omit chunk bytes");
        assert!(compact.hits.iter().all(|h| !h.summary.is_empty() && !h.source_ref.is_empty()));
        let full_req = SearchRequest { query: "moonvault".to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("summary,source_ref,score,content".to_string()), cursor: None, limit: None };
        let full = search(&settings, &embedder, full_req, &data_dir).await;
        assert!(full.hits.iter().all(|h| !h.content.is_empty()), "fields incl. content must return bytes");
    }

    #[tokio::test]
    async fn search_cursor_pagination_walks_all_hits() {
        let (dir, conn, settings) = seed_gate_db();
        let data_dir = dir.path().to_string_lossy().to_string();
        let embedder = Arc::new(Embedder::new(settings.clone()));
        settings.set("answer_base_url", "http://127.0.0.1:11434/v1", &conn).unwrap();
        // Both seeded chunks share these words, so one page of 1 must leave a next cursor.
        let mk = |cursor: Option<usize>| SearchRequest { query: "quarterly report numbers".to_string(), k: Some(8), project_id: None, public_only: None, fields: Some("all".to_string()), cursor, limit: Some(1) };
        let p1 = search(&settings, &embedder, mk(None), &data_dir).await;
        assert_eq!(p1.hits.len(), 1, "page size 1 must return exactly one hit");
        let next = p1.next_cursor.expect("first page must carry a cursor when hits remain");
        let p2 = search(&settings, &embedder, mk(Some(next)), &data_dir).await;
        assert_eq!(p2.hits.len(), 1);
        assert!(p2.next_cursor.is_none(), "second page must exhaust the two seeded hits");
        let mut ids: Vec<Option<i64>> = vec![p1.hits[0].chunk_id, p2.hits[0].chunk_id];
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 2, "two pages must cover two distinct chunks");
    }
}
