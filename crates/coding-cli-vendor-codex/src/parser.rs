//! Codex stdout is plain text interspersed with optional JSON
//! envelopes. The parser is intentionally lenient: anything that
//! parses as JSON with a known `type` is normalized; everything else
//! is forwarded as an `AssistantTextDelta`. Unknown JSON envelopes are
//! forwarded as `RawVendorEvent` so we never silently drop data.

use atomr_agents_coding_cli_core::{
    CliEventParser, CliVendorKind, CodingCliEvent, FinishReason, ParseError,
};
use serde_json::Value;

#[derive(Default)]
pub struct CodexParser;

impl CodexParser {
    pub fn new() -> Self {
        Self
    }
}

impl CliEventParser for CodexParser {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError> {
        let trimmed = line.trim_end_matches('\n');
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(stripped) = strip_braces(trimmed) {
            if let Ok(v) = serde_json::from_str::<Value>(stripped) {
                return Ok(normalize(&v));
            }
        }
        // Plain text — emit as a delta.
        Ok(vec![CodingCliEvent::AssistantTextDelta {
            text: format!("{}\n", trimmed),
        }])
    }

    fn flush(&mut self) -> Result<Vec<CodingCliEvent>, ParseError> {
        Ok(Vec::new())
    }
}

fn strip_braces(s: &str) -> Option<&str> {
    let s = s.trim();
    if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')) {
        Some(s)
    } else {
        None
    }
}

fn normalize(v: &Value) -> Vec<CodingCliEvent> {
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "assistant" | "message" => {
            if let Some(text) = v
                .get("content")
                .and_then(Value::as_str)
                .or_else(|| v.get("text").and_then(Value::as_str))
            {
                return vec![CodingCliEvent::AssistantTextDelta {
                    text: text.to_string(),
                }];
            }
            vec![CodingCliEvent::RawVendorEvent {
                vendor: CliVendorKind::Codex,
                payload: v.clone(),
            }]
        }
        "tool_call" => {
            let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let input = v.get("arguments").or_else(|| v.get("input")).cloned().unwrap_or(Value::Null);
            vec![CodingCliEvent::ToolCallStarted {
                tool_call_id: id,
                name,
                input,
            }]
        }
        "tool_result" => {
            let id = v
                .get("tool_call_id")
                .or_else(|| v.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let output = v.get("output").or_else(|| v.get("content")).cloned();
            let error = v.get("error").and_then(Value::as_str).map(|s| s.to_string());
            vec![CodingCliEvent::ToolCallFinished {
                tool_call_id: id,
                output,
                error,
            }]
        }
        "usage" => {
            let input_tokens = v.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            let output_tokens = v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
            let cost_usd = v.get("cost_usd").and_then(Value::as_f64);
            vec![CodingCliEvent::Usage {
                input_tokens,
                output_tokens,
                cost_usd,
            }]
        }
        "done" | "result" => {
            let text = v
                .get("result")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            vec![CodingCliEvent::RunFinished {
                reason: FinishReason::Completed,
                result_text: text,
            }]
        }
        _ => vec![CodingCliEvent::RawVendorEvent {
            vendor: CliVendorKind::Codex,
            payload: v.clone(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_becomes_assistant_delta() {
        let mut p = CodexParser::new();
        let ev = p.parse_line("Hello world").unwrap();
        assert!(matches!(&ev[0], CodingCliEvent::AssistantTextDelta { text } if text.contains("Hello world")));
    }

    #[test]
    fn json_assistant_envelope() {
        let mut p = CodexParser::new();
        let ev = p.parse_line(r#"{"type":"assistant","content":"hi"}"#).unwrap();
        assert!(matches!(&ev[0], CodingCliEvent::AssistantTextDelta { text } if text == "hi"));
    }

    #[test]
    fn tool_call_and_result() {
        let mut p = CodexParser::new();
        let a = p.parse_line(r#"{"type":"tool_call","id":"c1","name":"read","arguments":{"path":"a.rs"}}"#).unwrap();
        let b = p.parse_line(r#"{"type":"tool_result","tool_call_id":"c1","output":"file"}"#).unwrap();
        assert!(matches!(&a[0], CodingCliEvent::ToolCallStarted { name, .. } if name == "read"));
        assert!(matches!(&b[0], CodingCliEvent::ToolCallFinished { error: None, .. }));
    }

    #[test]
    fn done_envelope() {
        let mut p = CodexParser::new();
        let ev = p.parse_line(r#"{"type":"done","result":"finished"}"#).unwrap();
        assert!(matches!(
            &ev[0],
            CodingCliEvent::RunFinished { reason: FinishReason::Completed, result_text: Some(t) } if t == "finished"
        ));
    }

    #[test]
    fn empty_line() {
        let mut p = CodexParser::new();
        assert!(p.parse_line("").unwrap().is_empty());
    }
}
