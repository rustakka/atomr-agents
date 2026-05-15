//! Fully type-erased STT harness.
//!
//! Holds `Box<dyn SttLoopStrategy>` / `Box<dyn SttTermination>` so
//! callers without compile-time access to the strategy generics
//! (Python loaders, registry wiring) can construct a runnable harness.
//! The body of `run` is shared with [`crate::SttHarness::run`] through
//! [`crate::harness::run_impl`].

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, Result as CoreResult, Value};
use atomr_agents_observability::EventBus;
use atomr_agents_stt_core::DynSpeechToText;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::audio_source::AudioSource;
use crate::conversation::SttConversation;
use crate::dispatch::SttHarnessDispatch;
use crate::error::{Result, SttHarnessError};
use crate::events::{SttEventSink, SttEventStream, SttHarnessEvent};
use crate::harness::run_impl;
use crate::loop_strategy::SttLoopStrategy;
use crate::spec::SttHarnessSpec;
use crate::termination::SttTermination;

/// An STT harness whose strategy generics have been erased into trait
/// objects. Constructed via [`SttHarnessSpec::into_harness`] or
/// [`crate::SttHarness::into_boxed`].
pub struct BoxedSttHarness {
    pub spec: SttHarnessSpec,
    pub backend: DynSpeechToText,
    pub(crate) audio: Mutex<Option<AudioSource>>,
    pub loop_strategy: Box<dyn SttLoopStrategy>,
    pub termination: Box<dyn SttTermination>,
    pub bus: EventBus,
    pub(crate) event_tx: broadcast::Sender<SttHarnessEvent>,
}

impl BoxedSttHarness {
    /// Assemble a boxed harness directly.
    pub fn new(
        spec: SttHarnessSpec,
        backend: DynSpeechToText,
        audio: AudioSource,
        loop_strategy: Box<dyn SttLoopStrategy>,
        termination: Box<dyn SttTermination>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(512);
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

    /// Subscribe to the STT-domain event stream. Call before `run()`.
    pub fn events(&self) -> SttEventStream {
        SttEventStream::new(self.event_tx.subscribe())
    }

    /// Drive the pipeline. Identical semantics to
    /// [`crate::SttHarness::run`].
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
            &*self.loop_strategy,
            &*self.termination,
            &self.bus,
            &sink,
        )
        .await
    }
}

#[async_trait]
impl SttHarnessDispatch for BoxedSttHarness {
    async fn dispatch(&self) -> CoreResult<Value> {
        let conv = self.run().await?;
        Ok(serde_json::to_value(conv).map_err(SttHarnessError::from)?)
    }
}

#[async_trait]
impl Callable for BoxedSttHarness {
    async fn call(&self, _input: Value, _ctx: CallCtx) -> CoreResult<Value> {
        let conv = self.run().await?;
        Ok(serde_json::to_value(conv).map_err(SttHarnessError::from)?)
    }

    fn label(&self) -> &str {
        self.spec.id.as_str()
    }
}
