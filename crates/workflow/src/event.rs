use std::sync::Arc;

use atomr_agents_core::{Result, Value, WorkflowId};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::dag::StepId;

/// Events appended to the workflow's journal. State is rebuilt by
/// folding these in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowEvent {
    StepStarted {
        step_id: StepId,
        idempotency_key: String,
    },
    StepCompleted {
        step_id: StepId,
        output: Value,
    },
    StepFailed {
        step_id: StepId,
        error: String,
    },
    BranchTaken {
        step_id: StepId,
        chosen: StepId,
    },
    HumanApproved {
        step_id: StepId,
        approver: String,
    },
    Terminated {
        ok: bool,
    },
}

/// Pluggable journal abstraction. Phase 6 ships the in-memory
/// implementation; production setups plug in a journal backed by
/// `atomr-persistence`.
#[async_trait::async_trait]
pub trait Journal: Send + Sync + 'static {
    async fn append(&self, workflow_id: &WorkflowId, event: WorkflowEvent) -> Result<()>;
    async fn replay(&self, workflow_id: &WorkflowId) -> Result<Vec<WorkflowEvent>>;
}

#[derive(Default, Clone)]
pub struct InMemoryJournal {
    inner: Arc<RwLock<Vec<(WorkflowId, WorkflowEvent)>>>,
}

impl InMemoryJournal {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Journal for InMemoryJournal {
    async fn append(&self, workflow_id: &WorkflowId, event: WorkflowEvent) -> Result<()> {
        self.inner.write().push((workflow_id.clone(), event));
        Ok(())
    }

    async fn replay(&self, workflow_id: &WorkflowId) -> Result<Vec<WorkflowEvent>> {
        Ok(self
            .inner
            .read()
            .iter()
            .filter(|(id, _)| id.as_str() == workflow_id.as_str())
            .map(|(_, e)| e.clone())
            .collect())
    }
}
