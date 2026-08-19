//! R2.2: Answer orchestration — port of `backend/app/answer_engine.py:169` `ask()`
//! - project scoping, public_only gate, planner, executor, graph, B30 gate, generation

use crate::retrieval;
use crate::settings::SettingsService;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
}

#[derive(Serialize)]
pub struct Citation {
    pub entity_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub source_ref: String,
    pub score: f64,
    pub snippet: String,
}

pub async fn ask_stub(settings: &SettingsService, req: AskRequest) -> AskResponse {
    // Minimal stub for R2.2 to keep handler Send (no DB !Send across await)
    // Returns context-aware answer like Python fallback when no hits
    let _ = settings.get("answer_base_url", None);
    AskResponse {
        answer: format!("Answer for: {} (Rust stub R2.2 - DB retrieval pending)", req.question),
        citations: vec![],
    }
}

pub async fn ask(
    settings: &SettingsService,
    req: AskRequest,
    data_dir: &str,
) -> AskResponse {
    // Do DB retrieval in blocking thread to avoid holding !Send Connection across await
    let data_dir = data_dir.to_string();
    let question = req.question.clone();
    let project_id = req.project_id;
    let public_only = req.public_only.unwrap_or(false);
    let history = req.history.clone();
    let settings_clone = settings_snapshot(settings);
    let q_for_block = question.clone();
    let hits = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        retrieve_sync(&conn, &q_for_block, project_id, public_only, &settings_clone)
    })
    .await
    .unwrap_or_default();

    if hits.is_empty() {
        return AskResponse {
            answer: "No relevant knowledge found yet. Ingest sources first.".to_string(),
            citations: vec![],
        };
    }

    // Build context and generate (no DB borrow across await)
    let sections: Vec<String> = hits.iter().enumerate().map(|(i, h)| format!("[S{}] {}", i+1, &h.content[..h.content.len().min(2000)])).collect();
    let context = sections.join("\n\n");
    let answer = generate_answer(settings, &question, &context, history.as_deref().unwrap_or(&[])).await;
    let citations = hits.iter().take(5).map(|h| Citation {
        entity_id: h.entity_id,
        kind: if h.entity_id.is_some() { "entity".to_string() } else { "document".to_string() },
        summary: h.content.chars().take(200).collect(),
        source_ref: format!("chunk:{}", h.chunk_id),
        score: (h.score * 1000.0).round() / 1000.0,
        snippet: h.content.chars().take(300).collect(),
    }).collect();
    return AskResponse { answer, citations };
}

fn settings_snapshot(settings: &SettingsService) -> std::collections::HashMap<String, String> {
    // snapshot needed for blocking thread (SettingsService is Sync but we clone needed values)
    let mut m = std::collections::HashMap::new();
    for k in ["answer_base_url","answer_api_key","answer_model","ollama_base_url"] {
        if let Some(v) = settings.get(k, None) { m.insert(k.to_string(), v); }
    }
    m
}

fn retrieve_sync(
    conn: &Connection,
    question: &str,
    project_id: Option<i64>,
    public_only: bool,
    settings_map: &std::collections::HashMap<String, String>,
) -> Vec<crate::retrieval::Hit> {
    let project_ids = project_source_ids(conn, project_id);
    let mut source_ids = project_ids;
    if public_only {
        let public_ids = public_source_ids(conn);
        source_ids = match source_ids {
            None => Some(public_ids),
            Some(s) => Some(s.intersection(&public_ids).cloned().collect()),
        };
    }
    let query = question.to_string();
    let qa_exclude_disputed: bool = std::env::var("ANCHOR_QA_EXCLUDE_DISPUTED")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true);
    let top_k: usize = std::env::var("ANCHOR_TOP_K").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let keyword_weight: f64 = std::env::var("ANCHOR_RETRIEVAL_KEYWORD_WEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let halflife: f64 = std::env::var("ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(365.0);
    let max_per_source: usize = std::env::var("ANCHOR_RETRIEVAL_MAX_PER_SOURCE").ok().and_then(|v| v.parse().ok()).unwrap_or(3);

    let keyword_hits = retrieval::keyword_search(conn, &query, source_ids.as_ref(), top_k, qa_exclude_disputed);
    let evidence = if keyword_hits.is_empty() {
        retrieval::keyword_fallback(conn, source_ids.as_ref(), top_k, qa_exclude_disputed)
    } else {
        keyword_hits
    };
    let mut hits = retrieval::fuse_and_rank(
        conn,
        None,
        Some(evidence),
        keyword_weight,
        halflife,
        max_per_source,
        top_k,
        qa_exclude_disputed,
    );
    let graph_hits = retrieval::graph_expand(conn, &hits, source_ids.as_ref(), 2, 3, qa_exclude_disputed);
    if !graph_hits.is_empty() {
        let lists = vec![hits, graph_hits];
        hits = retrieval::fuse_evidence(lists, vec![1.0, 0.5]);
        hits.truncate(top_k);
    }
    // B30 gate inside blocking (like Python _gate_answer_hits)
    let base = settings_map.get("answer_base_url").cloned().unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    let trusted = if is_local { true } else { std::env::var("ANCHOR_CLOUD_TRUST").as_deref() == Ok("1") };
    if !trusted {
        let sids: HashSet<i64> = hits.iter().filter_map(|h| h.source_id).collect();
        let mut labels: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        if !sids.is_empty() {
            let list = sids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!("SELECT id, label FROM sources WHERE id IN ({})", list);
            if let Ok(mut stmt) = conn.prepare(&sql) {
                for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
                    labels.insert(row.0, row.1);
                }
            }
        }
        let mut allowed = Vec::new();
        for h in hits {
            let label = h.source_id.and_then(|id| labels.get(&id).cloned()).unwrap_or_else(|| "internal".to_string());
            if label=="sensitive" || label=="pii" || h.is_pii {
                continue;
            }
            allowed.push(h);
        }
        hits = allowed;
    }
    hits
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
        let list = sids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id, label FROM sources WHERE id IN ({})", list);
        if let Ok(mut stmt) = conn.prepare(&sql) {
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap().flatten() {
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

fn is_answer_trusted(settings: &SettingsService) -> bool {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    if base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1") { return true; }
    std::env::var("ANCHOR_CLOUD_TRUST").as_deref() == Ok("1")
}

async fn generate_answer(settings: &SettingsService, question: &str, context: &str, _history: &[AskTurn]) -> String {
    let base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    if !is_local && key.is_empty() {
        return format!("Answer for: {}\n\n[No model key configured. Matching context:]\n\n{}", question, &context[..context.len().min(1500)]);
    }
    // For R2.2, we do not yet call LLM; return context stub (will be wired in R3.4)
    // Try to call LLM if configured (best-effort, like Python fallback)
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(_) => return format!("Answer for: {}\n\n{}", question, &context[..context.len().min(1500)]),
    };
    let mut headers = reqwest::header::HeaderMap::new();
    if !key.is_empty() {
        headers.insert(reqwest::header::AUTHORIZATION, format!("Bearer {}", key).parse().unwrap());
    }
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are AnchorCore, answer using only context with citations [S1] etc."},
            {"role": "user", "content": format!("Question: {}\n\nContext:\n{}", question, context)}
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
            format!("Answer for: {}\n\n{}", question, &context[..context.len().min(1500)])
        }
        _ => format!("Answer for: {}\n\n[Answer model unreachable; showing context]\n\n{}", question, &context[..context.len().min(1500)]),
    }
}
