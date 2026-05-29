//! Structured, per-step run telemetry (FR-19).
//!
//! [`RunEvent`] is the single external backbone that the determinism /
//! visibility layer consumes. Unlike the process-local [`crate::EventBus`]
//! `Event` taxonomy (which is oriented at in-process tracing), every
//! `RunEvent` carries a [`CheckpointRef`] back to the exact
//! `(workflow, run, super_step)` record so an external projector can
//! reconstruct what a live agent did — without re-inference.
//!
//! The model-drift (FR-3), interrupt-registry (FR-4), cost (FR-18), and
//! host-trigger (FR-12) features publish onto this backbone rather than
//! inventing their own event streams.

use serde::{Deserialize, Serialize};

/// Pointer back to a persisted checkpoint. Mirrors the state crate's
/// `CheckpointKey` without depending on it, so an external projector can
/// reconstruct the key and call `Checkpointer::load`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRef {
    pub workflow: String,
    pub run: String,
    pub super_step: u64,
}

impl CheckpointRef {
    pub fn new(workflow: impl Into<String>, run: impl Into<String>, super_step: u64) -> Self {
        Self {
            workflow: workflow.into(),
            run: run.into(),
            super_step,
        }
    }
}

/// The resolved model pin stamped onto a step (FR-3). Strings only, so
/// the telemetry layer does not depend on the agent crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPinRef {
    pub provider: String,
    pub model_id: String,
    pub model_version: String,
    pub params_hash: String,
}

/// Token I/O for an inference step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenIo {
    pub input: u32,
    pub output: u32,
    pub reasoning: u32,
    pub cached: u32,
}

/// What happened at a step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEventKind {
    CheckpointCreated,
    ToolDispatched {
        tool: String,
    },
    ToolReturned {
        tool: String,
        ok: bool,
    },
    InferenceCompleted,
    InterruptRaised {
        interrupt_id: String,
    },
    InterruptResolved {
        interrupt_id: String,
    },
    ModelDrift {
        expected: String,
        actual: String,
        drift: String,
    },
    BudgetSpent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_key: Option<String>,
        micro_usd: u64,
    },
    /// A host trigger fired or a control action was applied (FR-12).
    Trigger {
        trigger_id: String,
        action: String,
    },
}

/// A single externally-observable run event with a checkpoint pointer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    pub kind: RunEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<CheckpointRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_pin: Option<ModelPinRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenIo>,
    pub ts: i64,
}

impl RunEvent {
    /// Construct a `RunEvent` of `kind` stamped with the current time.
    pub fn new(kind: RunEventKind) -> Self {
        Self {
            kind,
            workflow: None,
            run: None,
            step: None,
            checkpoint_ref: None,
            model_pin: None,
            tokens: None,
            ts: chrono_now_ms(),
        }
    }

    pub fn with_run_ids(mut self, workflow: impl Into<String>, run: impl Into<String>, step: u64) -> Self {
        self.workflow = Some(workflow.into());
        self.run = Some(run.into());
        self.step = Some(step);
        self
    }

    pub fn with_checkpoint(mut self, cp: CheckpointRef) -> Self {
        // Keep the flat run identifiers consistent with the pointer.
        self.workflow = Some(cp.workflow.clone());
        self.run = Some(cp.run.clone());
        self.step = Some(cp.super_step);
        self.checkpoint_ref = Some(cp);
        self
    }

    pub fn with_model_pin(mut self, pin: ModelPinRef) -> Self {
        self.model_pin = Some(pin);
        self
    }

    pub fn with_tokens(mut self, tokens: TokenIo) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

fn chrono_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
