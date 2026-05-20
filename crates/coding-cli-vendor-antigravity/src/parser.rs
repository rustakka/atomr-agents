//! Antigravity CLI stream-json shape — close to Claude's but with
//! different envelope tags. Normalizes init / message / tool / result
//! events.

use atomr_agents_coding_cli_core::{
    CliEventParser, CliVendorKind, CodingCliEvent, FinishReason, ParseError,
};
use serde_json::Value;

#[derive(Default)]
pub struct AntigravityParser;

impl AntigravityParser {
    pub fn new() -> Self {
        Self
    }
}

impl CliEventParser for AntigravityParser {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError> {
        let s = line.trim();
        if s.is_empty() {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(s)?;
        Ok(normalize(&v))
    }

    fn flush(&mut self) -> Result<Vec<CodingCliEvent>, ParseError> {
        Ok(Vec::new())
    }
}

fn normalize(v: &Value) -> Vec<CodingCliEvent> {
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "init" => vec![CodingCliEvent::SystemInit {
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            plugins: Vec::new(),
        }],
        "message" => {
            if let Some(text) = v.pointer("/delta/text").and_then(Value::as_str) {
                return vec![CodingCliEvent::AssistantTextDelta {
                    text: text.to_string(),
                }];
            }
            if let Some(text) = v.get("text").and_then(Value::as_str) {
                return vec![CodingCliEvent::AssistantTextDelta {
                    text: text.to_string(),
                }];
            }
            vec![CodingCliEvent::RawVendorEvent {
                vendor: CliVendorKind::Antigravity,
                payload: v.clone(),
            }]
        }
        "tool_use" => {
            let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let input = v.get("args").or_else(|| v.get("input")).cloned().unwrap_or(Value::Null);
            vec![CodingCliEvent::ToolCallStarted {
                tool_call_id: id,
                name,
                input,
            }]
        }
        "tool_result" => {
            let id = v
                .get("tool_use_id")
                .or_else(|| v.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let output = v.get("content").or_else(|| v.get("output")).cloned();
            let error = v.get("error").and_then(Value::as_str).map(|s| s.to_string());
            vec![CodingCliEvent::ToolCallFinished {
                tool_call_id: id,
                output,
                error,
            }]
        }
        "usage" => {
            let input_tokens = v
                .pointer("/stats/input_tokens")
                .or_else(|| v.get("input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output_tokens = v
                .pointer("/stats/output_tokens")
                .or_else(|| v.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            vec![CodingCliEvent::Usage {
                input_tokens,
                output_tokens,
                cost_usd: None,
            }]
        }
        "result" => {
            let text = v
                .get("response")
                .and_then(Value::as_str)
                .or_else(|| v.get("result").and_then(Value::as_str))
                .map(|s| s.to_string());
            vec![CodingCliEvent::RunFinished {
                reason: FinishReason::Completed,
                result_text: text,
            }]
        }
        _ => vec![CodingCliEvent::RawVendorEvent {
            vendor: CliVendorKind::Antigravity,
            payload: v.clone(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_delta() {
        let mut p = AntigravityParser::new();
        let ev = p.parse_line(r#"{"type":"message","delta":{"text":"hi"}}"#).unwrap();
        assert!(matches!(&ev[0], CodingCliEvent::AssistantTextDelta { text } if text == "hi"));
    }

    #[test]
    fn parses_result_response() {
        let mut p = AntigravityParser::new();
        let ev = p.parse_line(r#"{"type":"result","response":"done"}"#).unwrap();
        assert!(matches!(
            &ev[0],
            CodingCliEvent::RunFinished {
                reason: FinishReason::Completed,
                result_text: Some(t),
            } if t == "done"
        ));
    }

    #[test]
    fn parses_usage_stats() {
        let mut p = AntigravityParser::new();
        let ev = p
            .parse_line(r#"{"type":"usage","stats":{"input_tokens":10,"output_tokens":5}}"#)
            .unwrap();
        assert!(matches!(
            &ev[0],
            CodingCliEvent::Usage { input_tokens: 10, output_tokens: 5, cost_usd: None }
        ));
    }

    #[test]
    fn unknown_falls_through() {
        let mut p = AntigravityParser::new();
        let ev = p.parse_line(r#"{"type":"weird","x":1}"#).unwrap();
        assert!(matches!(&ev[0], CodingCliEvent::RawVendorEvent { .. }));
    }
}
