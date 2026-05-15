//! Translates Claude Code's `--output-format stream-json` NDJSON into
//! normalized `CodingCliEvent`s.
//!
//! The schema is publicly documented at
//! <https://code.claude.com/docs/en/headless>. We normalize each
//! known envelope into the common event model, and pass anything else
//! through as `RawVendorEvent` so the UI never silently drops data.

use atomr_agents_coding_cli_core::{
    CliEventParser, CliVendorKind, CodingCliEvent, FinishReason, McpServerInit, ParseError,
    ToolDescriptorInit,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Default)]
pub struct ClaudeParser;

impl ClaudeParser {
    pub fn new() -> Self {
        Self
    }
}

impl CliEventParser for ClaudeParser {
    fn parse_line(&mut self, line: &str) -> Result<Vec<CodingCliEvent>, ParseError> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(trimmed)?;
        Ok(normalize(&value))
    }

    fn flush(&mut self) -> Result<Vec<CodingCliEvent>, ParseError> {
        Ok(Vec::new())
    }
}

fn normalize(v: &Value) -> Vec<CodingCliEvent> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "system" => normalize_system(v),
        "stream_event" => normalize_stream_event(v),
        "tool_use" => vec![normalize_tool_use(v)],
        "tool_result" => vec![normalize_tool_result(v)],
        "result" => vec![normalize_result(v)],
        _ => vec![CodingCliEvent::RawVendorEvent {
            vendor: CliVendorKind::Claude,
            payload: v.clone(),
        }],
    }
}

fn normalize_system(v: &Value) -> Vec<CodingCliEvent> {
    let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
    match subtype {
        "init" => {
            #[derive(Deserialize)]
            struct Tool {
                #[serde(default)]
                name: String,
                #[serde(default)]
                description: Option<String>,
            }
            #[derive(Deserialize)]
            struct Mcp {
                #[serde(default)]
                name: String,
                #[serde(default)]
                status: Option<String>,
            }
            let tools: Vec<Tool> = v
                .get("tools")
                .and_then(|t| serde_json::from_value(t.clone()).ok())
                .unwrap_or_default();
            let mcp: Vec<Mcp> = v
                .get("mcp_servers")
                .and_then(|t| serde_json::from_value(t.clone()).ok())
                .unwrap_or_default();
            vec![CodingCliEvent::SystemInit {
                tools: tools
                    .into_iter()
                    .map(|t| ToolDescriptorInit {
                        name: t.name,
                        description: t.description,
                    })
                    .collect(),
                mcp_servers: mcp
                    .into_iter()
                    .map(|m| McpServerInit {
                        name: m.name,
                        status: m.status,
                    })
                    .collect(),
                plugins: Vec::new(),
            }]
        }
        "api_retry" | "api_error_retry" => {
            let attempt = v.get("attempt").and_then(Value::as_u64).unwrap_or(0) as u32;
            let delay_ms = v.get("delay_ms").and_then(Value::as_u64).unwrap_or(0);
            let reason = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("retry")
                .to_string();
            vec![CodingCliEvent::ApiRetry {
                attempt,
                delay_ms,
                reason,
            }]
        }
        _ => vec![CodingCliEvent::RawVendorEvent {
            vendor: CliVendorKind::Claude,
            payload: v.clone(),
        }],
    }
}

/// `stream_event` envelopes carry partial assistant text deltas via
/// `.event.delta.text`. Other stream events (start, stop) are passed
/// through as raw to keep the schema honest.
fn normalize_stream_event(v: &Value) -> Vec<CodingCliEvent> {
    if let Some(text) = v
        .pointer("/event/delta/text")
        .and_then(Value::as_str)
    {
        return vec![CodingCliEvent::AssistantTextDelta {
            text: text.to_string(),
        }];
    }
    if let Some(text) = v
        .pointer("/delta/text")
        .and_then(Value::as_str)
    {
        return vec![CodingCliEvent::AssistantTextDelta {
            text: text.to_string(),
        }];
    }
    vec![CodingCliEvent::RawVendorEvent {
        vendor: CliVendorKind::Claude,
        payload: v.clone(),
    }]
}

