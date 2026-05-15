//! The typed STT harness and the shared loop body.
//!
//! [`SttHarness`] keeps the hot path monomorphized over its strategy
//! generics; [`crate::BoxedSttHarness`] is the type-erased twin. Both
//! funnel into the free function [`run_impl`], exactly mirroring the
//! `Harness` / `BoxedHarness` / `run_impl` split in
//! `atomr_agents_harness`.

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Event, Result as CoreResult, RunId, Value};
use atomr_agents_observability::EventBus;
use atomr_agents_stt_core::DynSpeechToText;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::audio_source::AudioSource;
use crate::boxed::BoxedSttHarness;
use crate::conversation::SttConversation;
use crate::diarize::DiarizationStage;
use crate::dispatch::SttHarnessDispatch;
use crate::error::{Result, SttHarnessError};
use crate::events::{SttEventSink, SttEventStream, SttHarnessEvent};
use crate::loop_strategy::{SttLoopStrategy, SttStepCtx, SttStepOutcome};
use crate::session_actor::{spawn_session, SessionHandle};
use crate::spec::SttHarnessSpec;
use crate::state::{SttHarnessState, SttStepEvent};
use crate::termination::{SttTermination, Termination};

/// Broadcast channel capacity for the [`SttHarnessEvent`] stream.
const EVENT_CHANNEL_CAPACITY: usize = 512;

/// A typed STT harness. Generic over the loop and termination
/// strategies so the run path is fully monomorphized.
pub struct SttHarness<L, T>
where
    L: SttLoopStrategy,
    T: SttTermination,
{
    pub spec: SttHarnessSpec,
    pub backend: DynSpeechToText,
    /// Bound at construction, taken once by `run()`. A second `run()`
    /// fails with a clear configuration error.
    audio: Mutex<Option<AudioSource>>,
    pub loop_strategy: L,
    pub termination: T,
    pub bus: EventBus,
    event_tx: broadcast::Sender<SttHarnessEvent>,
}

impl<L, T> SttHarness<L, T>
where
    L: SttLoopStrategy,
    T: SttTermination,
{
    /// Assemble a harness from a spec, a backend, an audio source, and
    /// the two strategies.
    pub fn new(
        spec: SttHarnessSpec,
        backend: DynSpeechToText,
        audio: AudioSource,
        loop_strategy: L,
        termination: T,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            spec,
            backend,
            audio: Mutex::new(Some(audio)),
            loop_strategy,
            termination,
            bus: EventBus::new(),
            event_tx,
        }
    }

    /// Subscribe to the STT-domain event stream. Call this *before*
    /// `run()` so no events are missed.
    pub fn events(&self) -> SttEventStream {
        SttEventStream::new(self.event_tx.subscribe())
    }

    /// Drive the pipeline to completion, returning the accumulated
    /// conversation.
    pub async fn run(&self) -> Result<SttConversation> {
        let audio =
            self.audio.lock().take().ok_or_else(|| {
                SttHarnessError::config("audio source already consumed by a previous run()")
            })?;
        let sink = SttEventSink::new(self.event_tx.clone());
        run_impl(
            &self.spec,
            &self.backend,
            audio,
            &self.loop_strategy,
            &self.termination,
            &self.bus,
            &sink,
        )
        .await
    }

    /// Erase the strategy generics into a [`BoxedSttHarness`].
    pub fn into_boxed(self) -> BoxedSttHarness {
        BoxedSttHarness {
            spec: self.spec,
            backend: self.backend,
            audio: self.audio,
            loop_strategy: Box::new(self.loop_strategy),
            termination: Box::new(self.termination),
            bus: self.bus,
            event_tx: self.event_tx,
        }
    }
}

