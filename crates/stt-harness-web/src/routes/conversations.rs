//! `/api/conversations` — list, fetch, delete, and edit speaker labels.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use atomr_agents_stt_harness::{ConversationStore, ConversationSummary, SttConversation, SttHarnessError};

use crate::AppState;

/// Error type for the JSON API. Maps store failures and missing ids
/// to appropriate HTTP statuses.
#[derive(Debug)]
pub enum ApiError {
    NotFound,
    Internal(String),
}

impl From<SttHarnessError> for ApiError {
    fn from(e: SttHarnessError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "conversation not found".to_string()),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

/// `GET /api/conversations` — summary rows for every stored conversation.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    Ok(Json(state.store.list().await?))
}

/// `GET /api/conversations/:id` — the full conversation, transcript and
/// speaker labels included.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SttConversation>, ApiError> {
    state.store.get(&id).await?.map(Json).ok_or(ApiError::NotFound)
}

/// `GET /api/conversations/:id/transcript.json` — export alias for
/// [`get`]; kept distinct so the UI can offer a download link.
pub async fn transcript_json(
    state: State<AppState>,
    path: Path<String>,
) -> Result<Json<SttConversation>, ApiError> {
    get(state, path).await
}

/// `DELETE /api/conversations/:id` — remove a conversation.
pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Body for [`rename_speaker`].
#[derive(Debug, Deserialize)]
pub struct RenameSpeakerBody {
    /// The new display label, e.g. `"Alice"`.
    pub label: String,
}

/// `PUT /api/conversations/:id/speakers/:speaker_id` — rename a speaker.
/// The numeric id is unchanged; every turn by that speaker picks up the
/// new label. Returns the updated conversation.
pub async fn rename_speaker(
    State(state): State<AppState>,
    Path((id, speaker_id)): Path<(String, u8)>,
    Json(body): Json<RenameSpeakerBody>,
) -> Result<Json<SttConversation>, ApiError> {
    state
        .store
        .rename_speaker(&id, speaker_id, body.label)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
