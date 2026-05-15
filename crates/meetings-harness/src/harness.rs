//! The typed meetings harness and shared loop body.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Event, Result as CoreResult, RunId, Value};
use atomr_agents_observability::EventBus;
use atomr_agents_stt_harness::ConversationStore;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::analysis::{AnalysisState, MeetingAnalysis, RunMode};
use crate::boxed::BoxedMeetingsHarness;
use crate::dispatch::MeetingsHarnessDispatch;
use crate::error::{MeetingsHarnessError, Result};
use crate::events::{MeetingsEventStream, MeetingsHarnessEvent};
use crate::extractor::MeetingExtractor;
use crate::loop_strategy::{MeetingsLoopStrategy, MeetingsStepCtx, MeetingsStepOutcome};
use crate::spec::MeetingsHarnessSpec;
use crate::state::{MeetingsHarnessState, MeetingsStepEvent};
use crate::store::MeetingsStore;
use crate::termination::{MeetingsTermination, Termination};
use crate::tools::ToolHandle;

const EVENT_CHANNEL_CAPACITY: usize = 512;

/// A typed meetings harness.
pub struct MeetingsHarness<L, T>
where
    L: MeetingsLoopStrategy,
    T: MeetingsTermination,
{
    pub spec: MeetingsHarnessSpec,
    pub transcript_store: Arc<dyn ConversationStore>,
    pub analysis_store: Arc<dyn MeetingsStore>,
    pub extractor: Arc<dyn MeetingExtractor>,
    pub loop_strategy: L,
    pub termination: T,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<MeetingsHarnessEvent>,
    cancel: Arc<parking_lot::Mutex<bool>>,
}

impl<L, T> MeetingsHarness<L, T>
where
    L: MeetingsLoopStrategy,
    T: MeetingsTermination,
{
    /// Assemble a typed harness.
    pub fn new(
        spec: MeetingsHarnessSpec,
        transcript_store: Arc<dyn ConversationStore>,
        analysis_store: Arc<dyn MeetingsStore>,
        extractor: Arc<dyn MeetingExtractor>,
        loop_strategy: L,
        termination: T,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            spec,
            transcript_store,
            analysis_store,
            extractor,
            loop_strategy,
            termination,
            bus: EventBus::new(),
            event_tx,
            cancel: Arc::new(parking_lot::Mutex::new(false)),
        }
    }

    /// Subscribe to the domain event stream. Call before `run()`.
    pub fn events(&self) -> MeetingsEventStream {
        MeetingsEventStream::new(self.event_tx.subscribe())
    }

    /// Sender clone — for forwarding into a web server's broadcast.
    pub fn event_sender(&self) -> broadcast::Sender<MeetingsHarnessEvent> {
        self.event_tx.clone()
    }

    /// Request cooperative cancellation. The next loop iteration will
    /// see [`MeetingsHarnessState::cancel_requested`] = true.
    pub fn cancel(&self) {
        *self.cancel.lock() = true;
    }

    /// Run the harness against an existing transcript identified by
    /// `source_conversation_id`. The transcript is loaded from
    /// `transcript_store`; the resulting analysis is keyed by that same
    /// id and persisted into `analysis_store`.
    pub async fn run(&self, source_conversation_id: &str) -> Result<MeetingAnalysis> {
        // Load the transcript.
        let conv = self
            .transcript_store
            .get(source_conversation_id)
            .await
            .map_err(MeetingsHarnessError::Stt)?
            .ok_or_else(|| MeetingsHarnessError::TranscriptNotFound(source_conversation_id.into()))?;

        // Persist an initial Pending analysis so the web UI sees it.
        let mut analysis = MeetingAnalysis::new(conv.id.clone());
        analysis.model_id = Some(self.spec.model_id.clone());
        self.analysis_store.put(&analysis).await?;

        // Set up the shared tool handle.
        let analysis_arc = Arc::new(Mutex::new(analysis.clone()));
        let transcript_arc = Arc::new(Mutex::new(conv.clone()));
        let handle =
            ToolHandle::new(analysis_arc.clone(), transcript_arc.clone()).with_events(self.event_tx.clone());

        // Emit Started.
        let mode_label = match &self.spec.config.mode {
            RunMode::Batch => "batch",
            RunMode::Live { .. } => "live",
        };
        let _ = self.event_tx.send(MeetingsHarnessEvent::Started {
            mode: mode_label.into(),
            source_transcript_id: conv.id.clone(),
        });

        let mut state = MeetingsHarnessState::new(conv, self.spec.initial_budget.remaining);
        state.analysis = analysis_arc.lock().clone();

        let run_id = RunId::new();
        let final_reason = run_loop(
            &self.spec,
            &run_id,
            &self.loop_strategy as &dyn MeetingsLoopStrategy,
            &self.termination as &dyn MeetingsTermination,
            self.extractor.as_ref(),
            self.analysis_store.clone(),
            &handle,
            &self.event_tx,
            self.cancel.clone(),
            &self.bus,
            &mut state,
        )
        .await?;

        // Persist final state.
        let mut final_analysis = analysis_arc.lock().clone();
        if final_analysis.state != AnalysisState::Final {
            final_analysis.state = AnalysisState::Final;
        }
        final_analysis.touch();
        self.analysis_store.put(&final_analysis).await?;

        let _ = self.event_tx.send(MeetingsHarnessEvent::Finalized {
            reason: final_reason.to_string(),
            note_count: final_analysis.notes.len(),
            action_count: final_analysis.actions.len(),
        });

        Ok(final_analysis)
    }

    /// Erase the strategy generics.
    pub fn into_boxed(self) -> BoxedMeetingsHarness {
        BoxedMeetingsHarness {
            spec: self.spec,
            transcript_store: self.transcript_store,
            analysis_store: self.analysis_store,
            extractor: self.extractor,
            loop_strategy: Box::new(self.loop_strategy),
            termination: Box::new(self.termination),
            bus: self.bus,
            event_tx: self.event_tx,
            cancel: self.cancel,
        }
    }
}

