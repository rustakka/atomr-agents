//! [`InferenceClientFactory`] — the integration seam between an
//! `AgentBased{Role}` and a concrete LLM provider runner.
//!
//! The harness crate stays provider-agnostic: callers ship a factory
//! (e.g. wrapping `atomr_infer_runtime_anthropic::AnthropicRunner`) and
//! the role impls ask it for an
//! [`atomr_agents_agent::InferenceClient`] keyed on the per-role model
//! id.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_agent::InferenceClient;

use crate::error::Result;

/// Builds an [`InferenceClient`] for a given model id.
///
/// Implementations look at the model id (e.g. `claude-opus-4-7`,
/// `gpt-4o-mini`, `gemini-2.0-flash`) and return a runner wired to the
/// matching provider. The trait is async-trait-friendly to allow
/// implementations that lazily initialize the underlying runner.
#[async_trait]
pub trait InferenceClientFactory: Send + Sync + 'static {
    /// Build (or look up) an inference client for `model_id`.
    fn build(&self, model_id: &str) -> Result<Arc<dyn InferenceClient>>;
}
