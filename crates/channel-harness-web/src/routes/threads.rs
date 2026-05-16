//! `/api/channels/:id/threads` and `/api/threads/:id` — thread lifecycle.
//!
//! Opening a thread through REST is also restricted: the caller must
//! supply a concrete [`ThreadTarget`], which is a Rust object — see
//! `channels.rs` for the same rationale. We expose listing and message
//! access here; for admin send we accept a JSON body with a `text`
//! payload and route through `ChannelHarness::send`.

use atomr_agents_channel_core::{ChannelId, ChannelMessageRecord, MessageContent, ThreadId, ThreadSummary};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::AppState;

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ThreadSummary>>, StatusCode> {
    let v = state
        .harness
        .list_threads(&ChannelId::from(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(v))
}

pub async fn create() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(serde_json::json!({
            "error": "threads are opened via ChannelHarness::open_thread (target must be a Callable)",
        })),
    )
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ThreadInfo>, StatusCode> {
    let tid = ThreadId::from(id);
    let snapshot = state
        .harness
        .thread(&tid)
        .ok_or(StatusCode::NOT_FOUND)?
        .snapshot();
    Ok(Json(ThreadInfo {
        id: snapshot.id.as_str().to_string(),
        channel: snapshot.channel.as_str().to_string(),
        peer: snapshot.peer.as_str().to_string(),
        target_kind: snapshot.target.kind().to_string(),
        target_label: snapshot.target.label().to_string(),
        history_len: snapshot.history.len(),
    }))
}

pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .harness
        .close_thread(&ThreadId::from(id))
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ListMessagesQuery>,
) -> Result<Json<Vec<ChannelMessageRecord>>, StatusCode> {
    let recs = state
        .harness
        .list_messages(&ThreadId::from(id), q.limit.unwrap_or(0))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(recs))
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SendBody {
    Text { text: String },
    Content { content: MessageContent },
}

pub async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let content = match body {
        SendBody::Text { text } => MessageContent::text(text),
        SendBody::Content { content } => content,
    };
    let ack = state
        .harness
        .send(&ThreadId::from(id), content)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(serde_json::json!({
        "provider_msg_id": ack.provider_msg_id,
        "sent_at": ack.sent_at,
    })))
}

#[derive(Debug, serde::Serialize)]
pub struct ThreadInfo {
    pub id: String,
    pub channel: String,
    pub peer: String,
    pub target_kind: String,
    pub target_label: String,
    pub history_len: usize,
}
