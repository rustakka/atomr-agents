use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, HarnessId, RunId, ToolId, WorkflowId};
use crate::inference::FinishReason;

/// Structured event emitted by every observable boundary in the
/// framework. Fed to `atomr-telemetry`, used by traces, metrics, and
/// the eval-suite replay path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    StrategyResolved {
        strategy: String,
        agent_id: Option<AgentId>,
        elapsed_ms: u64,
        tokens_used: u32,
    },
    ToolInvoked {
        tool_id: ToolId,
        args_hash: u64,
        elapsed_ms: u64,
        ok: bool,
    },
    AgentTurn {
        agent_id: AgentId,
        input_tokens: u32,
        output_tokens: u32,
        finish_reason: Option<FinishReason>,
        elapsed_ms: u64,
    },
    WorkflowStep {
        workflow_id: WorkflowId,
        step_id: String,
        step_kind: String,
        elapsed_ms: u64,
        ok: bool,
    },
    HarnessIteration {
        harness_id: HarnessId,
        iteration: u64,
        outcome: String,
        budget_remaining_tokens: u32,
    },
    Backpressure {
        actor_path: String,
        queued: u32,
        dropped: u32,
    },
}

/// Tagged envelope around an event with timestamp + correlation id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub timestamp_ms: i64,
    pub correlation_id: Option<String>,
    /// LangSmith-style run identification. Optional so existing call
    /// sites still compile; tracers and the run-tree builder require
    /// these to be populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub event: Event,
}

impl EventEnvelope {
    pub fn now(event: Event) -> Self {
        Self {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            correlation_id: None,
            run_id: None,
            parent_run_id: None,
            tags: Vec::new(),
            event,
        }
    }

    pub fn with_run(mut self, run_id: RunId, parent: Option<RunId>) -> Self {
        self.run_id = Some(run_id);
        self.parent_run_id = parent;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}
