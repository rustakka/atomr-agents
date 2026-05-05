//! Basic parsers.

use std::marker::PhantomData;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result, Value};
use serde::de::DeserializeOwned;

use crate::Parser;

// --------------------------------------------------------------------
// JsonParser — `Value`
// --------------------------------------------------------------------

#[derive(Default)]
pub struct JsonParser;

#[async_trait]
impl Parser<Value> for JsonParser {
    async fn parse(&self, raw: &str) -> Result<Value> {
        let raw = strip_code_fence(raw);
        serde_json::from_str(&raw).map_err(|e| AgentError::Tool(format!("json parse: {e}")))
    }
    fn format_instructions(&self) -> String {
        "Respond with a single valid JSON value.".into()
    }
}

// --------------------------------------------------------------------
// JsonSchemaParser — `Value`, validated against a JSON-Schema-shaped
// guard. (Lightweight: only checks `type`, `required`, top-level
// property types — enough for tests; production users plug in a real
// JSON-Schema validator.)
// --------------------------------------------------------------------

pub struct JsonSchemaParser {
    pub schema: Value,
}

impl JsonSchemaParser {
    pub fn new(schema: Value) -> Self {
        Self { schema }
    }
}

#[async_trait]
impl Parser<Value> for JsonSchemaParser {
    async fn parse(&self, raw: &str) -> Result<Value> {
        let v: Value = JsonParser.parse(raw).await?;
        validate(&self.schema, &v)?;
        Ok(v)
    }
    fn format_instructions(&self) -> String {
        format!(
            "Respond with JSON matching this schema:\n```\n{}\n```",
            serde_json::to_string_pretty(&self.schema).unwrap_or_default()
        )
    }
}

fn validate(schema: &Value, v: &Value) -> Result<()> {
    let want_type = schema.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if want_type == "object" {
        if !v.is_object() {
            return Err(AgentError::Tool("expected object".into()));
        }
        if let Some(req) = schema.get("required").and_then(|r| r.as_array()) {
            for r in req {
                let key = r.as_str().unwrap_or("");
                if v.get(key).is_none() {
                    return Err(AgentError::Tool(format!("missing required field '{key}'")));
                }
            }
        }
    } else if want_type == "array" && !v.is_array() {
        return Err(AgentError::Tool("expected array".into()));
    } else if want_type == "string" && !v.is_string() {
        return Err(AgentError::Tool("expected string".into()));
    } else if want_type == "integer" && !v.is_i64() {
        return Err(AgentError::Tool("expected integer".into()));
    }
    Ok(())
}

// --------------------------------------------------------------------
// SchemaParser<T> — Pydantic-style: deserialize into a typed Rust
// struct, with format-instructions surfacing the schema description.
// --------------------------------------------------------------------

