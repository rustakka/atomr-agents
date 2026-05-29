//! `Agent<I, T, Ms, Ml, Sk>` and the per-turn pipeline.
//!
//! The agent holds an `InferenceClient` (an abstraction over a
//! `ModelRunner` from atomr-infer); strategies are generic so the
//! hot path is monomorphized. [`BoxedAgent`] (object-erased form)
//! and [`AgentRef`] (Arc-wrapped handle) provide config-driven
//! instantiation; see [`AgentSpec::into_agent`].

mod boxed;
mod cost;
mod inference;
mod middleware;
mod model_pin;
mod pipeline;
mod replay;
mod spec;
mod r#trait;

pub use boxed::BoxedAgent;
pub use cost::{
    Budget, BudgetExceeded, BudgetScope, Cap, CostEstimate, CostMeter, DecisionKey, ModelPricing, Pricing,
    Spend, SpendLedger,
};
pub use inference::{InferenceClient, LocalRunnerClient, TurnResult};
pub use middleware::{
    AgentMiddleware, LoggingMiddleware, MiddlewareStack, RateLimitMiddleware, RedactionMiddleware,
    ToolErrorRecoveryMiddleware,
};
pub use model_pin::{
    detect_drift, DriftKind, ModelPin, ModelPinViolation, ModelResolver, PinnedClient, ResolvedModel,
    StaticResolver,
};
pub use pipeline::{Agent, AgentBudgets};
pub use r#trait::AgentRef;
pub use replay::{record_turn, turn_from_record, ReplayError, ReplayProvider};
pub use spec::AgentSpec;

pub use atomr_agents_tool::Provider;

/// Provider runtime back-ends, gated by per-provider features.
/// Wrap any of these `Runner` types in `LocalRunnerClient` to use them
/// as an `InferenceClient`.
pub mod providers {
    #[cfg(feature = "provider-anthropic")]
    pub mod anthropic {
        pub use atomr_infer_runtime_anthropic::{
            classify_anthropic_error, AnthropicConfig, AnthropicPricing, AnthropicRunner,
        };
    }
    #[cfg(feature = "provider-openai")]
    pub mod openai {
        pub use atomr_infer_runtime_openai::{
            classify_openai_error, OpenAiConfig, OpenAiPricing, OpenAiRunner, OpenAiVariant,
        };
    }
    #[cfg(feature = "provider-gemini")]
    pub mod gemini {
        pub use atomr_infer_runtime_gemini::{
            classify_gemini_error, GeminiConfig, GeminiPricing, GeminiRunner, GeminiVariant, SafetySetting,
        };
    }
}
