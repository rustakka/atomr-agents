//! The meetings harness loop strategy — one iteration drives the
//! extractor over a new window of turns and persists the result.
//!
//! Two implementations ship:
//!
//! - [`BatchExtractionLoop`] — reads the source transcript once, runs
//!   the extractor over the full content, finalizes, and returns.
//! - [`StreamingExtractionLoop`] — subscribes to the source STT
//!   harness's broadcast and processes turns incrementally. The
//!   in-flight tail segment summary is revised as new turns arrive;
//!   earlier segments are frozen; the notes/actions ledger only ever
//!   grows.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_harness::{SttConversation, SttHarnessEvent};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::analysis::AnalysisState;
use crate::error::Result;
use crate::events::MeetingsHarnessEvent;
use crate::extractor::{ExtractionRequest, ExtractionWindow, MeetingExtractor};
use crate::state::MeetingsHarnessState;
use crate::store::MeetingsStore;
use crate::tools::ToolHandle;

/// Outcome of one loop iteration.
#[derive(Debug, Clone)]
pub enum MeetingsStepOutcome {
    /// More work to do. `label` is telemetry.
    Continue { label: String },
    /// The extractor finalized; stop. `label` is telemetry.
    Done { label: String },
}

/// Context bundle handed to a [`MeetingsLoopStrategy::step`] call.
pub struct MeetingsStepCtx<'a> {
    pub state: &'a mut MeetingsHarnessState,
    pub handle: &'a ToolHandle,
    pub store: Arc<dyn MeetingsStore>,
    pub extractor: &'a dyn MeetingExtractor,
    pub segment_turn_count: u32,
    pub system_prompt: Option<String>,
    pub events: &'a broadcast::Sender<MeetingsHarnessEvent>,
}

/// Strategy that drives one iteration.
#[async_trait]
pub trait MeetingsLoopStrategy: Send + Sync + 'static {
    async fn step(&self, ctx: &mut MeetingsStepCtx<'_>) -> Result<MeetingsStepOutcome>;
}

#[async_trait]
impl MeetingsLoopStrategy for Box<dyn MeetingsLoopStrategy> {
    async fn step(&self, ctx: &mut MeetingsStepCtx<'_>) -> Result<MeetingsStepOutcome> {
        (**self).step(ctx).await
    }
}

/// Batch loop — one iteration runs the extractor over the entire
/// transcript with `finalize: true` and returns `Done`.
pub struct BatchExtractionLoop;

impl Default for BatchExtractionLoop {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl MeetingsLoopStrategy for BatchExtractionLoop {
    async fn step(&self, ctx: &mut MeetingsStepCtx<'_>) -> Result<MeetingsStepOutcome> {
        let request = ExtractionRequest {
            window: ExtractionWindow::all(),
            finalize: true,
            live: false,
            segment_turn_count: ctx.segment_turn_count,
            system_prompt: ctx.system_prompt.clone(),
        };
        ctx.state.analysis.state = AnalysisState::Streaming;
        ctx.extractor.extract(&request, ctx.handle).await?;
        ctx.state.analysis = ctx.handle.snapshot();
        ctx.store.put(&ctx.state.analysis).await?;
        Ok(MeetingsStepOutcome::Done {
            label: "batch_complete".into(),
        })
    }
}

/// Streaming loop — subscribes to the source STT harness's event
/// broadcast and processes newly-committed turns incrementally.
///
/// The strategy holds a `tokio::broadcast::Receiver` of [`SttHarnessEvent`]s
/// inside a `Mutex` because the `step` method takes `&self` and the
/// receiver requires `&mut`. The harness drives one event burst per
/// iteration: it awaits the first event, then drains everything ready,
/// reloads the transcript snapshot from the store, runs the extractor
/// over the new window, persists, and signals continue. On `Finished`
/// (or stream close) it signals done.
pub struct StreamingExtractionLoop {
    events: parking_lot::Mutex<Option<broadcast::Receiver<SttHarnessEvent>>>,
    store: Arc<dyn atomr_agents_stt_harness::ConversationStore>,
    source_conversation_id: String,
}

impl StreamingExtractionLoop {
    /// Build a streaming loop that watches the given STT event channel
    /// and re-loads the source transcript from `store` on each burst.
    pub fn new(
        events: broadcast::Receiver<SttHarnessEvent>,
        store: Arc<dyn atomr_agents_stt_harness::ConversationStore>,
        source_conversation_id: impl Into<String>,
    ) -> Self {
        Self {
            events: parking_lot::Mutex::new(Some(events)),
            store,
            source_conversation_id: source_conversation_id.into(),
        }
    }

    async fn next_event(&self) -> Option<std::result::Result<SttHarnessEvent, RecvError>> {
        // Pull the receiver out under the lock, then drop the guard
        // before await — parking_lot guards are not Send.
        let mut rx = self.events.lock().take()?;
        let result = rx.recv().await;
        *self.events.lock() = Some(rx);
        Some(result)
    }
}

#[async_trait]
impl MeetingsLoopStrategy for StreamingExtractionLoop {
    async fn step(&self, ctx: &mut MeetingsStepCtx<'_>) -> Result<MeetingsStepOutcome> {
        // Mark streaming on the first iteration.
        if ctx.state.iteration == 1 {
            ctx.state.analysis.state = AnalysisState::Streaming;
        }

        // Wait for the next event (or session close).
        let Some(first) = self.next_event().await else {
            return Ok(MeetingsStepOutcome::Done {
                label: "stream_closed".into(),
            });
        };
        let mut finalize = false;
        let mut last_label = "burst".to_string();
        match first {
            Ok(ev) => {
                if matches!(ev, SttHarnessEvent::Finished { .. }) {
                    finalize = true;
                    last_label = "finished".into();
                    ctx.state.stream_closed = true;
                }
            }
            Err(RecvError::Closed) => {
                return Ok(MeetingsStepOutcome::Done {
                    label: "stream_closed".into(),
                })
            }
            Err(RecvError::Lagged(_)) => {
                // Lagged: skip ahead and re-snapshot transcript below.
                last_label = "lagged".into();
            }
        }

        // Reload the transcript from the configured store so the new
        // turns become visible.
        let conv = match self
            .store
            .get(&self.source_conversation_id)
            .await
            .map_err(crate::error::MeetingsHarnessError::Stt)?
        {
            Some(c) => c,
            None => SttConversation::new(self.source_conversation_id.clone()),
        };
        ctx.state.transcript = conv.clone();
        ctx.handle.set_transcript(conv);

        // Run the extractor over the new window.
        let watermark = ctx.state.analysis.last_processed_turn_index;
        let request = ExtractionRequest {
            window: ExtractionWindow::after(watermark),
            finalize,
            live: true,
            segment_turn_count: ctx.segment_turn_count,
            system_prompt: ctx.system_prompt.clone(),
        };
        ctx.extractor.extract(&request, ctx.handle).await?;
        ctx.state.analysis = ctx.handle.snapshot();
        ctx.store.put(&ctx.state.analysis).await?;

        let processed = ctx.state.analysis.last_processed_turn_index.unwrap_or(0);
        let total = ctx.state.transcript.turns.len() as u64;
        let _ = ctx.events.send(MeetingsHarnessEvent::Progress {
            processed,
            total,
        });

        if finalize {
            Ok(MeetingsStepOutcome::Done {
                label: last_label,
            })
        } else {
            Ok(MeetingsStepOutcome::Continue { label: last_label })
        }
    }
}

