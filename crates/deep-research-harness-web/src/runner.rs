//! In-process run supervisor for the deep-research web layer.

use std::sync::Arc;

use atomr_agents_deep_research_harness::BoxedDeepResearchHarness;
use tokio::task::JoinHandle;

#[derive(Default)]
pub struct RunSupervisor {
    pub active: Option<Arc<BoxedDeepResearchHarness>>,
    pub task: Option<JoinHandle<()>>,
}

impl RunSupervisor {
    /// Register a freshly-spawned run; the previous run (if any) is
    /// asked to cancel.
    pub fn install(&mut self, harness: Arc<BoxedDeepResearchHarness>, task: JoinHandle<()>) {
        if let Some(prev) = &self.active {
            prev.cancel();
        }
        self.active = Some(harness);
        self.task = Some(task);
    }

    /// Cancel the active run, if any.
    pub fn cancel(&self) {
        if let Some(h) = &self.active {
            h.cancel();
        }
    }
}
