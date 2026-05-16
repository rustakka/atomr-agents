//! `/api/channels` — channel listing and inspection.
//!
//! Provider attachment requires a concrete `ChannelProvider` impl, which
//! is supplied by the embedding application (not constructed from JSON
//! here). This endpoint surfaces the **persisted** channel state. To
//! attach a provider with config the caller does so directly through
//! `ChannelHarness::attach_provider`; the resulting `ChannelSpec`
//! shows up here automatically.

use atomr_agents_channel_core::{ChannelId, ChannelSpec};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::AppState;

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ChannelSpec>>, StatusCode> {
    let specs = state
        .harness
        .list_channels()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(specs))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChannelSpec>, StatusCode> {
    let spec = state
        .harness
        .get_channel(&ChannelId::from(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(spec))
}

/// Creating a channel through the REST surface is a no-op for now: the
/// caller must invoke `ChannelHarness::attach_provider` directly with a
/// concrete provider. We return 405 to make that explicit.
pub async fn create() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(serde_json::json!({
            "error": "channels are attached via ChannelHarness::attach_provider",
        })),
    )
}

pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .harness
        .detach_provider(&ChannelId::from(id))
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}
