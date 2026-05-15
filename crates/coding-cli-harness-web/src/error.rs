//! HTTP-shaped errors for the web layer.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use atomr_agents_coding_cli_harness::HarnessError;

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Harness(#[from] HarnessError),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            WebError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            WebError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            WebError::Harness(h) => match h {
                HarnessError::UnknownVendor(_) => (StatusCode::BAD_REQUEST, h.to_string()),
                HarnessError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, h.to_string()),
                HarnessError::InvalidWorkdir(_) => (StatusCode::BAD_REQUEST, h.to_string()),
                HarnessError::SessionNotFound(_) => (StatusCode::NOT_FOUND, h.to_string()),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, h.to_string()),
            },
            WebError::Serde(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}
