//! The harness loop strategy — one iteration consumes a burst of
//! stream events and folds them into the conversation.
//!
//! Mirrors `atomr_agents_harness::LoopStrategy`, but STT-shaped: a step
//! reads from the forwarded `StreamEvent` channel (not an opaque
//! state), folds partials/finals/markers into the
//! [`SttConversation`](crate::conversation::SttConversation), runs
//! diarization on finals, and emits domain events. Stream-end is
//! signalled by returning [`SttStepOutcome::Done`].

use async_trait::async_trait;
use atomr_agents_stt_core::{Segment, StreamEvent, SttError};
use atomr_agents_stt_voice::VoiceMode;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::conversation::SttConversation;
use crate::diarize::DiarizationStage;
use crate::error::Result;
use crate::events::{SttEventSink, SttHarnessEvent};
use crate::state::SttHarnessState;

/// Outcome of one loop iteration.
#[derive(Debug, Clone)]
pub enum SttStepOutcome {
    /// More audio may follow — keep looping. `label` is telemetry.
    Continue { label: String },
    /// The stream has ended — stop. `label` is telemetry.
    Done { label: String },
}

/// Everything one [`SttLoopStrategy::step`] needs. Bundled into a
/// struct because a step touches the loop state, the forwarded event
/// channel, the diarization stage, and the event sink. The fields are
/// private; strategies operate through the methods below.
pub struct SttStepCtx<'a> {
    state: &'a mut SttHarnessState,
    events: &'a mut UnboundedReceiver<std::result::Result<StreamEvent, SttError>>,
    diarize: &'a mut DiarizationStage,
    sink: &'a SttEventSink,
}

impl<'a> SttStepCtx<'a> {
    pub(crate) fn new(
        state: &'a mut SttHarnessState,
        events: &'a mut UnboundedReceiver<std::result::Result<StreamEvent, SttError>>,
        diarize: &'a mut DiarizationStage,
        sink: &'a SttEventSink,
    ) -> Self {
        Self {
            state,
            events,
            diarize,
            sink,
        }
    }

    /// Mutable access to the conversation being accumulated.
    pub fn conversation(&mut self) -> &mut SttConversation {
        &mut self.state.conversation
    }

    /// Read access to the full loop state.
    pub fn state(&self) -> &SttHarnessState {
        self.state
    }

    /// Await the next forwarded stream event. `None` once the upstream
    /// session has closed and drained.
    pub async fn next_event(&mut self) -> Option<std::result::Result<StreamEvent, SttError>> {
        self.events.recv().await
    }

    /// Take an event that is immediately ready, without awaiting.
    pub fn try_next_event(&mut self) -> Option<std::result::Result<StreamEvent, SttError>> {
        self.events.try_recv().ok()
    }

    /// Resolve the speaker for a freshly-final segment per the
    /// configured diarization policy.
    pub async fn resolve_diarization(&mut self, seg: &mut Segment) -> Result<()> {
        self.diarize.resolve_segment(seg).await
    }

    /// Emit an STT-domain event to subscribers.
    pub fn emit(&self, event: SttHarnessEvent) {
        self.sink.emit(event);
    }
}

/// Strategy that drives one loop iteration.
#[async_trait]
pub trait SttLoopStrategy: Send + Sync + 'static {
    async fn step(&self, ctx: &mut SttStepCtx<'_>) -> Result<SttStepOutcome>;
}

#[async_trait]
impl SttLoopStrategy for Box<dyn SttLoopStrategy> {
    async fn step(&self, ctx: &mut SttStepCtx<'_>) -> Result<SttStepOutcome> {
        (**self).step(ctx).await
    }
}

/// The default loop strategy: await the next event, greedily drain any
/// further events that are immediately ready, fold the whole burst
/// into the conversation. One iteration ≈ one burst, which keeps the
/// termination check frequent without being per-single-event.
///
/// [`VoiceMode::Live`] surfaces partials as [`SttHarnessEvent::Partial`];
/// [`VoiceMode::TurnBased`] buffers them silently until a `Final` or an
/// `UtteranceEnd` commits a turn.
pub struct StreamingLoop {
    pub coalesce: VoiceMode,
}

impl Default for StreamingLoop {
    fn default() -> Self {
        Self {
            coalesce: VoiceMode::default(),
        }
    }
}

impl StreamingLoop {
    pub fn new(coalesce: VoiceMode) -> Self {
        Self { coalesce }
    }
}

#[async_trait]
impl SttLoopStrategy for StreamingLoop {
    async fn step(&self, ctx: &mut SttStepCtx<'_>) -> Result<SttStepOutcome> {
        // Block for the next event; a closed channel means the stream
        // is done.
        let first = match ctx.next_event().await {
            None => {
                return Ok(SttStepOutcome::Done {
                    label: "stream_closed".into(),
                })
            }
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(ev)) => ev,
        };

        // Greedily collect the rest of the ready burst.
        let mut batch = vec![first];
        while let Some(slot) = ctx.try_next_event() {
            match slot {
                Ok(ev) => batch.push(ev),
                Err(e) => return Err(e.into()),
            }
        }

        let mut committed = 0u32;
        for ev in batch {
            match ev {
                StreamEvent::Partial {
                    text,
                    start_ms,
                    end_ms,
                    words,
                } => {
                    ctx.conversation()
                        .apply_partial(text.clone(), start_ms, end_ms, words);
                    if matches!(self.coalesce, VoiceMode::Live) {
                        ctx.emit(SttHarnessEvent::Partial {
                            text,
                            start_ms,
                            end_ms,
                        });
                    }
                }
                StreamEvent::Final { mut segment } => {
                    ctx.resolve_diarization(&mut segment).await?;
                    let turn = ctx.conversation().commit_segment(segment);
                    committed += 1;
                    ctx.emit(SttHarnessEvent::UtteranceCommitted { turn });
                }
                StreamEvent::SpeakerTurn { speaker, at_ms } => {
                    ctx.conversation().note_speaker_change(speaker.clone(), at_ms);
                    ctx.emit(SttHarnessEvent::SpeakerChanged { speaker, at_ms });
                }
                StreamEvent::UtteranceEnd { at_ms } => {
                    if let Some(turn) = ctx.conversation().close_open_turn(at_ms) {
                        committed += 1;
                        ctx.emit(SttHarnessEvent::UtteranceCommitted { turn });
                    }
                    ctx.emit(SttHarnessEvent::UtteranceEnd { at_ms });
                }
                StreamEvent::Metadata(data) => {
                    ctx.emit(SttHarnessEvent::Metadata { data });
                }
            }
        }

        let label = if committed > 0 {
            format!("committed:{committed}")
        } else {
            "partials".into()
        };
        Ok(SttStepOutcome::Continue { label })
    }
}
