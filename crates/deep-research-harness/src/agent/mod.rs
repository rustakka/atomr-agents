//! LLM-driven role implementations behind the `agent` feature flag.
//!
//! Each `AgentBased{Role}` impl wraps an [`atomr_agents_agent::Agent`]
//! built from the caller's [`InferenceClientFactory`]. Roles fall into
//! two patterns:
//!
//! - **Pattern B (one-shot structured output)** — clarifier, planner,
//!   critic, verifier. The agent is configured with an empty toolset and
//!   a role-specific system prompt; the model returns JSON that the role
//!   parses into its outcome type.
//! - **Pattern A (tool-loop)** — researcher, writer. The agent is
//!   configured with a subset of [`crate::tools::ResearchToolSet`] +
//!   (researcher only) [`atomr_agents_web_search_tool::WebSearchTool`].
//!   The agent's tool loop mutates the [`crate::ResearchHandle`]
//!   directly via the bound tools; the role then reads the updated
//!   `handle.snapshot()` to reconstruct its outcome.
//!
//! The factory is the integration seam: callers provide a closure or
//! struct that maps a per-role model id to an
//! [`atomr_agents_agent::InferenceClient`]. The harness crate stays
//! provider-agnostic.

mod clarifier;
mod critic;
mod factory;
mod parse;
mod planner;
mod prompts;
mod researcher;
mod strategies;
mod verifier;
mod writer;

pub use clarifier::AgentBasedClarifier;
pub use critic::AgentBasedCritic;
pub use factory::InferenceClientFactory;
pub use planner::AgentBasedPlanner;
pub use researcher::AgentBasedResearcher;
pub use verifier::AgentBasedCitationVerifier;
pub use writer::AgentBasedWriter;
