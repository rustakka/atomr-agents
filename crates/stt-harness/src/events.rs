//! The STT-domain event stream.
//!
//! Two layers of observability, matching the rest of the framework:
//!
//! - Structured telemetry — the harness emits
//!   [`atomr_agents_core::Event::HarnessIteration`] to the shared
//!   `EventBus` (done in [`crate::harness`]), so STT runs appear in the
//!   run tree.
//! - The domain stream — [`SttHarnessEvent`], a rich per-event surface
//!   that UIs subscribe to. Backed by a `tokio::broadcast` channel so a
//!   logger and the web UI can both listen.

use atomr_agents_stt_core::SpeakerTag;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::conversation::SttTurn;

/// A single STT-pipeline event. Serializes with an internal `kind` tag
/// for transport to the web UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SttHarnessEvent {
    /// The streaming session opened.
    Started { backend: String, diarization: String },
    /// An in-progress (non-final) transcript.
    Partial {
        text: String,
        start_ms: u32,
        end_ms: u32,
    },
    /// A committed utterance turn.
    UtteranceCommitted { turn: SttTurn },
    /// The backend reported a speaker change.
    SpeakerChanged { speaker: SpeakerTag, at_ms: u32 },
    /// The backend (or VAD) reported end-of-utterance.
    UtteranceEnd { at_ms: u32 },
    /// Backend-specific metadata blob.
    Metadata { data: serde_json::Value },
    /// The configured diarization policy does not match the backend's
    /// capabilities. The run continues.
    DiarizationWarning { detail: String },
    /// The harness loop finished.
    Finished {
        reason: String,
        turn_count: usize,
        total_audio_secs: f32,
    },
    /// A fatal error ended the run.
    Error { detail: String },
}

/// Subscriber handle for [`SttHarnessEvent`]s. Obtain one from
/// [`crate::SttHarness::events`] (or [`crate::BoxedSttHarness::events`])
/// *before* calling `run()` so no events are missed.
pub struct SttEventStream {
    rx: broadcast::Receiver<SttHarnessEvent>,
}

impl SttEventStream {
    pub(crate) fn new(rx: broadcast::Receiver<SttHarnessEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event. Returns `None` once the harness has
    /// finished and the channel is closed. Lagged events (slow
    /// consumer) are skipped rather than erroring.
    pub async fn recv(&mut self) -> Option<SttHarnessEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Internal write side of the event stream. The harness loop pushes
/// every domain event here; cloning is cheap.
#[derive(Clone)]
pub(crate) struct SttEventSink {
    tx: broadcast::Sender<SttHarnessEvent>,
}

impl SttEventSink {
    pub(crate) fn new(tx: broadcast::Sender<SttHarnessEvent>) -> Self {
        Self { tx }
    }

    /// Fan an event out to all current subscribers. A send with no
    /// subscribers is a no-op (not an error).
    pub(crate) fn emit(&self, event: SttHarnessEvent) {
        let _ = self.tx.send(event);
    }
}
