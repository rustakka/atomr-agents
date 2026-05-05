use atomr_agents_core::{TokenBudget, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepEvent {
    pub iteration: u64,
    pub outcome: String,
    pub timestamp_ms: i64,
}

/// Mutable state shared between iterations of a harness loop. Used to
/// be `Box<dyn Any>` in the architecture doc; in v0 it's just a
/// `serde_json::Value` so persistence is trivial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessState {
    pub iteration: u64,
    pub history: Vec<StepEvent>,
    pub working_memory: Value,
    pub remaining_tokens: u32,
}

impl HarnessState {
    pub fn new(initial_budget: TokenBudget) -> Self {
        Self {
            iteration: 0,
            history: Vec::new(),
            working_memory: Value::Null,
            remaining_tokens: initial_budget.remaining,
        }
    }
}