#[async_trait]
impl<L, T> Callable for MeetingsHarness<L, T>
where
    L: MeetingsLoopStrategy,
    T: MeetingsTermination,
{
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        // The callable interface accepts the source conversation id as
        // either a bare string or `{ "conversation_id": "..." }`.
        let id = extract_conversation_id(&input).map_err(MeetingsHarnessError::Config)?;
        let analysis = self.run(&id).await?;
        Ok(serde_json::to_value(analysis).map_err(MeetingsHarnessError::from)?)
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}

#[async_trait]
impl<L, T> MeetingsHarnessDispatch for MeetingsHarness<L, T>
where
    L: MeetingsLoopStrategy,
    T: MeetingsTermination,
{
    async fn dispatch(&self, conversation_id: &str) -> CoreResult<Value> {
        let analysis = self.run(conversation_id).await?;
        Ok(serde_json::to_value(analysis).map_err(MeetingsHarnessError::from)?)
    }
}

pub(crate) fn extract_conversation_id(input: &Value) -> std::result::Result<String, String> {
    if let Some(s) = input.as_str() {
        return Ok(s.to_string());
    }
    if let Some(obj) = input.as_object() {
        if let Some(id) = obj.get("conversation_id").and_then(|v| v.as_str()) {
            return Ok(id.to_string());
        }
    }
    Err("call() requires a conversation id (string or { conversation_id: ... })".into())
}

/// Shared loop body used by both the typed and boxed harnesses.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_loop(
    spec: &MeetingsHarnessSpec,
    run_id: &RunId,
    loop_strategy: &dyn MeetingsLoopStrategy,
    termination: &dyn MeetingsTermination,
    extractor: &dyn MeetingExtractor,
    analysis_store: Arc<dyn MeetingsStore>,
    handle: &ToolHandle,
    events: &broadcast::Sender<MeetingsHarnessEvent>,
    cancel: Arc<parking_lot::Mutex<bool>>,
    bus: &EventBus,
    state: &mut MeetingsHarnessState,
) -> Result<&'static str> {
    let segment_turn_count = match &spec.config.mode {
        RunMode::Batch => 8,
        RunMode::Live { segment_turn_count } => (*segment_turn_count).max(1),
    };

    let final_reason: &'static str = loop {
        // Cancellation check.
        if *cancel.lock() {
            state.cancel_requested = true;
        }
        if let Termination::Done(reason) = termination.should_terminate(state) {
            emit_iteration(bus, spec, run_id, state.iteration, &format!("terminated:{reason}"));
            if reason == "cancelled" {
                let _ = events.send(MeetingsHarnessEvent::Stopped {
                    reason: reason.into(),
                });
            }
            break reason;
        }
        state.iteration += 1;

        let mut ctx = MeetingsStepCtx {
            state,
            handle,
            store: analysis_store.clone(),
            extractor,
            segment_turn_count,
            system_prompt: spec.config.system_prompt_override.clone(),
            events,
        };
        let outcome = loop_strategy.step(&mut ctx).await?;
        let label_owned = match &outcome {
            MeetingsStepOutcome::Continue { label } => label.clone(),
            MeetingsStepOutcome::Done { label } => label.clone(),
        };
        push_step(state, label_owned.clone());
        emit_iteration(bus, spec, run_id, state.iteration, &label_owned);

        if matches!(outcome, MeetingsStepOutcome::Done { .. }) {
            break "complete";
        }
    };

    Ok(final_reason)
}

fn push_step(state: &mut MeetingsHarnessState, outcome: String) {
    state.history.push(MeetingsStepEvent {
        iteration: state.iteration,
        outcome,
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
    });
}

fn emit_iteration(
    bus: &EventBus,
    spec: &MeetingsHarnessSpec,
    run_id: &RunId,
    iteration: u64,
    outcome: &str,
) {
    bus.emit_run(
        Event::HarnessIteration {
            harness_id: spec.id.clone(),
            iteration,
            outcome: outcome.to_string(),
            budget_remaining_tokens: 0,
        },
        run_id.clone(),
        None,
    );
}
