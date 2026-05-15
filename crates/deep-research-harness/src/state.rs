//! Mutable per-run loop state for the deep-research harness.

use atomr_agents_deep_research_core::ResearchResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepResearchStepEvent {
    pub iteration: u64,
    pub outcome: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DeepResearchState {
    /// 1-based iteration counter.
    pub iteration: u64,
    /// Per-iteration outcome log.
    pub history: Vec<DeepResearchStepEvent>,
    /// Mirror of the working result (also held inside the
    /// [`crate::ResearchHandle`]).
    pub result: ResearchResult,
    /// Cooperative cancel flag.
    pub cancel_requested: bool,
    /// Token-shaped budget proxy.
    pub remaining_budget: u32,
    /// Wall-clock millis elapsed since the run started.
    pub elapsed_ms: u64,
}

impl DeepResearchState {
    pub fn new(result: ResearchResult, initial_budget: u32) -> Self {
        Self {
            iteration: 0,
            history: Vec::new(),
            result,
            cancel_requested: false,
            remaining_budget: initial_budget,
            elapsed_ms: 0,
        }
    }
}
