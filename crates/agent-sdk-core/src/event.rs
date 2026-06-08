//! Normalized broadcast event schema for the agent-sdk harness.
//!
//! The harness fans these out on a `tokio::sync::broadcast` channel; SSE in
//! the web companion and the Python async iterator both consume from it.
//! Mirrors `CodingCliEvent` in shape and `recv` semantics.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::request::AgentRunId;

/// Why a run finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Completed,
    MaxTurns,
    BudgetExhausted,
    Interrupted,
    Error,
}

/// Normalized lifecycle events. Tagged enum — serializes as
/// `{"kind": "...", ...}` so the web client can switch on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentSdkEvent {
    /// The harness started a run, before any SDK events.
    RunStarted {
        run_id: AgentRunId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// The SDK `system`/`init`: tools + MCP servers + session id.
    SystemInit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        tools: Vec<String>,
        mcp_servers: Vec<String>,
    },
    /// Streaming assistant text (one per block / delta).
    AssistantTextDelta { text: String },
    /// Streaming extended-thinking text.
    ThinkingDelta { text: String },
    /// Agent invoked a tool.
    ToolUse {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
    },
    /// A tool returned.
    ToolResult { tool_use_id: String, is_error: bool },
    /// Token / cost accounting from a result.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    /// Terminal event.
    RunFinished {
        reason: FinishReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_text: Option<String>,
    },
    /// Free-form diagnostic.
    Note { message: String },
}

/// Subscriber handle backed by a `broadcast::Receiver`. Drops lagged events
/// silently; returns `None` once the channel closes.
pub struct AgentSdkEventStream {
    rx: broadcast::Receiver<AgentSdkEvent>,
}

impl AgentSdkEventStream {
    pub fn new(rx: broadcast::Receiver<AgentSdkEvent>) -> Self {
        Self { rx }
    }

    pub async fn recv(&mut self) -> Option<AgentSdkEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_round_trips_json() {
        let ev = AgentSdkEvent::AssistantTextDelta { text: "Hi".into() };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains("\"kind\":\"assistant_text_delta\""));
        let back: AgentSdkEvent = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, AgentSdkEvent::AssistantTextDelta { .. }));
    }

    #[test]
    fn finish_reason_snake_case() {
        assert_eq!(
            serde_json::to_string(&FinishReason::MaxTurns).unwrap(),
            "\"max_turns\""
        );
    }
}
