//! Tool-call parser.
//!
//! Consumes the opaque `tool_call_delta` JSON values that
//! `atomr-infer-core` streams as part of `TokenChunk`. Producers
//! differ by provider, so the parser dispatches on a `Provider`
//! discriminant and accumulates partial JSON-string arguments
//! across chunks.

use atomr_agents_core::{AgentError, Result, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    /// JSON string of arguments (assembled from streaming deltas).
    pub arguments_raw: String,
}

impl ParsedToolCall {
    pub fn arguments(&self) -> Result<Value> {
        if self.arguments_raw.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str::<Value>(&self.arguments_raw)
            .map_err(|e| AgentError::Tool(format!("tool args parse: {e}")))
    }
}

/// Stateful streaming parser. Feed each `tool_call_delta` value as it
/// arrives; call `finish` to drain the accumulated calls.
pub struct ToolCallParser {
    provider: Provider,
    /// keyed by tool-call index (OpenAI) or content-block index (Anthropic).
    calls: BTreeMap<u32, Partial>,
}

#[derive(Debug, Default)]
struct Partial {
    id: String,
    name: String,
    args: String,
}

impl ToolCallParser {
    pub fn new(provider: Provider) -> Self {
        Self { provider, calls: BTreeMap::new() }
    }

    pub fn feed(&mut self, delta: &Value) -> Result<()> {
        match self.provider {
            Provider::OpenAi => self.feed_openai(delta),
            Provider::Anthropic => self.feed_anthropic(delta),
        }
    }

    pub fn finish(self) -> Vec<ParsedToolCall> {
        self.calls
            .into_iter()
            .map(|(_, p)| ParsedToolCall { id: p.id, name: p.name, arguments_raw: p.args })
            .collect()
    }

    fn feed_openai(&mut self, delta: &Value) -> Result<()> {
        // OpenAI streams an array of tool-call deltas under
        // `delta.tool_calls`. Each element has `index`, optional `id`,
        // optional `type`, and `function: { name?, arguments? }`.
        let arr = delta
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AgentError::Tool("openai: missing tool_calls".into()))?;
        for item in arr {
            let idx = item
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| AgentError::Tool("openai: tool_call missing index".into()))?
                as u32;
            let entry = self.calls.entry(idx).or_default();
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                entry.id = id.to_string();
            }
            if let Some(func) = item.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    entry.name.push_str(name);
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.args.push_str(args);
                }
            }
        }
        Ok(())
    }

    fn feed_anthropic(&mut self, delta: &Value) -> Result<()> {
        // Anthropic SSE event shapes:
        //   content_block_start { index, content_block: { type:"tool_use", id, name, input: {} } }
        //   content_block_delta { index, delta: { type:"input_json_delta", partial_json: "..." } }
        // We accept either shape, identified by which keys are
        // present.
        let idx = delta
            .get("index")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| AgentError::Tool("anthropic: delta missing index".into()))?
            as u32;
        let entry = self.calls.entry(idx).or_default();
        if let Some(block) = delta.get("content_block") {
            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                    entry.id = id.to_string();
                }
                if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                    entry.name = name.to_string();
                }
                // Sometimes the full input arrives at start. Skip
                // empty objects since deltas will follow.
                if let Some(input) = block.get("input") {
                    let is_empty_object = input
                        .as_object()
                        .map(|m| m.is_empty())
                        .unwrap_or(false);
                    if !input.is_null() && !is_empty_object {
                        entry.args = serde_json::to_string(input).unwrap_or_default();
                    }
                }
            }
        }
        if let Some(d) = delta.get("delta") {
            if d.get("type").and_then(|v| v.as_str()) == Some("input_json_delta") {
                if let Some(pj) = d.get("partial_json").and_then(|v| v.as_str()) {
                    entry.args.push_str(pj);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_streaming_assembly() {
        let mut p = ToolCallParser::new(Provider::OpenAi);
        p.feed(&json!({
            "tool_calls": [{
                "index": 0,
                "id": "call_abc",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":"}
            }]
        }))
        .unwrap();
        p.feed(&json!({
            "tool_calls": [{
                "index": 0,
                "function": {"arguments": "\"NYC\"}"}
            }]
        }))
        .unwrap();
        let calls = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].name, "get_weather");
        let args = calls[0].arguments().unwrap();
        assert_eq!(args, json!({"city": "NYC"}));
    }

    #[test]
    fn anthropic_streaming_assembly() {
        let mut p = ToolCallParser::new(Provider::Anthropic);
        p.feed(&json!({
            "index": 1,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_xyz",
                "name": "search",
                "input": {}
            }
        }))
        .unwrap();
        p.feed(&json!({
            "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"q\":"}
        }))
        .unwrap();
        p.feed(&json!({
            "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "\"rust\"}"}
        }))
        .unwrap();
        let calls = p.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_xyz");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments().unwrap(), json!({"q": "rust"}));
    }
}
