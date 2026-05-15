//! Domain event stream emitted by the meetings harness.
//!
//! Same shape as [`atomr_agents_stt_harness::SttHarnessEvent`]: an
//! internally-tagged enum that serializes cleanly to JSON for the web
//! UI, fanned out over a `tokio::broadcast` channel.

use serde::Serialize;
use tokio::sync::broadcast;

use crate::analysis::{Action, ActionStatus, Attendee, Note, SegmentSummary};

/// A single domain event for the meetings pipeline.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeetingsHarnessEvent {
    /// The harness started a run.
    Started {
        /// `"batch"` or `"live"`.
        mode: String,
        source_transcript_id: String,
    },
    /// An attendee was added or merged.
    AttendeeUpserted { attendee: Attendee },
    /// A new note appended to the linear ledger.
    NoteAppended { note: Note },
    /// A new action appended to the linear ledger.
    ActionAppended { action: Action },
    /// An existing action was patched (status / owner / due / quote).
    ActionUpdated {
        action_id: String,
        status: Option<ActionStatus>,
        owner_attendee_id: Option<String>,
        due_iso: Option<String>,
    },
    /// The in-flight tail segment summary was revised.
    SegmentRevised { segment: SegmentSummary },
    /// A segment summary was finalized; a new in-flight tail may open.
    SegmentFinalized { segment: SegmentSummary },
    /// The running rollup was regenerated.
    RunningSummaryUpdated { text: String },
    /// The meeting title was set or replaced.
    TitleSet { title: String },
    /// The watermark advanced (live mode).
    WatermarkAdvanced { turn_index: u64 },
    /// Progress heartbeat.
    Progress { processed: u64, total: u64 },
    /// The run terminated normally.
    Finalized {
        reason: String,
        note_count: usize,
        action_count: usize,
    },
    /// The run was stopped via a cancellation signal.
    Stopped { reason: String },
    /// A fatal error ended the run.
    Error { detail: String },
}

/// Subscriber handle for [`MeetingsHarnessEvent`]s.
pub struct MeetingsEventStream {
    rx: broadcast::Receiver<MeetingsHarnessEvent>,
}

impl MeetingsEventStream {
    pub(crate) fn new(rx: broadcast::Receiver<MeetingsHarnessEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event. `None` once the channel closes.
    pub async fn recv(&mut self) -> Option<MeetingsHarnessEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
