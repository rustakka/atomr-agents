//! REST + SSE + WS route registration.

use std::sync::Arc;

use atomr_agents_coding_cli_core::{CliRequest, CliRunId, CliSessionId};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::error::WebError;
use crate::AppState;

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/cli/vendors", get(list_vendors))
        .route("/cli/runs", get(list_runs).post(start_run))
        .route("/cli/runs/:id", get(get_run))
        .route("/cli/runs/events", get(crate::sse::sse_events))
        .route(
            "/cli/sessions",
            get(list_sessions).post(start_session),
        )
        .route("/cli/sessions/:id", delete(stop_session))
        .route("/cli/sessions/:id/io", get(crate::ws::session_ws))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/healthz", get(|| async { "ok" }))
        .fallback(crate::spa::serve_embedded)
        .with_state(state)
        .layer(CorsLayer::permissive())
}

// ----- handlers ---------------------------------------------------------

async fn list_vendors(State(state): State<AppState>) -> impl IntoResponse {
    let kinds: Vec<_> = state
        .harness
        .available_vendors()
        .into_iter()
        .map(|k| k.as_str().to_string())
        .collect();
    Json(json!({ "vendors": kinds }))
}

async fn start_run(
    State(state): State<AppState>,
    Json(req): Json<CliRequest>,
) -> Result<impl IntoResponse, WebError> {
    let harness = state.harness.clone();
    let run_id = CliRunId::new();
    let id_clone = run_id.clone();
    // Spawn so the HTTP request returns immediately. Errors land in
    // tracing + the run's store entry.
    let task = tokio::spawn(async move {
        if let Err(e) = harness.run(req).await {
            tracing::error!(run_id = %id_clone, error = %e, "headless run failed");
        }
    });
    state.supervisor.lock().register(run_id.clone(), task);
    Ok((StatusCode::ACCEPTED, Json(json!({ "run_id": run_id }))))
}

async fn list_runs(State(state): State<AppState>) -> Result<impl IntoResponse, WebError> {
    let runs = state.harness.store.list(50).await?;
    Ok(Json(json!({ "runs": runs })))
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, WebError> {
    let id = CliRunId::from(id);
    match state.harness.store.get(&id).await? {
        Some(r) => Ok(Json(r).into_response()),
        None => Ok((StatusCode::NOT_FOUND, Json(json!({"error":"run not found"}))).into_response()),
    }
}

async fn start_session(
    State(state): State<AppState>,
    Json(req): Json<CliRequest>,
) -> Result<impl IntoResponse, WebError> {
    let handle = state.harness.start_interactive(req).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "session_id": handle.id,
            "vendor": handle.vendor.as_str(),
            "tmux_session": handle.tmux_session,
            "started_at": handle.started_at,
        })),
    ))
}

async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sessions: Vec<_> = state
        .harness
        .sessions()
        .list()
        .into_iter()
        .map(|h: Arc<atomr_agents_coding_cli_harness::InteractiveSessionHandle>| {
            json!({
                "id": h.id,
                "vendor": h.vendor.as_str(),
                "tmux_session": h.tmux_session,
                "started_at": h.started_at,
                "closed": *h.closed.lock(),
            })
        })
        .collect();
    Json(json!({ "sessions": sessions }))
}

async fn stop_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, WebError> {
    let id = CliSessionId::from(id);
    state.harness.stop_interactive(&id).await?;
    Ok((StatusCode::NO_CONTENT, ()))
}
