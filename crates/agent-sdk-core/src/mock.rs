//! `MockBackend` — an in-memory, deterministic backend.
//!
//! Lives in `-core` (like `MockBackend` in `sandbox-core`) so every
//! downstream crate — harness, web, pyo3 — can be unit-tested without the
//! `claude-agent-sdk`, the `claude` CLI, or network/credits. It emits a
//! scripted turn: `system` → `assistant` → optional tool round-trip →
//! `result`.

use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;

use crate::backend::{AgentSdkBackend, AgentSdkSession, MessageStream};
use crate::config::AgentSdkConfig;
use crate::error::AgentSdkError;
use crate::message::{AgentSdkMessage, ContentBlock};
use crate::request::{AgentSessionId, QueryRequest, SessionSpec};
use crate::result::{ResultSummary, UsageSummary};

/// In-memory deterministic backend. Always available.
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    /// If set, the assistant echoes this instead of the prompt.
    pub canned_reply: Option<String>,
    /// If set, the scripted turn includes one tool round-trip with this name.
    pub tool: Option<String>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_reply(mut self, reply: impl Into<String>) -> Self {
        self.canned_reply = Some(reply.into());
        self
    }

    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    fn scripted_turn(&self, prompt: &str, config: &AgentSdkConfig) -> Vec<AgentSdkMessage> {
        let session_id = "mock-session".to_string();
        let reply = self
            .canned_reply
            .clone()
            .unwrap_or_else(|| format!("[mock] {prompt}"));
        let mut msgs = vec![AgentSdkMessage::System {
            subtype: Some("init".into()),
            session_id: Some(session_id.clone()),
            model: config.model.clone(),
            tools: config.allowed_tools.clone(),
            mcp_servers: config.mcp_servers.keys().cloned().collect(),
        }];

        let mut blocks = vec![ContentBlock::Text { text: reply.clone() }];
        if let Some(name) = &self.tool {
            blocks.push(ContentBlock::ToolUse {
                id: "mock-tool-1".into(),
                name: name.clone(),
                input: serde_json::json!({}),
            });
            blocks.push(ContentBlock::ToolResult {
                tool_use_id: "mock-tool-1".into(),
                content: Some(serde_json::json!("ok")),
                is_error: false,
            });
        }
        msgs.push(AgentSdkMessage::Assistant { blocks });

        msgs.push(AgentSdkMessage::Result(ResultSummary {
            subtype: "success".into(),
            result: Some(reply),
            session_id: Some(session_id),
            num_turns: 1,
            duration_ms: Some(0),
            cost_usd: Some(0.0),
            usage: UsageSummary {
                input_tokens: 1,
                output_tokens: 1,
                ..Default::default()
            },
            is_error: false,
        }));
        msgs
    }

    fn stream_of(msgs: Vec<AgentSdkMessage>) -> MessageStream {
        futures::stream::iter(msgs.into_iter().map(Ok)).boxed()
    }
}

#[async_trait]
impl AgentSdkBackend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }

    async fn available(&self) -> bool {
        true
    }

    async fn query(&self, req: QueryRequest) -> Result<MessageStream, AgentSdkError> {
        Ok(Self::stream_of(self.scripted_turn(&req.prompt, &req.config)))
    }

    async fn create_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSdkSession>, AgentSdkError> {
        Ok(Box::new(MockSession {
            backend: self.clone(),
            id: AgentSessionId::new(),
            config: spec.config,
            last_prompt: Mutex::new(spec.initial_prompt.unwrap_or_default()),
        }))
    }
}

/// A live mock session.
pub struct MockSession {
    backend: MockBackend,
    id: AgentSessionId,
    config: AgentSdkConfig,
    last_prompt: Mutex<String>,
}

#[async_trait]
impl AgentSdkSession for MockSession {
    fn session_id(&self) -> &AgentSessionId {
        &self.id
    }

    async fn send(&self, prompt: String) -> Result<(), AgentSdkError> {
        *self.last_prompt.lock() = prompt;
        Ok(())
    }

    async fn receive(&self) -> Result<MessageStream, AgentSdkError> {
        let prompt = self.last_prompt.lock().clone();
        Ok(MockBackend::stream_of(
            self.backend.scripted_turn(&prompt, &self.config),
        ))
    }

    async fn interrupt(&self) -> Result<(), AgentSdkError> {
        Ok(())
    }

    async fn set_permission_mode(&self, _mode: String) -> Result<(), AgentSdkError> {
        Ok(())
    }

    async fn set_model(&self, _model: String) -> Result<(), AgentSdkError> {
        Ok(())
    }

    async fn close(&self) -> Result<(), AgentSdkError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn mock_query_yields_terminal_result() {
        let b = MockBackend::new().with_reply("hello");
        let mut s = b.query(QueryRequest::new("ignored")).await.unwrap();
        let mut saw_result = false;
        while let Some(item) = s.next().await {
            if let AgentSdkMessage::Result(r) = item.unwrap() {
                assert_eq!(r.subtype, "success");
                assert_eq!(r.result.as_deref(), Some("hello"));
                saw_result = true;
            }
        }
        assert!(saw_result);
    }

    #[tokio::test]
    async fn mock_session_round_trips() {
        let b = MockBackend::new().with_tool("Read");
        let sess = b.create_session(SessionSpec::default()).await.unwrap();
        sess.send("hi".into()).await.unwrap();
        let mut s = sess.receive().await.unwrap();
        let mut tool_seen = false;
        while let Some(item) = s.next().await {
            if let AgentSdkMessage::Assistant { blocks } = item.unwrap() {
                tool_seen |= blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
            }
        }
        assert!(tool_seen);
        sess.close().await.unwrap();
    }
}
