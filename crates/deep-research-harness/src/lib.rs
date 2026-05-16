//! Deep-research harness.
//!
//! See the crate's `README.md` and `docs/deep-research-harness.md` for
//! the conceptual overview. In short: this crate hosts the typed +
//! boxed harness, six pluggable role traits (`Clarifier`, `Planner`,
//! `Researcher`, `Writer`, `Critic`, `CitationVerifier`), three v1
//! topology strategies, and a deterministic LLM-free default role set
//! so tests and the web UI run end-to-end without a model provider.
//!
//! The contract types ([`ResearchRequest`], [`ResearchResult`]) live
//! in the sibling [`atomr_agents_deep_research_core`] crate.

#![forbid(unsafe_code)]

mod boxed;
mod dispatch;
mod error;
mod events;
mod handle;
mod harness;
mod loop_strategy;
mod roles;
mod spec;
mod state;
mod store;
mod strategies;
mod termination;
pub mod tools;

#[cfg(feature = "agent")]
pub mod agent;

pub use boxed::BoxedDeepResearchHarness;
pub use dispatch::{parse_request, DeepResearchHarnessDispatch, DeepResearchHarnessRef};
pub use error::{DeepResearchError, Result};
pub use events::{DeepResearchEvent, DeepResearchEventStream};
pub use handle::ResearchHandle;
pub use harness::{DeepResearchHarness, DeepResearchRoles};
pub use loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
pub use roles::{
    CitationVerifier, Clarifier, ClarifyOutcome, ConcatWriter, Critic, CritiqueOutcome,
    DeterministicCitationVerifier, HeuristicPlanner, MockResearcher, Planner, RegexCritic, Researcher,
    TemplateClarifier, Writer,
};
pub use spec::{DeepResearchConfig, DeepResearchHarnessSpec};
pub use state::{DeepResearchState, DeepResearchStepEvent};
pub use store::{InMemoryResearchStore, ResearchStore, ResearchSummary};
pub use strategies::{
    ClarifyPlanSearchVerifyLoop, IterativeDeepeningLoop, LinearWriteCritiqueLoop, MultiAgentParallelLoop,
    OutlineFirstSectionFanoutLoop, PlanAndExecuteLoop,
};
pub use termination::{
    BudgetTermination, CompositeTermination, DeepResearchTermination, IterationCapTermination,
    StrategyControlledTermination, Termination,
};
pub use tools::{
    AppendCitationTool, AppendDraftSectionTool, AppendSubQuestionTool, RecordClarificationTool,
    RecordCritiqueTool, RecordSearchHitTool, ResearchToolSet, SetFinalReportTool, SetPlanTool,
    SetSubQuestionStatusTool,
};

#[cfg(feature = "agent")]
pub use agent::{
    AgentBasedCitationVerifier, AgentBasedClarifier, AgentBasedCritic, AgentBasedPlanner,
    AgentBasedResearcher, AgentBasedWriter, InferenceClientFactory,
};

/// Re-exported core types for convenience.
pub use atomr_agents_deep_research_core::{
    Artifacts, Citation, CitationStatus, ClarificationTurn, CoverageSignals, DataSourceRef, DraftSection,
    HitlPolicy, LlmOverrides, NodeKind, NodeStep, NodeTelemetry, OutputFormat, Plan, RawSearchHit,
    ResearchRequest, ResearchResult, ResearchScope, ResearchState, SubQuestion, SubQuestionStatus, Telemetry,
};

/// Every deep-research harness is a `Callable`.
pub use atomr_agents_callable::Callable;