#[async_trait]
impl<L, T> Callable for SttHarness<L, T>
where
    L: SttLoopStrategy,
    T: SttTermination,
{
    async fn call(&self, _input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let conv = self.run().await?;
        Ok(serde_json::to_value(conv).map_err(SttHarnessError::from)?)
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}

#[async_trait]
impl<L, T> SttHarnessDispatch for SttHarness<L, T>
where
    L: SttLoopStrategy,
    T: SttTermination,
{
    async fn dispatch(&self) -> CoreResult<Value> {
        let conv = self.run().await?;
        Ok(serde_json::to_value(conv).map_err(SttHarnessError::from)?)
    }
}

/// Shared loop body for both the typed and boxed harnesses.
///
/// Opens a streaming session, hands it (with the audio pump) to the
/// [session task](crate::session_actor), then loops:
/// termination check → `loop_strategy.step` → emit `HarnessIteration`.
/// Stream-end arrives as [`SttStepOutcome::Done`]. On exit the session
/// task is stopped and awaited.
pub(crate) async fn run_impl(
    spec: &SttHarnessSpec,
    backend: &DynSpeechToText,
    audio: AudioSource,
    loop_strategy: &dyn SttLoopStrategy,
    termination: &dyn SttTermination,
    bus: &EventBus,
    sink: &SttEventSink,
) -> Result<SttConversation> {
    let run_id = RunId::new();
    let conversation_id = format!("{}:{}", spec.id.as_str(), run_id.as_str());
    let mut state = SttHarnessState::new(conversation_id, spec.initial_budget.remaining);
    state.conversation.backend = Some(backend.backend_kind());

    // --- Setup ----------------------------------------------------------
    let pump = audio.into_pump()?;
    let mut stream_opts = spec.config.stream_options.clone();
    if stream_opts.format.is_none() {
        stream_opts.format = Some(pump.format());
    }
    let session = backend.open_stream(stream_opts).await?;
    let backend_caps_diar = session.capabilities().diarization;

    let mut diarize = DiarizationStage::new(spec.config.diarization.clone(), backend_caps_diar);
    diarize.warn_mismatch(sink);
    let want_pcm = diarize.wants_pcm();

    let SessionHandle {
        mut event_rx,
        pcm_rx,
        stop,
        join,
    } = spawn_session(session, pump, want_pcm);
    diarize.attach_pcm(pcm_rx);

    sink.emit(SttHarnessEvent::Started {
        backend: backend.backend_kind().as_str().to_string(),
        diarization: diarize.describe().to_string(),
    });
    emit_iteration(bus, spec, &run_id, 0, "stt_open", state.remaining_budget);

    // --- Main loop ------------------------------------------------------
    let final_reason: &'static str = loop {
        if let Termination::Done(reason) = termination.should_terminate(&state) {
            emit_iteration(
                bus,
                spec,
                &run_id,
                state.iteration,
                &format!("terminated:{reason}"),
                state.remaining_budget,
            );
            break reason;
        }
        state.iteration += 1;

        let outcome = {
            let mut ctx = SttStepCtx::new(&mut state, &mut event_rx, &mut diarize, sink);
            loop_strategy.step(&mut ctx).await?
        };

        match outcome {
            SttStepOutcome::Continue { label } => {
                push_step(&mut state, label.clone());
                emit_iteration(
                    bus,
                    spec,
                    &run_id,
                    state.iteration,
                    &label,
                    state.remaining_budget,
                );
            }
            SttStepOutcome::Done { label } => {
                state.stream_closed = true;
                push_step(&mut state, label.clone());
                emit_iteration(
                    bus,
                    spec,
                    &run_id,
                    state.iteration,
                    &format!("done:{label}"),
                    state.remaining_budget,
                );
                break "stream_end";
            }
        }
    };

    // --- Teardown -------------------------------------------------------
    stop.store(true, Ordering::Relaxed);
    drop(event_rx); // signal the session task that nobody is listening
    let _ = join.await;

    sink.emit(SttHarnessEvent::Finished {
        reason: final_reason.to_string(),
        turn_count: state.conversation.turns.len(),
        total_audio_secs: state.conversation.total_audio_secs,
    });

    Ok(state.conversation)
}

fn push_step(state: &mut SttHarnessState, outcome: String) {
    state.history.push(SttStepEvent {
        iteration: state.iteration,
        outcome,
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
    });
}

fn emit_iteration(
    bus: &EventBus,
    spec: &SttHarnessSpec,
    run_id: &RunId,
    iteration: u64,
    outcome: &str,
    budget: u32,
) {
    bus.emit_run(
        Event::HarnessIteration {
            harness_id: spec.id.clone(),
            iteration,
            outcome: outcome.to_string(),
            budget_remaining_tokens: budget,
        },
        run_id.clone(),
        None,
    );
}
