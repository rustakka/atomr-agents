//! Pluggable role traits + deterministic default implementations.
//!
//! Every loop strategy composes the same six roles. The defaults are
//! LLM-free so tests, the web UI, and ingest pipelines exercise
//! end-to-end without a model provider — the same pattern as
//! `RuleBasedExtractor` in the meetings harness.

mod clarifier;
mod critic;
mod planner;
mod researcher;
mod verifier;
mod writer;

pub use clarifier::{Clarifier, ClarifyOutcome, TemplateClarifier};
pub use critic::{Critic, CritiqueOutcome, RegexCritic};
pub use planner::{HeuristicPlanner, Planner};
pub use researcher::{MockResearcher, Researcher};
pub use verifier::{CitationVerifier, DeterministicCitationVerifier};
pub use writer::{ConcatWriter, Writer};
