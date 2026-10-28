//! Route table — moved out of `main.rs` when the crate gained a `lib`
//! target, so unit tests (e.g. the auth route-walk) can build the router
//! without the binary root.

use axum::{
    middleware,
    routing::{delete, get, post},
    Router,
};

use crate::{
    answer, assets_api, auth, backup, entities, frontend, fsbrowse, glossary, health, items, jobs,
    mcp_setup, ollama, projects, records_api, report, review, sections, security, settings,
    skills_setup, sources, stubs, system, tags,
};

pub fn build_router(state: health::AppState) -> Router {
    // R1.3: stub all routers with 501, keep /health real (already done in R1.2)
    // Note: axum 0.7 uses `/:id` style; keep literal routes before param routes
    Router::new()
        .route("/health", get(health::health))
        // sources (R4.5 + pipeline sync)
        .route("/sources", get(sources::list_handler).post(sources::create_handler))
        .route("/sources/jira/projects", post(stubs::not_implemented))
        .route("/sources/rest/preview", post(sources::rest_preview_handler))
        .route("/sources/jobs", get(jobs::list_handler))
        .route("/sources/jobs/running", get(jobs::running_handler))
        .route("/sources/jobs/:id", get(jobs::get_handler))
        .route("/sources/jobs/:id/cancel", post(jobs::cancel_handler))
        .route("/sources/:id", get(sources::get_handler).put(sources::update_handler).delete(sources::delete_handler))
        .route("/sources/:id/config", get(sources::config_handler))
        .route("/sources/:id/sync", post(sources::sync_handler))
        .route("/sources/:id/reclassify", post(sources::reclassify_handler))
        // entities + review (R4.4)
        .route("/entities", get(entities::list_handler))
        .route("/entities/:id", get(entities::get_handler).patch(entities::patch_handler))
        .route("/entities/:id/related", get(entities::related_handler))
        .route("/entities/:id/dispute", post(entities::dispute_handler))
        .route("/entities/:id/disputes", get(entities::disputes_handler))
        .route("/entities/:id/conflicts", get(entities::conflicts_list_handler).post(entities::conflict_create_handler))
        .route("/entities/:id/conflicts/:other_id", delete(entities::conflict_delete_handler))
        // R45.9: decision log + action tracker (MCP `decisions`/`actions` wrap these)
        .route("/decisions", get(records_api::decisions_handler))
        .route("/actions", get(records_api::actions_handler))
        .route("/items", get(items::list_handler))
        .route("/items/:id", get(items::get_handler).patch(items::patch_handler))
        .route("/items/:id/outline", get(sections::outline_handler))
        // R46.9: asset bytes from the DB (not `/assets/*` — that is the public SPA bundle dir)
        .route("/media", get(assets_api::list_handler))
        .route("/media/:uid", get(assets_api::media_handler).patch(assets_api::patch_handler))
        .route("/entities/:id/context", get(entities::context_handler))
        .route("/review/low-confidence", get(review::low_confidence_handler))
        .route("/review/duplicates", get(review::duplicates_handler))
        .route("/review/merge", post(review::merge_handler))
        // R45.13: people-owned glossary (replaces /review/vocabulary)
        .route("/glossary", get(glossary::list_handler).post(glossary::create_handler))
        .route("/glossary/:id", delete(glossary::delete_handler))
        // projects (R4.4)
        .route("/projects", get(projects::list_handler).post(projects::create_handler))
        .route("/projects/default", get(projects::default_handler))
        .route("/projects/:id", get(projects::get_handler).patch(projects::patch_handler).delete(projects::delete_handler))
        // sections (R14.1)
        .route("/sections", get(sections::list_handler))
        .route("/sections/:id/chunks", get(sections::chunks_handler))
        // nodes (R36.6) — single tree-node lookup with opt-in links
        .route("/nodes/:id", get(sections::node_handler))
        // tags (R14.4)
        .route("/tags", get(tags::list_handler))
        .route("/tags/:id/chunks", get(tags::chunks_handler))
        // qa (R2.2 + R4.5 search)
        .route("/qa", post(qa_handler))
        .route("/qa/public", post(qa_public_handler))
        .route("/qa/search", post(search_handler))
        // settings (R6.1)
        .route("/settings", get(settings::get_handler).put(settings::put_handler))
        .route("/settings/test-connection", post(settings::test_connection_handler))
        // system (R6.2 + R11 manual ollama start)
        .route("/system/status", get(system::status_handler))
        .route("/system/onboarding", get(system::onboarding_handler))
        .route("/system/update", get(system::update_handler))
        .route("/system/errors", get(crate::events_api::errors_handler))
        .route("/system/errors/summary", get(crate::events_api::summary_handler))
        .route("/system/errors/dismiss", post(crate::events_api::dismiss_handler))
        .route("/system/errors/restore", post(crate::events_api::restore_handler))
        .route("/system/fs", get(fsbrowse::fs_browse_handler))
        .route("/system/mcp", get(system::mcp_info_handler))
        .route("/system/mcp/clients", get(mcp_setup::clients_handler))
        .route("/system/mcp/connect", post(mcp_setup::connect_handler))
        .route("/system/mcp/disconnect", post(mcp_setup::disconnect_handler))
        .route("/system/logs", get(system::logs_handler))
        .route("/system/logs/:name", get(system::log_download_handler))
        .route("/system/report", get(report::report_handler))
        .route("/system/embed-model", get(crate::embed_model_api::status_handler))
        .route("/system/embed-model/download", post(crate::embed_model_api::download_handler))
        .route("/system/ollama/start", post(system::ollama_start_handler))
        .route("/system/ollama/pull", post(ollama::pull_handler))
        .route("/system/ollama/pulls", get(ollama::pulls_handler))
        .route("/system/skills/:name", get(system::skills_handler))
        .route("/system/skills/install-targets", get(skills_setup::install_targets_handler))
        .route("/system/skills/:name/install", post(skills_setup::install_handler))
        // auth (R6.3 B40)
        .route("/auth/status", get(auth::status_handler))
        .route("/auth/signup", post(auth::signup_handler))
        .route("/auth/login", post(auth::login_handler))
        .route("/auth/me", get(auth::me_handler))
        // R7.1: CSRF token endpoint (public, used by SPA to fetch per-session token)
        .route("/csrf", get(security::csrf_handler))
        // backup export (R25.2)
        .route("/backup/export", post(backup::export_handler))
        // backup import (R25.3)
        .route(
            "/backup/import",
            post(backup::import_handler).layer(axum::extract::DefaultBodyLimit::max(backup::IMPORT_BODY_LIMIT)),
        )
        // frontend (R5.1) — must be last, SPA fallback to index.html
        .fallback(frontend::handler)
        // Layers: outermost -> innermost: Host/Origin -> CORS -> CSRF -> Auth
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), security::csrf_middleware))
        .layer(security::cors_layer())
        .layer(middleware::from_fn_with_state(state.clone(), security::host_origin_middleware))
        .with_state(state)
}

