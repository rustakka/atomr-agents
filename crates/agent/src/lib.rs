//! `Agent<I, T, Ms, Ml, Sk>` and the per-turn pipeline.
//!
//! The agent holds an `InferenceClient` (an abstraction over a
//! `ModelRunner` from atomr-infer); strategies are generic so the
//! hot path is monomorphized. A `BoxedAgent` form is provided for
//! config-driven instantiation.

mod inference;
mod middleware;
mod pipeline;
mod spec;
mod r#trait;

pub use inference::{InferenceClient, LocalRunnerClient, TurnResult};
pub use middleware::{
    AgentMiddleware, LoggingMiddleware, MiddlewareStack, RateLimitMiddleware, RedactionMiddleware,
    ToolErrorRecoveryMiddleware,
};
pub use pipeline::Agent;
pub use spec::AgentSpec;
pub use r#trait::AgentRef;

pub use atomr_agents_tool::Provider;