fn normalize_tool_use(v: &Value) -> CodingCliEvent {
    let id = v
        .get("id")
        .or_else(|| v.get("tool_use_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let input = v.get("input").cloned().unwrap_or(Value::Null);
    CodingCliEvent::ToolCallStarted {
        tool_call_id: id,
        name,
        input,
    }
}

fn normalize_tool_result(v: &Value) -> CodingCliEvent {
    let id = v
        .get("tool_use_id")
        .or_else(|| v.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let error = v
        .get("error")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let output = v.get("content").or_else(|| v.get("output")).cloned();
    CodingCliEvent::ToolCallFinished {
        tool_call_id: id,
        output,
        error,
    }
}

fn normalize_result(v: &Value) -> CodingCliEvent {
    let result_text = v.get("result").and_then(Value::as_str).map(|s| s.to_string());
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    CodingCliEvent::RunFinished {
        reason: if is_error {
            FinishReason::ProcessError
        } else {
            FinishReason::Completed
        },
        result_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_init() {
        let mut p = ClaudeParser::new();
        let line = r#"{"type":"system","subtype":"init","tools":[{"name":"Bash","description":"Run shell"},{"name":"Read"}],"mcp_servers":[{"name":"linear","status":"connected"}]}"#;
        let events = p.parse_line(line).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            CodingCliEvent::SystemInit { tools, mcp_servers, .. } => {
                assert_eq!(tools.len(), 2);
                assert_eq!(tools[0].name, "Bash");
                assert_eq!(mcp_servers.len(), 1);
            }
            ev => panic!("expected SystemInit, got {ev:?}"),
        }
    }

    #[test]
    fn parses_text_delta() {
        let mut p = ClaudeParser::new();
        let line = r#"{"type":"stream_event","event":{"delta":{"text":"Hello"}}}"#;
        let events = p.parse_line(line).unwrap();
        assert!(matches!(
            &events[0],
            CodingCliEvent::AssistantTextDelta { text } if text == "Hello"
        ));
    }

    #[test]
    fn parses_tool_use_and_result() {
        let mut p = ClaudeParser::new();
        let line1 = r#"{"type":"tool_use","id":"toolu_01","name":"Read","input":{"path":"a.rs"}}"#;
        let line2 = r#"{"type":"tool_result","tool_use_id":"toolu_01","content":"file contents"}"#;
        let ev1 = p.parse_line(line1).unwrap();
        let ev2 = p.parse_line(line2).unwrap();
        assert!(matches!(&ev1[0], CodingCliEvent::ToolCallStarted { name, .. } if name == "Read"));
        assert!(matches!(&ev2[0], CodingCliEvent::ToolCallFinished { error: None, .. }));
    }

    #[test]
    fn parses_result_envelope() {
        let mut p = ClaudeParser::new();
        let line = r#"{"type":"result","result":"All done","is_error":false}"#;
        let ev = p.parse_line(line).unwrap();
        assert!(matches!(
            &ev[0],
            CodingCliEvent::RunFinished {
                reason: FinishReason::Completed,
                result_text: Some(t)
            } if t == "All done"
        ));
    }

    #[test]
    fn unknown_type_passes_through_as_raw() {
        let mut p = ClaudeParser::new();
        let line = r#"{"type":"new_event_we_dont_know_yet","payload":42}"#;
        let ev = p.parse_line(line).unwrap();
        assert!(matches!(&ev[0], CodingCliEvent::RawVendorEvent { .. }));
    }

    #[test]
    fn empty_line_is_ignored() {
        let mut p = ClaudeParser::new();
        assert!(p.parse_line("").unwrap().is_empty());
        assert!(p.parse_line("   ").unwrap().is_empty());
    }
}
