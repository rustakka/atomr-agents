//! HTTP-shaped errors for the web layer.

use atomr_agents_host::HostError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Host(#[from] HostError),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            WebError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            WebError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            WebError::Host(h) => (host_status(h), h.to_string()),
            WebError::Serde(_) | WebError::Yaml(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

/// Map a host-domain error onto an HTTP status. Missing resources are 404,
/// invalid user-supplied content is 400, state/precondition refusals (e.g.
/// "refusing to delete `main`", "no checkpoint to fork from") are 409, and
/// genuine infrastructure faults (io, actor system) stay 500.
fn host_status(err: &HostError) -> StatusCode {
    match err {
        HostError::AgentNotFound(..) => StatusCode::NOT_FOUND,
        HostError::AgentSpec(_)
        | HostError::Config(_)
        | HostError::Markdown { .. }
        | HostError::Skill { .. }
        | HostError::Hook { .. } => StatusCode::BAD_REQUEST,
        HostError::Branching(_)
        | HostError::Scheduler(_)
        | HostError::Registry(_)
        | HostError::Eval(_)
        | HostError::Mcp(_)
        | HostError::Gateway(_)
        | HostError::Curator(_)
        | HostError::HookDispatch(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub type WebResult<T> = Result<T, WebError>;
