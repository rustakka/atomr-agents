//! `/api/meetings` — list, fetch, delete, edit, and trigger runs.

use std::sync::Arc;

use atomr_agents_meetings_harness::{
    ActionStatus, BatchExtractionLoop, BoxedMeetingsHarness, IterationCapTermination,
    MeetingAnalysis, MeetingsHarnessError, MeetingsHarnessSpec, MeetingsSummary,
    RuleBasedExtractor, RunMode,
};
use atomr_agents_stt_harness::ConversationStore as _;
use atomr_agents_stt_harness::ConversationSummary;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::ws::forward_events;
use crate::AppState;

/// JSON-API error.
#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl From<MeetingsHarnessError> for ApiError {
    fn from(e: MeetingsHarnessError) -> Self {
        match e {
            MeetingsHarnessError::TranscriptNotFound(_) => ApiError::NotFound,
            MeetingsHarnessError::Config(m) | MeetingsHarnessError::Tool(m) => ApiError::BadRequest(m),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

/// `GET /api/meetings` — summary rows.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<MeetingsSummary>>, ApiError> {
    Ok(Json(state.store.list().await?))
}

/// `GET /api/meetings/:id` — full analysis.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MeetingAnalysis>, ApiError> {
    state.store.get(&id).await?.map(Json).ok_or(ApiError::NotFound)
}

/// `DELETE /api/meetings/:id`.
pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct RenameAttendeeBody {
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub email: Option<String>,
}

/// `PUT /api/meetings/:id/attendees/:attendee_id`.
pub async fn rename_attendee(
    State(state): State<AppState>,
    Path((id, attendee_id)): Path<(String, String)>,
    Json(body): Json<RenameAttendeeBody>,
) -> Result<Json<MeetingAnalysis>, ApiError> {
    state
        .store
        .update_attendee(&id, &attendee_id, body.display_name, body.role, body.email)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct UpdateActionBody {
    pub status: Option<ActionStatus>,
    pub owner_attendee_id: Option<String>,
    pub due_iso: Option<String>,
}

/// `PATCH /api/meetings/:id/actions/:action_id`.
pub async fn update_action(
    State(state): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    Json(body): Json<UpdateActionBody>,
) -> Result<Json<MeetingAnalysis>, ApiError> {
    state
        .store
        .update_action(
            &id,
            &action_id,
            body.status,
            body.owner_attendee_id,
            body.due_iso,
        )
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct TriggerRunBody {
    /// `"batch"` or `"live"`.
    pub mode: String,
    /// Required: caller-supplied model id. Recorded on the analysis.
    pub model_id: String,
    /// Optional: cap on loop iterations. Defaults to 32.
    pub max_iterations: Option<u32>,
    /// Live-mode segment size. Defaults to 8.
    pub segment_turn_count: Option<u32>,
}

/// `POST /api/meetings/:id/run` — fire off a fresh harness run for the
/// transcript with this id. Returns immediately with the initial
/// (Pending) analysis; progress streams on `/ws`.
pub async fn trigger_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TriggerRunBody>,
) -> Result<Json<MeetingAnalysis>, ApiError> {
    let spec = MeetingsHarnessSpec::new("meetings-web", body.model_id)
        .with_max_iterations(body.max_iterations.unwrap_or(32));
    let extractor = Arc::new(RuleBasedExtractor::new());
    let termination = Box::new(IterationCapTermination::new(spec.config.max_iterations));

    // Validate the source transcript exists *before* we spawn anything
    // and synchronously persist a Pending analysis so the caller's
    // immediate GET sees it.
    if state
        .transcripts
        .get(&id)
        .await
        .map_err(MeetingsHarnessError::Stt)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let mut pending = MeetingAnalysis::new(id.clone());
    pending.model_id = Some(spec.model_id.clone());
    state.store.put(&pending).await?;

    let harness: Arc<BoxedMeetingsHarness> = match body.mode.as_str() {
        "batch" => {
            let spec = spec.with_mode(RunMode::Batch);
            Arc::new(BoxedMeetingsHarness::new(
                spec,
                state.transcripts.clone(),
                state.store.clone(),
                extractor,
                Box::new(BatchExtractionLoop::default()),
                termination,
            ))
        }
        "live" => {
            return Err(ApiError::BadRequest(
                "live mode requires an STT event channel; trigger via CLI for now".into(),
            ));
        }
        other => {
            return Err(ApiError::BadRequest(format!("unknown mode `{other}`")));
        }
    };

    // Forward harness events into the web broadcast channel.
    let stream = harness.events();
    let sink = state.events.clone();
    tokio::spawn(forward_events(stream, sink));

    // Spawn the run.
    let h = harness.clone();
    let cid = id.clone();
    let task = tokio::spawn(async move {
        if let Err(e) = h.run(&cid).await {
            tracing::error!(target: "meetings-web", error = %e, conversation = %cid, "meetings run failed");
        }
    });
    state.supervisor.lock().install(harness.clone(), task);

    // Return the freshly-persisted Pending analysis.
    Ok(Json(pending))
}

/// `POST /api/meetings/:id/stop` — request cancellation of the active run.
pub async fn stop_run(
    State(state): State<AppState>,
    Path(_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.supervisor.lock().cancel();
    Ok(StatusCode::ACCEPTED)
}

/// `GET /api/transcripts` — summary rows of available STT transcripts
/// the dashboard can run an analysis on.
pub async fn list_transcripts(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let rows = state
        .transcripts
        .list()
        .await
        .map_err(MeetingsHarnessError::Stt)?;
    Ok(Json(rows))
}
