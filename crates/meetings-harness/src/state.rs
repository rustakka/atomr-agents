//! Mutable per-run loop state.
//!
//! Analogous to [`atomr_agents_stt_harness::SttHarnessState`] — but the
//! working memory is a [`MeetingAnalysis`], plus a snapshot of the
//! source transcript pulled from the configured store.

use atomr_agents_stt_harness::SttConversation;
use serde::{Deserialize, Serialize};

use crate::analysis::MeetingAnalysis;

/// One iteration's outcome, recorded for the run history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingsStepEvent {
    pub iteration: u64,
    pub outcome: String,
    pub timestamp_ms: i64,
}

/// State threaded through the meetings harness loop.
#[derive(Debug, Clone)]
pub struct MeetingsHarnessState {
    /// 1-based iteration counter.
    pub iteration: u64,
    /// Per-iteration outcome log.
    pub history: Vec<MeetingsStepEvent>,
    /// The analysis accumulated so far — the harness's "working
    /// memory" and the value `run()` ultimately returns.
    pub analysis: MeetingAnalysis,
    /// Snapshot of the source transcript. In batch mode the snapshot
    /// is loaded once. In live mode the loop replaces it with a fresh
    /// snapshot at each iteration so new turns become visible.
    pub transcript: SttConversation,
    /// Token-shaped budget proxy carried from the spec.
    pub remaining_budget: u32,
    /// Cooperative stop signal honoured by streaming loops.
    pub cancel_requested: bool,
    /// Set once the upstream STT stream is done (or batch input is
    /// fully consumed).
    pub stream_closed: bool,
}

impl MeetingsHarnessState {
    /// Fresh state for a run, with an empty analysis under `id` and a
    /// snapshot of the source transcript already loaded.
    pub fn new(transcript: SttConversation, initial_budget: u32) -> Self {
        let analysis = MeetingAnalysis::new(transcript.id.clone());
        Self {
            iteration: 0,
            history: Vec::new(),
            analysis,
            transcript,
            remaining_budget: initial_budget,
            cancel_requested: false,
            stream_closed: false,
        }
    }
}
