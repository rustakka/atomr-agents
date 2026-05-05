//! Strategy traits — the universal extension point.
//!
//! Every component answers one question: given the current context
//! and budget, what do you contribute?

mod combinators;
mod memory;
mod policy;
mod routing;
mod skill;
mod tool;

pub use combinators::ChainedMemoryStrategy;
pub use memory::MemoryStrategy;
pub use policy::{Policy, PolicyDecision, PolicyStrategy};
pub use routing::{RoutingStrategy, RoutingTarget};
pub use skill::{SkillRef, SkillStrategy};
pub use tool::{ToolRef, ToolStrategy};

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, Result, TokenBudget};

/// Generic context strategy. Each implementer produces some output
/// given the agent context + a mutable token budget.
#[async_trait]
pub trait ContextStrategy: Send + Sync + 'static {
    type Output: Send + 'static;
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Self::Output>;
}

/// Loop strategy used by `Harness`. Generic over the harness state
/// type so the harness crate can plug in its concrete state.
#[async_trait]
pub trait LoopStrategy<S>: Send + Sync + 'static
where
    S: Send + 'static,
{
    type Outcome: Send + 'static;
    async fn step(&self, state: &mut S) -> Result<Self::Outcome>;
}

/// Termination strategy used by `Harness`.
pub trait TerminationStrategy<S>: Send + Sync + 'static
where
    S: Send + 'static,
{
    fn should_terminate(&self, state: &S) -> Termination;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    Continue,
    Done(&'static str),
}
