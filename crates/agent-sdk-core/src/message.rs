//! Normalized message protocol yielded by an [`AgentSdkBackend`].
//!
//! The Python wrapper produces these as plain dicts from the SDK's message
//! objects (`SystemMessage` / `AssistantMessage` / `ResultMessage`), so the
//! `#[serde(tag = "type")]` discriminant and field names must stay in lock
//! step with `python/atomr_agents/agent_sdk.py::_normalize`.

use serde::{Deserialize, Serialize};

use crate::result::ResultSummary;

/// One normalized message from a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentSdkMessage {
    /// The SDK `system`/`init` message — carries the conversation session id.
    System {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtype: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        mcp_servers: Vec<String>,
    },
    /// An assistant turn, decomposed into content blocks.
    Assistant {
        #[serde(default)]
        blocks: Vec<ContentBlock>,
    },
    /// The terminal `ResultMessage`.
    Result(ResultSummary),
    /// Any message the wrapper didn't map. Always safe to ignore.
    #[serde(other)]
    Unknown,
}

/// One assistant content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    Thinking {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<serde_json::Value>,
        #[serde(default)]
        is_error: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_blocks_deserialize() {
        let j = r#"{"type":"assistant","blocks":[
            {"kind":"text","text":"hi"},
            {"kind":"tool_use","id":"t1","name":"Read","input":{"path":"a"}}
        ]}"#;
        let m: AgentSdkMessage = serde_json::from_str(j).unwrap();
        match m {
            AgentSdkMessage::Assistant { blocks } => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(blocks[0], ContentBlock::Text { .. }));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn result_flattens_fields() {
        let j = r#"{"type":"result","subtype":"success","result":"done","num_turns":2,"cost_usd":0.01}"#;
        let m: AgentSdkMessage = serde_json::from_str(j).unwrap();
        match m {
            AgentSdkMessage::Result(r) => {
                assert_eq!(r.subtype, "success");
                assert_eq!(r.num_turns, 2);
                assert_eq!(r.cost_micro_usd(), 10_000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn unknown_is_tolerated() {
        let m: AgentSdkMessage = serde_json::from_str(r#"{"type":"weird","x":1}"#).unwrap();
        assert!(matches!(m, AgentSdkMessage::Unknown));
    }
}
