//! Normalized event schema emitted by every vendor adapter.
//!
//! Adapters parse vendor-specific NDJSON / line streams and translate
//! them into `CodingCliEvent`. The harness fans events out on a
//! `tokio::sync::broadcast` channel; SSE in the web companion and the
//! Python async iterator both consume from this channel.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::request::{CliRunId, CliSessionId};
use crate::vendor::CliVendorKind;

/// Why the run finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// CLI reported a successful completion.
    Completed,
    /// Reached the wall-clock or token budget.
    BudgetExhausted,
    /// User or harness cancelled the run.
    Cancelled,
    /// CLI exited with a non-zero status.
    ProcessError,
    /// Parse pipeline gave up on the stream.
    StreamError,
}

/// One tool descriptor reported by the CLI during init.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptorInit {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One MCP server the CLI loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInit {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Normalized lifecycle events. Tagged enum — serializes as
/// `{"kind": "...", ...}` so the web client can switch on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodingCliEvent {
    /// The harness has spawned the CLI process. Emitted before any
    /// vendor-side events.
    RunStarted {
        run_id: CliRunId,
        vendor: CliVendorKind,
        model: Option<String>,
        session_id: Option<CliSessionId>,
    },

    /// The CLI's `system/init` (or equivalent): tools loaded, MCP
    /// servers connected, plugins resolved.
    SystemInit {
        tools: Vec<ToolDescriptorInit>,
        mcp_servers: Vec<McpServerInit>,
        #[serde(default)]
        plugins: Vec<String>,
    },

    /// Streaming assistant text. Adapters emit one event per delta.
    AssistantTextDelta { text: String },

    /// CLI started a tool call.
    ToolCallStarted {
        tool_call_id: String,
        name: String,
        input: serde_json::Value,
    },

    /// CLI's tool call returned.
    ToolCallFinished {
        tool_call_id: String,
        output: Option<serde_json::Value>,
        error: Option<String>,
    },

    /// Vendor reported a retryable API error.
    ApiRetry {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },

    /// Token / cost accounting.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    },

    /// Terminal event — process is done.
    RunFinished {
        reason: FinishReason,
        result_text: Option<String>,
    },

    /// Pass-through for vendor-specific events the normalizer doesn't
    /// yet map. Always safe to ignore in the UI.
    RawVendorEvent {
        vendor: CliVendorKind,
        payload: serde_json::Value,
    },

    /// Free-form diagnostic message (parser warnings, etc.).
    Note {
        message: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        fields: BTreeMap<String, serde_json::Value>,
    },
}

/// Subscriber handle backed by a `broadcast::Receiver`. Drops missed
/// events silently — same semantics as `DeepResearchEventStream`.
pub struct CodingCliEventStream {
    rx: broadcast::Receiver<CodingCliEvent>,
}

impl CodingCliEventStream {
    pub fn new(rx: broadcast::Receiver<CodingCliEvent>) -> Self {
        Self { rx }
    }

    /// Wait for the next event. Returns `None` once the channel closes.
    pub async fn recv(&mut self) -> Option<CodingCliEvent> {
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
        let ev = CodingCliEvent::AssistantTextDelta {
            text: "Hello".into(),
        };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains("\"kind\":\"assistant_text_delta\""));
        let back: CodingCliEvent = serde_json::from_str(&j).unwrap();
        if let CodingCliEvent::AssistantTextDelta { text } = back {
            assert_eq!(text, "Hello");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn finish_reason_serializes_snake_case() {
        let j = serde_json::to_string(&FinishReason::BudgetExhausted).unwrap();
        assert_eq!(j, "\"budget_exhausted\"");
    }
}
