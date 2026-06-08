//! Axum REST + SSE companion for the [`AgentSdkHarness`].
//!
//! Endpoints (all JSON):
//! - `POST   /run`                          — headless query → `ResultSummary`
//! - `POST   /sessions`                     — open an interactive session → `{ session_id }`
//! - `GET    /sessions`                     — list live session ids
//! - `POST   /sessions/:id/messages`        — send a prompt (`{ prompt }`)
//! - `POST   /sessions/:id/interrupt`       — interrupt the in-flight turn
//! - `POST   /sessions/:id/permission-mode` — change mode (`{ mode }`)
//! - `POST   /sessions/:id/model`           — change model (`{ model }`)
//! - `DELETE /sessions/:id`                 — close the session
//! - `GET    /events`                       — SSE stream of `AgentSdkEvent`
//! - `GET    /healthz`                      — liveness
//!
//! Mirrors `sandbox-harness-web`.

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

use atomr_agents_agent_sdk_core::{AgentSessionId, QueryRequest, SessionSpec};
use atomr_agents_agent_sdk_harness::{AgentSdkHarness, HarnessError};

/// Web server configuration.
#[derive(Debug, Clone)]
pub struct WebConfig {
    pub bind: SocketAddr,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 8080)),
        }
    }
}

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    pub harness: Arc<AgentSdkHarness>,
}

/// HTTP error wrapper mapping [`HarnessError`] to status codes.
struct ApiError(HarnessError);

impl From<HarnessError> for ApiError {
    fn from(e: HarnessError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match &self.0 {
            HarnessError::SessionNotFound(_) => StatusCode::NOT_FOUND,
            HarnessError::InvalidRequest(_)
            | HarnessError::InvalidWorkdir(_)
            | HarnessError::SessionQuota(_) => StatusCode::BAD_REQUEST,
            HarnessError::Budget(_) => StatusCode::PAYMENT_REQUIRED,
            HarnessError::PolicyDenied(_) => StatusCode::FORBIDDEN,
            HarnessError::StreamClosed | HarnessError::Sdk(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct MessageRequest {
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct PermissionModeRequest {
    mode: String,
}

#[derive(Debug, Deserialize)]
struct ModelRequest {
    model: String,
}

/// Build the router over the given state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/run", post(run_once))
        .route("/sessions", post(create_session).get(list_sessions))
        .route("/sessions/:id/messages", post(send_message))
        .route("/sessions/:id/interrupt", post(interrupt_session))
        .route("/sessions/:id/permission-mode", post(set_permission_mode))
        .route("/sessions/:id/model", post(set_model))
        .route("/sessions/:id", delete(stop_session))
        .route("/events", get(events))
        .route("/healthz", get(healthz))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "backend": state.harness.backend_name(),
        "live_sessions": state.harness.live_count(),
    }))
}

async fn run_once(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let res = state.harness.run(req).await?;
    Ok(Json(serde_json::to_value(res).unwrap_or_default()))
}

async fn create_session(
    State(state): State<AppState>,
    Json(spec): Json<SessionSpec>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state.harness.start_session(spec).await?;
    Ok(Json(json!({ "session_id": session.id.to_string() })))
}

async fn list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let ids: Vec<String> = state
        .harness
        .sessions()
        .list()
        .iter()
        .map(|s| s.id.to_string())
        .collect();
    Json(json!({ "sessions": ids }))
}

async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> Result<StatusCode, ApiError> {
    let session = state
        .harness
        .sessions()
        .get(&AgentSessionId::from(id.clone()))
        .ok_or(HarnessError::SessionNotFound(id))?;
    session.send(req.prompt).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn interrupt_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session = state
        .harness
        .sessions()
        .get(&AgentSessionId::from(id.clone()))
        .ok_or(HarnessError::SessionNotFound(id))?;
    session.interrupt().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_permission_mode(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PermissionModeRequest>,
) -> Result<StatusCode, ApiError> {
    let session = state
        .harness
        .sessions()
        .get(&AgentSessionId::from(id.clone()))
        .ok_or(HarnessError::SessionNotFound(id))?;
    session.set_permission_mode(req.mode).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ModelRequest>,
) -> Result<StatusCode, ApiError> {
    let session = state
        .harness
        .sessions()
        .get(&AgentSessionId::from(id.clone()))
        .ok_or(HarnessError::SessionNotFound(id))?;
    session.set_model(req.model).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.harness.stop_session(&AgentSessionId::from(id)).await?;
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
    pub fn new(config: WebConfig, harness: Arc<AgentSdkHarness>) -> Self {
        Self {
            config,
            state: AppState { harness },
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::util::ServiceExt; // for `oneshot`

    fn app() -> Router {
        router(AppState {
            harness: Arc::new(AgentSdkHarness::local_default()),
        })
    }

    #[tokio::test]
    async fn healthz_ok() {
        let res = app()
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["backend"], "mock");
    }

    #[tokio::test]
    async fn run_returns_result() {
        let req = Request::builder()
            .method("POST")
            .uri("/run")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"prompt":"hi"}"#))
            .unwrap();
        let res = app().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["subtype"], "success");
    }
}
