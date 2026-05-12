//! Inference client factories for `PyAgent::from_spec`.
//!
//! Maps a provider string onto a concrete `Arc<dyn InferenceClient>`.
//! W3b ships a `"mock"` provider that produces a deterministic
//! `TurnResult` so the Python type-system path can be exercised end
//! to end without an LLM. Real backends (`"anthropic"`, `"openai"`,
//! `"gemini"`) require:
//!
//! * the corresponding `provider-*` feature on `atomr-agents-agent`,
//! * an HTTP runner constructed from per-provider config
//!   (`AnthropicRunner::new(...)`, etc.),
//! * `LocalRunnerClient::new(runner, Provider::...)` to wrap it.
//!
//! Those features are gated through `atomr-agents-agent`'s optional
//! deps and are not enabled in the integration tree's default build,
//! so we surface a friendly error rather than silently breaking. A
//! future iteration can light them up by enabling the features in
//! py-bindings' `Cargo.toml` and threading API-key env vars through
//! here.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_agent::{InferenceClient, Provider, TurnResult};
use atomr_agents_core::{AgentError, Result as AgentResult};
use atomr_infer_core::batch::ExecuteBatch;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};

/// Deterministic in-process inference client used for tests / smoke
/// runs. Returns a fixed canned response regardless of input.
pub struct MockInferenceClient {
    text: String,
    provider: Provider,
}

impl MockInferenceClient {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            provider: Provider::OpenAi,
        }
    }
}

#[async_trait]
impl InferenceClient for MockInferenceClient {
    fn provider(&self) -> Provider {
        self.provider
    }

    async fn run(&self, _batch: ExecuteBatch) -> AgentResult<TurnResult> {
        // Approximate token counts from char length so usage looks
        // realistic to downstream observers but stays deterministic.
        let approx = ((self.text.chars().count() + 3) / 4) as u32;
        Ok(TurnResult {
            text: self.text.clone(),
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: approx,
                reasoning_tokens: 0,
                cached_tokens: 0,
            },
            finish_reason: Some(FinishReason::Stop),
            tool_calls: Vec::new(),
        })
    }
}

/// Factory: turn a provider string into an inference client. Only
/// `"mock"` is wired in this build; the real provider constructors
/// require feature-gated runner crates and env-driven configuration.
pub fn build_inference_client(provider: &str) -> Result<Arc<dyn InferenceClient>, AgentError> {
    match provider {
        "mock" => Ok(Arc::new(MockInferenceClient::new(
            "[mock] PyAgent.run_turn smoke response",
        ))),
        "anthropic" | "openai" | "gemini" => Err(AgentError::Inference(format!(
            "inference provider {provider:?} is not wired into py-bindings yet; \
             enable the provider-{provider} feature on atomr-agents-agent and \
             extend crate::inference::build_inference_client. Use \"mock\" for \
             smoke tests."
        ))),
        other => Err(AgentError::Inference(format!(
            "unknown inference provider {other:?}; expected one of mock|anthropic|openai|gemini"
        ))),
    }
}
