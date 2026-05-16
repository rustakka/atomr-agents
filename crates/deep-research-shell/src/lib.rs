//! Two-tier outer shell for [`atomr-agents-deep-research-harness`].
//!
//! An [`IntentClassifier`] routes incoming [`ResearchRequest`]s to
//! either a fast shallow path (one web-search call, no role loop) or
//! the full deep harness. The shell implements
//! [`atomr_agents_callable::Callable`] so it composes into agents,
//! workflows, and tool registries exactly like the underlying harness.
//!
//! The crate is intentionally minimal: it depends on
//! `atomr-agents-deep-research-core` for the contract types,
//! `atomr-agents-deep-research-harness` for the
//! [`DeepResearchHarnessRef`] handle, and
//! `atomr-agents-web-search-core` for the shallow path's `WebSearch`
//! provider. It does **not** depend on any web-search-provider crate;
//! callers wire in their own provider (or [`MockWebSearch`] for tests).
//!
//! ```ignore
//! use std::sync::Arc;
//! use atomr_agents_deep_research_shell::{
//!     DeepResearchShell, DirectSearchShallow, HeuristicIntentClassifier,
//! };
//!
//! let shell = DeepResearchShell::new(
//!     Arc::new(HeuristicIntentClassifier::new()),
//!     Arc::new(DirectSearchShallow::new(web_search.clone())),
//!     deep_ref,
//! );
//! let v = shell.call(serde_json::json!({"query": "rust"}), ctx).await?;
//! ```
//!
//! [`ResearchRequest`]: atomr_agents_deep_research_core::ResearchRequest
//! [`DeepResearchHarnessRef`]: atomr_agents_deep_research_harness::DeepResearchHarnessRef
//! [`MockWebSearch`]: atomr_agents_web_search_core::MockWebSearch

#![forbid(unsafe_code)]

mod classifier;
mod error;
mod shallow;
mod shell;

pub use classifier::{HeuristicIntentClassifier, IntentClassifier, ResearchTier};
pub use error::{Result, ShellError};
pub use shallow::{DirectSearchShallow, ShallowResearcher};
pub use shell::DeepResearchShell;

/// The shell exposes [`Callable`] from `atomr-agents-callable`.
pub use atomr_agents_callable::Callable;