async fn qa_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    headers: axum::http::HeaderMap,
    axum::Json(req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    let question = req.question.clone();
    let scope = report::scope_label(req.project_id, req.public_only.unwrap_or(false));
    let ui_caller = crate::read_gate::caller_is_ui(&headers);
    let resp = answer::ask(&state.settings, &state.embedder, req, &state.data_dir, ui_caller).await;
    report::record_ask(&question, scope, &resp);
    axum::Json(resp)
}

async fn qa_public_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    headers: axum::http::HeaderMap,
    axum::Json(mut req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    req.public_only = Some(true);
    let question = req.question.clone();
    let scope = report::scope_label(req.project_id, true);
    let ui_caller = crate::read_gate::caller_is_ui(&headers);
    let resp = answer::ask(&state.settings, &state.embedder, req, &state.data_dir, ui_caller).await;
    report::record_ask(&question, scope, &resp);
    axum::Json(resp)
}

async fn search_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    headers: axum::http::HeaderMap,
    axum::Json(req): axum::Json<answer::SearchRequest>,
) -> Result<axum::Json<answer::SearchResponse>, (axum::http::StatusCode, axum::Json<serde_json::Value>)> {
    // R36.15: a malformed path_prefix is a 400 naming the expected form
    // (same check as `GET /items`), never a silently empty hit list.
    items::validate_path_prefix(req.path_prefix.as_deref())
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"detail": e}))))?;
    let query = req.query.clone();
    let scope = report::scope_label(req.project_id, req.public_only.unwrap_or(false));
    let ui_caller = crate::read_gate::caller_is_ui(&headers);
    let resp = answer::search(&state.settings, &state.embedder, req, &state.data_dir, ui_caller).await;
    report::record_search(&query, scope, &resp);
    Ok(axum::Json(resp))
}
