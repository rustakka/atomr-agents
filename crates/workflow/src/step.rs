use std::sync::Arc;

use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

use crate::dag::StepId;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum JoinStrategy {
    /// Wait for all parallel steps; succeed iff all succeed.
    All,
    /// Wait for the first to succeed; cancel the rest.
    Any,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Concurrency(pub u32);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputMapping {
    /// Field paths from the workflow input that get plumbed in.
    /// Empty list means "pass workflow input through unchanged".
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HumanApproval {
    pub prompt: String,
    /// Free-form metadata for the approval UI.
    #[serde(default)]
    pub context: Value,
}

/// Pure predicate over the workflow's running state. Used by `Branch`
/// and `Loop`.
pub trait BranchPredicate: Send + Sync + 'static {
    fn evaluate(&self, output: &Value) -> bool;
}

/// One step in a workflow's DAG.
pub enum Step {
    /// Invoke a `Callable` (tool, agent, or other workflow).
    Invoke {
        callable: CallableHandle,
        mapping: InputMapping,
    },
    /// Branch to one of two next steps based on `predicate(output)`.
    Branch {
        predicate: Arc<dyn BranchPredicate>,
        if_true: StepId,
        if_false: StepId,
    },
    /// Run several steps in parallel; aggregate via `JoinStrategy`.
    Parallel {
        steps: Vec<StepId>,
        join: JoinStrategy,
    },
    /// Loop a step while the predicate evaluates true.
    Loop {
        body: StepId,
        predicate: Arc<dyn BranchPredicate>,
    },
    /// Apply `body` once per element of an input array, with
    /// bounded concurrency.
    Map {
        body: StepId,
        concurrency: Concurrency,
    },
    /// Pause the workflow until a human approves. Persists the
    /// pending approval so a process restart resumes correctly.
    Human {
        approval: HumanApproval,
    },
}

impl Step {
    pub fn invoke(callable: CallableHandle) -> Self {
        Self::Invoke { callable, mapping: InputMapping::default() }
    }
}

// Convenience: a closure-based predicate.
pub struct FnPredicate<F: Fn(&Value) -> bool + Send + Sync + 'static>(pub F);

impl<F: Fn(&Value) -> bool + Send + Sync + 'static> BranchPredicate for FnPredicate<F> {
    fn evaluate(&self, output: &Value) -> bool {
        (self.0)(output)
    }
}

#[allow(dead_code)]
fn _result_unused() -> Result<()> {
    Ok(())
}
