//! Uniform input/output contract for deep-research harnesses.
//!
//! The [`atomr_agents_deep_research_harness`](
//! https://docs.rs/atomr-agents-deep-research-harness) crate produces
//! values of this shape regardless of which loop-strategy
//! (clarify-plan-search-verify, multi-agent-parallel,
//! iterative-deepening) is selected, so callers can swap topologies
//! without touching the surrounding plumbing.

#![forbid(unsafe_code)]

mod artifacts;
mod citation;
mod coverage;
mod plan;
mod request;
mod result;
mod scope;
mod telemetry;
mod transcript;

pub use artifacts::{Artifacts, DraftSection, RawSearchHit};
pub use citation::{Citation, CitationStatus};
pub use coverage::CoverageSignals;
pub use plan::{Plan, SubQuestion, SubQuestionStatus};
pub use request::{ClarificationTurn, HitlPolicy, LlmOverrides, Markdown, OutputFormat, ResearchRequest};
pub use result::{ResearchResult, ResearchState};
pub use scope::{AttachmentRef, DataSourceRef, ResearchScope};
pub use telemetry::{NodeTelemetry, Telemetry};
pub use transcript::{NodeKind, NodeStep};
