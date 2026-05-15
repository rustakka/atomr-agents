//! In-process run supervisor for the meetings web layer.
//!
//! Holds at most one in-flight run; a `POST /api/meetings/:id/stop`
//! cancels via the boxed harness's cooperative signal. The runner does
//! not own the harness configuration — handlers build a
//! [`BoxedMeetingsHarness`] and hand it in, letting the route choose
//! between batch and live modes.

use std::sync::Arc;

use atomr_agents_meetings_harness::BoxedMeetingsHarness;
use tokio::task::JoinHandle;

#[derive(Default)]
pub struct RunSupervisor {
    /// The active harness, if any. Cloned out under the supervisor
    /// lock so a `stop` route can signal it without holding the lock
    /// across the await.
    pub active: Option<Arc<BoxedMeetingsHarness>>,
    /// JoinHandle for the spawned run task. Dropped when a new run
    /// starts; the previous task is left to finish naturally (with
    /// `cancel()` having been signalled first).
    pub task: Option<JoinHandle<()>>,
}

impl RunSupervisor {
    /// Register a freshly-spawned run.
    pub fn install(&mut self, harness: Arc<BoxedMeetingsHarness>, task: JoinHandle<()>) {
        // If there's a prior in-flight run, request cancellation; we do
        // not await it here.
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
