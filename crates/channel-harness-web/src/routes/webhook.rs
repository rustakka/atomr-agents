//! `POST /webhook/<provider>/<channel_id>` — verified webhook entry.
//!
//! The web layer is provider-agnostic: it forwards `(headers, body)`
//! into `ChannelHarness::ingest_webhook`, which delegates to the
//! attached provider for `verify_webhook` then `parse_webhook`.

use atomr_agents_channel_core::ChannelId;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use crate::AppState;

pub async fn receive(
    State(state): State<AppState>,
    Path((_provider, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match state
        .harness
        .ingest_webhook(&ChannelId::from(channel_id), &headers, &body)
        .await
    {
        Ok(n) => (StatusCode::OK, Json(serde_json::json!({ "accepted": n }))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
