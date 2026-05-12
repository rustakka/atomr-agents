use async_trait::async_trait;
use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

use crate::state::HarnessState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepOutcome {
    /// Continue the loop. The `working_memory` value is what the
    /// next iteration sees as input.
    Continue { working_memory: Value, label: String },
    /// Terminate immediately, returning this value as the harness
    /// result.
    Done { output: Value, label: String },
}

#[async_trait]
pub trait LoopStrategy: Send + Sync + 'static {
    async fn step(&self, state: &mut HarnessState) -> Result<StepOutcome>;
}

#[async_trait]
impl LoopStrategy for Box<dyn LoopStrategy> {
    async fn step(&self, state: &mut HarnessState) -> Result<StepOutcome> {
        (**self).step(state).await
    }
}
