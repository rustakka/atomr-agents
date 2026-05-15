//! Type-erased meetings harness.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Result as CoreResult, RunId, Value};
use atomr_agents_observability::EventBus;
use atomr_agents_stt_harness::ConversationStore;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::analysis::{AnalysisState, MeetingAnalysis, RunMode};
use crate::dispatch::MeetingsHarnessDispatch;
use crate::error::{MeetingsHarnessError, Result};
use crate::events::{MeetingsEventStream, MeetingsHarnessEvent};
use crate::extractor::MeetingExtractor;
use crate::harness::{extract_conversation_id, run_loop};
use crate::loop_strategy::MeetingsLoopStrategy;
use crate::spec::MeetingsHarnessSpec;
use crate::state::MeetingsHarnessState;
use crate::store::MeetingsStore;
use crate::termination::MeetingsTermination;
use crate::tools::ToolHandle;

/// Type-erased meetings harness — strategy generics replaced by trait
/// objects so callers without compile-time access can construct one.
pub struct BoxedMeetingsHarness {
    pub spec: MeetingsHarnessSpec,
    pub transcript_store: Arc<dyn ConversationStore>,
    pub analysis_store: Arc<dyn MeetingsStore>,
    pub extractor: Arc<dyn MeetingExtractor>,
    pub loop_strategy: Box<dyn MeetingsLoopStrategy>,
    pub termination: Box<dyn MeetingsTermination>,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<MeetingsHarnessEvent>,
    pub(crate) cancel: Arc<parking_lot::Mutex<bool>>,
}

impl BoxedMeetingsHarness {
    pub fn new(
        spec: MeetingsHarnessSpec,
        transcript_store: Arc<dyn ConversationStore>,
        analysis_store: Arc<dyn MeetingsStore>,
        extractor: Arc<dyn MeetingExtractor>,
        loop_strategy: Box<dyn MeetingsLoopStrategy>,
        termination: Box<dyn MeetingsTermination>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(512);
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

    /// Subscribe to the domain event stream.
    pub fn events(&self) -> MeetingsEventStream {
        MeetingsEventStream::new(self.event_tx.subscribe())
    }

    /// Sender clone for forwarding into a web server.
    pub fn event_sender(&self) -> broadcast::Sender<MeetingsHarnessEvent> {
        self.event_tx.clone()
    }

    /// Request cooperative cancellation.
    pub fn cancel(&self) {
        *self.cancel.lock() = true;
    }

    /// Identical semantics to [`crate::MeetingsHarness::run`].
    pub async fn run(&self, source_conversation_id: &str) -> Result<MeetingAnalysis> {
        let conv = self
            .transcript_store
            .get(source_conversation_id)
            .await
            .map_err(MeetingsHarnessError::Stt)?
            .ok_or_else(|| MeetingsHarnessError::TranscriptNotFound(source_conversation_id.into()))?;

        let mut analysis = MeetingAnalysis::new(conv.id.clone());
        analysis.model_id = Some(self.spec.model_id.clone());
        self.analysis_store.put(&analysis).await?;

        let analysis_arc = Arc::new(Mutex::new(analysis));
        let transcript_arc = Arc::new(Mutex::new(conv.clone()));
        let handle =
            ToolHandle::new(analysis_arc.clone(), transcript_arc.clone()).with_events(self.event_tx.clone());

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
            &*self.loop_strategy,
            &*self.termination,
            self.extractor.as_ref(),
            self.analysis_store.clone(),
            &handle,
            &self.event_tx,
            self.cancel.clone(),
            &self.bus,
            &mut state,
        )
        .await?;

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
}

#[async_trait]
impl MeetingsHarnessDispatch for BoxedMeetingsHarness {
    async fn dispatch(&self, conversation_id: &str) -> CoreResult<Value> {
        let analysis = self.run(conversation_id).await?;
        Ok(serde_json::to_value(analysis).map_err(MeetingsHarnessError::from)?)
    }
}

#[async_trait]
impl Callable for BoxedMeetingsHarness {
    async fn call(&self, input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let id = extract_conversation_id(&input).map_err(MeetingsHarnessError::Config)?;
        let analysis = self.run(&id).await?;
        Ok(serde_json::to_value(analysis).map_err(MeetingsHarnessError::from)?)
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}
