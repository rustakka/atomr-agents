//! Abstraction over a `ModelRunner` that produces a `TurnResult`
//! (text, usage, finish reason, parsed tool calls) per `ExecuteBatch`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};
use atomr_agents_tool::{Provider, ToolCallParser};
use atomr_infer_core::batch::ExecuteBatch;
use atomr_infer_core::runner::ModelRunner;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};
use futures::stream::StreamExt;
use tokio::sync::Mutex;

use atomr_agents_tool::ParsedToolCall;

#[derive(Debug, Default)]
pub struct TurnResult {
    pub text: String,
    pub usage: TokenUsage,
    pub finish_reason: Option<FinishReason>,
    pub tool_calls: Vec<ParsedToolCall>,
}

/// Implemented by anything the agent can use to drive an inference
/// request. The provider matters for tool-call delta parsing.
#[async_trait]
pub trait InferenceClient: Send + Sync + 'static {
    fn provider(&self) -> Provider;
    async fn run(&self, batch: ExecuteBatch) -> Result<TurnResult>;
}

/// Wrap any `ModelRunner` (including `MockRunner`) as an
/// `InferenceClient`. Single-runner concurrency is bounded by the
/// internal mutex; production setups should use the
/// `EngineCoreActor` from `atomr-infer-runtime` instead.
pub struct LocalRunnerClient<R: ModelRunner> {
    runner: Arc<Mutex<R>>,
    provider: Provider,
}

impl<R: ModelRunner + 'static> LocalRunnerClient<R> {
    pub fn new(runner: R, provider: Provider) -> Self {
        Self { runner: Arc::new(Mutex::new(runner)), provider }
    }

    pub fn from_arc(runner: Arc<Mutex<R>>, provider: Provider) -> Self {
        Self { runner, provider }
    }
}

#[async_trait]
impl<R: ModelRunner + 'static> InferenceClient for LocalRunnerClient<R> {
    fn provider(&self) -> Provider {
        self.provider
    }

    async fn run(&self, batch: ExecuteBatch) -> Result<TurnResult> {
        let mut g = self.runner.lock().await;
        let handle = g
            .execute(batch)
            .await
            .map_err(|e| AgentError::Inference(e.to_string()))?;
        // Drop the mutex before consuming the stream — the stream is
        // produced by `execute` and is independent of `&mut self`.
        drop(g);
        let mut text = String::new();
        let mut usage = TokenUsage::default();
        let mut finish: Option<FinishReason> = None;
        let mut parser = ToolCallParser::new(self.provider);
        let mut stream = handle.into_stream();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| AgentError::Inference(e.to_string()))?;
            text.push_str(&chunk.text_delta);
            if let Some(d) = chunk.tool_call_delta.as_ref() {
                parser.feed(d)?;
            }
            if let Some(u) = chunk.usage {
                usage.add(u);
            }
            if let Some(r) = chunk.finish_reason {
                finish = Some(r);
            }
        }
        let tool_calls = parser.finish();
        Ok(TurnResult { text, usage, finish_reason: finish, tool_calls })
    }
}