pub struct SchemaParser<T> {
    pub instructions: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T> SchemaParser<T> {
    pub fn new(instructions: impl Into<String>) -> Self {
        Self {
            instructions: instructions.into(),
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<T: DeserializeOwned + Send + Sync + 'static> Parser<T> for SchemaParser<T> {
    async fn parse(&self, raw: &str) -> Result<T> {
        let raw = strip_code_fence(raw);
        serde_json::from_str(&raw).map_err(|e| AgentError::Tool(format!("schema parse: {e}")))
    }
    fn format_instructions(&self) -> String {
        self.instructions.clone()
    }
}

// --------------------------------------------------------------------
// EnumParser
// --------------------------------------------------------------------

pub struct EnumParser {
    pub variants: Vec<String>,
}

impl EnumParser {
    pub fn new<I: IntoIterator<Item = impl Into<String>>>(variants: I) -> Self {
        Self {
            variants: variants.into_iter().map(Into::into).collect(),
        }
    }
}

#[async_trait]
impl Parser<String> for EnumParser {
    async fn parse(&self, raw: &str) -> Result<String> {
        let raw = raw.trim();
        for v in &self.variants {
            if v.eq_ignore_ascii_case(raw) {
                return Ok(v.clone());
            }
        }
        Err(AgentError::Tool(format!(
            "{raw:?} not one of {:?}",
            self.variants
        )))
    }
    fn format_instructions(&self) -> String {
        format!("Reply with exactly one of: {}", self.variants.join(", "))
    }
}

// --------------------------------------------------------------------
// CommaListParser
// --------------------------------------------------------------------

pub struct CommaListParser;

#[async_trait]
impl Parser<Vec<String>> for CommaListParser {
    async fn parse(&self, raw: &str) -> Result<Vec<String>> {
        Ok(raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }
    fn format_instructions(&self) -> String {
        "Reply with a comma-separated list of values.".into()
    }
}

// --------------------------------------------------------------------
// XmlParser — naive: extracts top-level <tag>contents</tag>
// pairs into a flat object.
// --------------------------------------------------------------------

pub struct XmlParser;

#[async_trait]
impl Parser<Value> for XmlParser {
    async fn parse(&self, raw: &str) -> Result<Value> {
        let mut out = serde_json::Map::new();
        let mut idx = 0;
        let bytes = raw.as_bytes();
        while idx < bytes.len() {
            // find '<'
            while idx < bytes.len() && bytes[idx] != b'<' {
                idx += 1;
            }
            if idx >= bytes.len() {
                break;
            }
            let tag_start = idx + 1;
            // find '>'
            let mut tag_end = tag_start;
            while tag_end < bytes.len() && bytes[tag_end] != b'>' {
                tag_end += 1;
            }
            if tag_end >= bytes.len() {
                break;
            }
            let tag = &raw[tag_start..tag_end];
            if tag.starts_with('/') {
                idx = tag_end + 1;
                continue;
            }
            let close = format!("</{tag}>");
            if let Some(close_pos) = raw[tag_end..].find(&close) {
                let body_start = tag_end + 1;
                let body_end = tag_end + close_pos;
                let body = &raw[body_start..body_end];
                out.insert(tag.to_string(), Value::String(body.trim().to_string()));
                idx = body_end + close.len();
            } else {
                idx = tag_end + 1;
            }
        }
        if out.is_empty() {
            return Err(AgentError::Tool("xml parse: no tags found".into()));
        }
        Ok(Value::Object(out))
    }
    fn format_instructions(&self) -> String {
        "Wrap each field in matching XML tags, e.g. <name>Alice</name>.".into()
    }
}

// --------------------------------------------------------------------
// YamlParser — accepts a small `key: value` dialect (one pair per
// line, no nesting). Sufficient for unit tests; users can plug in a
// full YAML crate behind a feature flag later.
// --------------------------------------------------------------------

pub struct YamlParser;

#[async_trait]
impl Parser<Value> for YamlParser {
    async fn parse(&self, raw: &str) -> Result<Value> {
        let mut out = serde_json::Map::new();
        for line in raw.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = l.split_once(':') {
                let k = k.trim();
                let v = v.trim();
                if k.is_empty() {
                    continue;
                }
                out.insert(k.to_string(), Value::String(v.to_string()));
            }
        }
        if out.is_empty() {
            return Err(AgentError::Tool("yaml parse: no key/value pairs".into()));
        }
        Ok(Value::Object(out))
    }
    fn format_instructions(&self) -> String {
        "Reply with one key: value pair per line.".into()
    }
}

fn strip_code_fence(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("```") {
        let mut lines: Vec<&str> = s.lines().collect();
        if lines.first().map(|l| l.starts_with("```")).unwrap_or(false) {
            lines.remove(0);
        }
        if lines.last().map(|l| l.trim() == "```").unwrap_or(false) {
            lines.pop();
        }
        return lines.join("\n");
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Plan {
        title: String,
        steps: Vec<String>,
    }

    #[tokio::test]
    async fn json_strips_fence() {
        let p = JsonParser;
        let v = p.parse("```json\n{\"a\":1}\n```").await.unwrap();
        assert_eq!(v, serde_json::json!({"a": 1}));
    }

    #[tokio::test]
    async fn schema_parser_round_trips_typed_struct() {
        let p: SchemaParser<Plan> = SchemaParser::new("...");
        let v = p.parse(r#"{"title":"x","steps":["a","b"]}"#).await.unwrap();
        assert_eq!(v.title, "x");
        assert_eq!(v.steps.len(), 2);
    }

    #[tokio::test]
    async fn schema_validation_catches_missing_field() {
        let p = JsonSchemaParser::new(serde_json::json!({
            "type": "object",
            "required": ["a", "b"]
        }));
        let r = p.parse(r#"{"a":1}"#).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn enum_parser_normalizes_case() {
        let p = EnumParser::new(["yes", "no"]);
        assert_eq!(p.parse("YES").await.unwrap(), "yes");
        assert!(p.parse("maybe").await.is_err());
    }

    #[tokio::test]
    async fn comma_list_parses_with_trim() {
        let p = CommaListParser;
        assert_eq!(p.parse("a, b,c , ").await.unwrap(), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn xml_parser_extracts_top_level_tags() {
        let p = XmlParser;
        let v = p.parse("<name>Alice</name><city>NYC</city>").await.unwrap();
        assert_eq!(v["name"], "Alice");
        assert_eq!(v["city"], "NYC");
    }

    #[tokio::test]
    async fn yaml_parser_simple_dialect() {
        let p = YamlParser;
        let v = p.parse("name: Alice\nrole: admin\n").await.unwrap();
        assert_eq!(v["name"], "Alice");
    }
}
