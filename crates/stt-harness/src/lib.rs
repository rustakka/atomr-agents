//! STT harness — an agentic streaming speech-to-text pipeline.
//!
//! This crate is the bridge between the workspace's STT stack
//! (`stt-core`, `stt-audio`, `stt-diarize-sherpa`, the `stt-runtime-*`
//! backends) and its agentic stack (`harness`, `agent`, `callable`,
//! `observability`). It drives speech-to-text as a harness-style loop,
//! diarizes, and accumulates an [`SttConversation`] that maps cleanly
//! to and from agentic `TurnInput` / `Message`.
//!
//! # Shape
//!
//! [`SttHarness`] follows the workspace `BoxedX` pattern: a typed,
//! monomorphized form plus the erased [`BoxedSttHarness`], both
//! funnelling into a shared loop body, both reachable through the
//! [`SttHarnessRef`] handle that implements
//! [`Callable`](atomr_agents_callable::Callable) — so an STT harness
//! composes into workflows and teams like any other executable unit.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
//! use atomr_agents_stt_harness::{
//!     AudioSource, SttHarness, SttHarnessSpec, StreamingLoop, StreamEndTermination,
//! };
//!
//! let backend = Arc::new(MockSpeechToText::new().with_text("hello world"));
//! let audio = AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1));
//! let harness = SttHarness::new(
//!     SttHarnessSpec::new("demo"),
//!     backend,
//!     audio,
//!     StreamingLoop::default(),
//!     StreamEndTermination,
//! );
//! let conversation = harness.run().await?;
//! assert!(!conversation.turns.is_empty());
//! # Ok(()) }
//! ```
//!
//! # Pipeline
//!
//! 1. An [`AudioSource`] (mic / file / bytes / PCM) builds an audio
//!    pump.
//! 2. The [session task](session_actor) owns the live
//!    `StreamingSession`, pumps audio in, forwards `StreamEvent`s out.
//! 3. The [`SttLoopStrategy`] folds each event burst into the
//!    conversation; [`SttTermination`] caps the run.
//! 4. [`DiarizationPolicy`] decides speaker attribution — trust the
//!    backend, run a layered [`Diarizer`](atomr_agents_stt_diarize_sherpa::Diarizer),
//!    or skip it.
//! 5. Edits to speaker labels persist through a [`ConversationStore`].

mod audio_source;
mod boxed;
mod conversation;
mod diarize;
mod dispatch;
mod error;
mod events;
mod harness;
mod loop_strategy;
mod session_actor;
mod spec;
mod state;
mod store;
mod termination;

pub use audio_source::AudioSource;
pub use boxed::BoxedSttHarness;
pub use conversation::{PartialBuf, SpeakerMap, SpeakerRef, SttConversation, SttTurn, TurnState};
pub use dispatch::{SttHarnessDispatch, SttHarnessRef};
pub use error::{Result, SttHarnessError};
pub use events::{SttEventStream, SttHarnessEvent};
pub use harness::SttHarness;
pub use loop_strategy::{StreamingLoop, SttLoopStrategy, SttStepCtx, SttStepOutcome};
pub use spec::{DiarizationPolicy, SttHarnessConfig, SttHarnessSpec};
pub use state::{SttHarnessState, SttStepEvent};
pub use store::{ConversationStore, ConversationSummary, InMemoryConversationStore};
pub use termination::{
    AudioSecsTermination, BudgetTermination, CompositeTermination, StreamEndTermination, SttTermination,
    Termination, UtteranceCapTermination,
};

#[cfg(feature = "state")]
pub use store::CheckpointerConversationStore;

/// Re-export for convenience: every STT harness is a `Callable`.
pub use atomr_agents_callable::Callable;
