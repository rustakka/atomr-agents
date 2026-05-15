//! Meetings harness — turns a diarized STT conversation into structured
//! attendees, notes, and actions with owners.
//!
//! This crate sits *downstream* of [`atomr_agents_stt_harness`]. It
//! takes the diarized [`SttConversation`](atomr_agents_stt_harness::SttConversation)
//! produced by an STT run (loaded from whichever `Checkpointer` backend
//! is configured) and produces a [`MeetingAnalysis`]: an attendee
//! roster, a **linear, append-only** ledger of notes and actions, and a
//! **tiered, dynamically regenerated** summary stack.
//!
//! The analysis is persisted under the **same `conversation_id`** as the
//! source transcript so the two records join naturally in the same
//! store. See [`MeetingsStore`] and (with feature `state`)
//! [`CheckpointerMeetingsStore`].
//!
//! # Modes
//!
//! - **Batch** ([`RunMode::Batch`]) — the source transcript is read
//!   once, the extractor produces the full analysis in a single bounded
//!   run.
//! - **Live** ([`RunMode::Live`]) — subscribes to the STT harness's
//!   `tokio::broadcast` event channel and updates the analysis as new
//!   turns commit; new notes/actions are *appended* (never reordered),
//!   the in-flight tail segment summary is revised, earlier segments
//!   are frozen, and the running rollup is recomposed when a segment
//!   finalizes.
//!
//! # Shape
//!
//! Like [`SttHarness`](atomr_agents_stt_harness::SttHarness), this
//! crate uses the typed-plus-boxed split: a monomorphized
//! [`MeetingsHarness<L, T>`], the type-erased [`BoxedMeetingsHarness`],
//! and the public [`MeetingsHarnessRef`] handle that implements
//! [`Callable`](atomr_agents_callable::Callable).

#![forbid(unsafe_code)]

mod analysis;
mod boxed;
mod dispatch;
mod error;
mod events;
mod extractor;
mod harness;
mod loop_strategy;
mod spec;
mod state;
mod store;
mod termination;
mod tools;

pub use analysis::{
    Action, ActionStatus, AnalysisState, Attendee, MeetingAnalysis, Note, RunMode, SegmentSummary,
    SummaryLevels,
};
pub use boxed::BoxedMeetingsHarness;
pub use dispatch::{MeetingsHarnessDispatch, MeetingsHarnessRef};
pub use error::{MeetingsHarnessError, Result};
pub use events::{MeetingsEventStream, MeetingsHarnessEvent};
pub use extractor::{
    ExtractionRequest, ExtractionWindow, MeetingExtractor, RuleBasedExtractor,
};
pub use harness::MeetingsHarness;
pub use loop_strategy::{
    BatchExtractionLoop, MeetingsLoopStrategy, MeetingsStepCtx, MeetingsStepOutcome,
    StreamingExtractionLoop,
};
pub use spec::{AutoTriggerCfg, MeetingsHarnessConfig, MeetingsHarnessSpec};
pub use state::{MeetingsHarnessState, MeetingsStepEvent};
pub use store::{InMemoryMeetingsStore, MeetingsStore, MeetingsSummary};
pub use termination::{
    BudgetTermination, CompositeTermination, IterationCapTermination, MeetingsTermination,
    StreamEndTermination, Termination,
};
pub use tools::{
    AppendActionTool, AppendNoteTool, FinalizeSegmentTool, FinalizeTool, GetTurnTool, ListTurnsTool,
    MeetingsToolSet, RegenerateRunningTool, ReviseTailSegmentTool, SetTitleTool, ToolHandle,
    UpdateActionTool, UpsertAttendeeTool,
};

#[cfg(feature = "state")]
pub use store::CheckpointerMeetingsStore;

/// Convenience: every meetings harness is a `Callable`.
pub use atomr_agents_callable::Callable;
