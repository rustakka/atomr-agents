//! Axum REST + SSE companion for the [`SandboxHarness`].
//!
//! Endpoints (all JSON):
//! - `POST   /run`                      — ephemeral one-shot exec → flat result
//! - `POST   /sandboxes`                — create a persistent sandbox → SandboxInfo
//! - `GET    /sandboxes`                — list live sandboxes
//! - `POST   /sandboxes/:id/exec`       — exec in a sandbox → flat result
//! - `POST   /sandboxes/:id/fork`       — fork → SandboxInfo
//! - `POST   /sandboxes/:id/snapshot`   — snapshot → { snapshot_id }
//! - `DELETE /sandboxes/:id`            — destroy
//! - `GET    /events`                   — SSE stream of SandboxEvents
//! - `GET    /healthz`                  — liveness
//!
//! Mirrors `channel-harness-web`.

#![forbid(unsafe_code)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;

use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, SandboxBackendSel, SandboxError, SandboxId, SandboxProfile,
};
use atomr_agents_sandbox_harness::SandboxHarness;

/// Web server configuration.
#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { bind: SocketAddr::from(([0, 0, 0, 0], 8080)) }
    }
}

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub harness: Arc<SandboxHarness>,
}

/// HTTP error wrapper mapping [`SandboxError`] to status codes.
struct ApiError(SandboxError);

impl From<SandboxError> for ApiError {
    fn from(e: SandboxError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            SandboxError::NotFound(_) => StatusCode::NOT_FOUND,
            SandboxError::Unsupported(_) | SandboxError::BudgetViolation(_) => {
                StatusCode::BAD_REQUEST
            }
            SandboxError::Timeout => StatusCode::REQUEST_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

/// Body for `POST /run` — an exec plus an optional profile.
#[derive(Debug, Deserialize)]
struct RunRequest {
    #[serde(default)]
    profile: Option<SandboxProfile>,
    #[serde(flatten)]
    exec: ExecRequest,
}

/// Build the router over the given state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/run", post(run_once))
        .route("/sandboxes", post(create_sandbox).get(list_sandboxes))
        .route("/sandboxes/:id/exec", post(exec_sandbox))
        .route("/sandboxes/:id/fork", post(fork_sandbox))
        .route("/sandboxes/:id/snapshot", post(snapshot_sandbox))
        .route("/sandboxes/:id", delete(destroy_sandbox))
        .route("/events", get(events))
        .route("/healthz", get(healthz))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "backend": state.harness.backend_name(),
        "live": state.harness.live_count(),
    }))
}

async fn run_once(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let res = state
        .harness
        .run_exec(req.exec, req.profile, SandboxBackendSel::Auto)
        .await?;
    Ok(Json(res.to_tool_json()))
}

async fn create_sandbox(
    State(state): State<AppState>,
    Json(req): Json<CreateSandbox>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let info = state.harness.create(req).await?;
    Ok(Json(serde_json::to_value(info).unwrap_or_default()))
}

async fn list_sandboxes(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.harness.list()).unwrap_or_default())
}

async fn exec_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExecRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let res = state.harness.exec(&SandboxId::from(id), req).await?;
    Ok(Json(res.to_tool_json()))
}

async fn fork_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let info = state.harness.fork(&SandboxId::from(id)).await?;
    Ok(Json(serde_json::to_value(info).unwrap_or_default()))
}

async fn snapshot_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = state.harness.snapshot(&SandboxId::from(id)).await?;
    Ok(Json(json!({ "snapshot_id": snap.to_string() })))
}

async fn destroy_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.harness.destroy(&SandboxId::from(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.harness.event_sender().subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => Some(Ok(SseEvent::default()
                .json_data(ev)
                .unwrap_or_else(|_| SseEvent::default().data("serialize_error")))),
            // Drop lagged notifications silently.
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// A ready-to-serve web server.
pub struct WebServer {
    config: WebConfig,
    state: AppState,
}

impl WebServer {
    pub fn new(config: WebConfig, harness: Arc<SandboxHarness>) -> Self {
        Self { config, state: AppState { harness } }
    }

    pub fn router(&self) -> Router {
        router(self.state.clone())
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.config.bind
    }

    /// Bind and serve until the process is terminated.
    pub async fn serve(self) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(self.config.bind).await?;
        axum::serve(listener, self.router()).await
    }
}
