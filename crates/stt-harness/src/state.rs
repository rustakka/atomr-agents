//! Mutable per-run loop state.
//!
//! Analogous to `atomr_agents_harness::HarnessState`, but the working
//! memory is a typed [`SttConversation`] rather than an opaque
//! `serde_json::Value` — the STT loop fully owns the conversation's
//! shape and (de)serialization.

use serde::{Deserialize, Serialize};

use crate::conversation::SttConversation;

/// One iteration's outcome, recorded for the run history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttStepEvent {
    pub iteration: u64,
    pub outcome: String,
    pub timestamp_ms: i64,
}

/// State threaded through the harness loop. Each iteration may fold
/// stream events into [`Self::conversation`] and inspect the running
/// totals to decide termination.
#[derive(Debug, Clone)]
pub struct SttHarnessState {
    /// 1-based iteration counter.
    pub iteration: u64,
    /// Per-iteration outcome log.
    pub history: Vec<SttStepEvent>,
    /// The conversation accumulated so far — the harness's "working
    /// memory" and the value `run()` ultimately returns.
    pub conversation: SttConversation,
    /// Token-shaped budget proxy carried from the spec. STT has no
    /// real token budget; [`crate::AudioSecsTermination`] is the
    /// natural cap. Decremented only by callers that choose to.
    pub remaining_budget: u32,
    /// Set once the upstream audio stream has closed.
    pub stream_closed: bool,
}

impl SttHarnessState {
    /// Fresh state for a run, with an empty conversation under `id`.
    pub fn new(conversation_id: impl Into<String>, initial_budget: u32) -> Self {
        Self {
            iteration: 0,
            history: Vec::new(),
            conversation: SttConversation::new(conversation_id),
            remaining_budget: initial_budget,
            stream_closed: false,
        }
    }
}
