//! `/api/research` — list, fetch, delete, start, stop.

use std::sync::Arc;

use atomr_agents_deep_research_core::ResearchRequest;
use atomr_agents_deep_research_harness::{
    BoxedDeepResearchHarness, ClarifyPlanSearchVerifyLoop, DeepResearchError, DeepResearchHarnessSpec,
    DeepResearchLoopStrategy, DeepResearchRoles, IterationCapTermination, IterativeDeepeningLoop,
    MultiAgentParallelLoop, ResearchResult, ResearchStore, ResearchSummary,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl From<DeepResearchError> for ApiError {
    fn from(e: DeepResearchError) -> Self {
        match e {
            DeepResearchError::NotFound(_) => ApiError::NotFound,
            DeepResearchError::Config(m) | DeepResearchError::Tool(m) => ApiError::BadRequest(m),
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

/// `GET /api/research` — summary rows.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ResearchSummary>>, ApiError> {
    Ok(Json(state.store.list().await?))
}

/// `GET /api/research/:id` — full result.
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ResearchResult>, ApiError> {
    state.store.get(&id).await?.map(Json).ok_or(ApiError::NotFound)
}

/// `DELETE /api/research/:id`.
pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Body for `POST /api/research`.
#[derive(Debug, Deserialize)]
pub struct RunRequestBody {
    pub request: ResearchRequest,
    /// One of `"clarify-plan-search-verify"`, `"multi-agent-parallel"`,
    /// `"iterative-deepening"`. Defaults to AI-Q.
    #[serde(default)]
    pub strategy: Option<String>,
    /// Optional cap on harness iterations; defaults to 64.
    #[serde(default)]
    pub max_iterations: Option<u32>,
    /// Optional model id to record on the result (LLM-driven roles
    /// would honour this; the deterministic defaults ignore it).
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunResponse {
    pub id: String,
}

/// `POST /api/research` — start a run; returns the new id.
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<RunRequestBody>,
) -> Result<Json<RunResponse>, ApiError> {
    let strategy_name = body.strategy.as_deref().unwrap_or("clarify-plan-search-verify");
    let strategy: Box<dyn DeepResearchLoopStrategy> = match strategy_name {
        "clarify-plan-search-verify" => Box::new(ClarifyPlanSearchVerifyLoop::new()),
        "multi-agent-parallel" => Box::new(MultiAgentParallelLoop::new()),
        "iterative-deepening" => Box::new(IterativeDeepeningLoop::new()),
        other => return Err(ApiError::BadRequest(format!("unknown strategy `{other}`"))),
    };

    let max_iters = body.max_iterations.unwrap_or(64);
    let mut spec = DeepResearchHarnessSpec::new("deep-research-web").with_max_iterations(max_iters);
    if let Some(m) = body.model_id {
        spec = spec.with_model_id(m);
    }

    let harness = Arc::new(BoxedDeepResearchHarness::new(
        spec,
        state.store.clone(),
        state.search.clone(),
        DeepResearchRoles::defaults(),
        strategy,
        Box::new(IterationCapTermination::new(max_iters)),
    ));

    // Forward harness events into the web broadcast channel.
    let mut stream = harness.events();
    let sink = state.events.clone();
    tokio::spawn(async move {
        while let Some(ev) = stream.recv().await {
            let _ = sink.send(ev);
        }
    });

    let req = body.request;
    let h = harness.clone();
    // Pre-emit a stable id so the caller can subscribe immediately.
    // We persist a Pending row up front the same way `meetings-web`
    // does — but the harness already creates its own id so we kick the
    // run off and return that id once the run task starts.
    let (id_tx, id_rx) = tokio::sync::oneshot::channel::<String>();
    let task = tokio::spawn(async move {
        // We have to wait until run() persists the initial result.
        // Cheapest way: snoop on the store list before/after.
        let before: std::collections::HashSet<String> = h
            .store
            .list()
            .await
            .map(|rows| rows.into_iter().map(|r| r.id).collect())
            .unwrap_or_default();
        let _ = h.run(req).await;
        if let Ok(after) = h.store.list().await {
            for row in after {
                if !before.contains(&row.id) {
                    let _ = id_tx.send(row.id.clone());
                    break;
                }
            }
        }
    });
    state.supervisor.lock().install(harness, task);
    // Wait for the spawned task to publish the new id (it does this
    // immediately after the harness persists the initial Pending row).
    let id = id_rx
        .await
        .map_err(|_| ApiError::Internal("run completed without persisting an id".into()))?;
    Ok(Json(RunResponse { id }))
}

/// `POST /api/research/:id/stop` — cooperative cancel.
pub async fn stop_run(
    State(state): State<AppState>,
    Path(_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.supervisor.lock().cancel();
    Ok(StatusCode::ACCEPTED)
}

/// `GET /api/strategies` — list available strategy ids.
pub async fn list_strategies() -> Json<Vec<&'static str>> {
    Json(vec![
        "clarify-plan-search-verify",
        "multi-agent-parallel",
        "iterative-deepening",
    ])
}
