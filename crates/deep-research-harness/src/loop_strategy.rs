//! The deep-research loop strategy trait — one impl per topology.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::Result;
use crate::events::DeepResearchEvent;
use crate::handle::ResearchHandle;
use crate::roles::{CitationVerifier, Clarifier, Critic, Planner, Researcher, Writer};
use crate::state::DeepResearchState;
use crate::store::ResearchStore;

/// One iteration's outcome.
#[derive(Debug, Clone)]
pub enum DeepResearchStepOutcome {
    Continue { label: String },
    Done { label: String },
}

/// Context bundle passed to one [`DeepResearchLoopStrategy::step`] call.
pub struct DeepResearchStepCtx<'a> {
    pub state: &'a mut DeepResearchState,
    pub handle: &'a ResearchHandle,
    pub store: Arc<dyn ResearchStore>,
    pub clarifier: &'a dyn Clarifier,
    pub planner: &'a dyn Planner,
    pub researcher: &'a dyn Researcher,
    pub writer: &'a dyn Writer,
    pub critic: &'a dyn Critic,
    pub verifier: &'a dyn CitationVerifier,
    pub events: &'a broadcast::Sender<DeepResearchEvent>,
}

/// Strategy that drives one iteration.
#[async_trait]
pub trait DeepResearchLoopStrategy: Send + Sync + 'static {
    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome>;

    /// Strategy id recorded on the resulting [`ResearchResult`].
    fn name(&self) -> &str;
}

#[async_trait]
impl DeepResearchLoopStrategy for Box<dyn DeepResearchLoopStrategy> {
    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
        (**self).step(ctx).await
    }
    fn name(&self) -> &str {
        (**self).name()
    }
}
