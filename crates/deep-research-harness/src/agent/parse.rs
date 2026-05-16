//! Helpers for parsing JSON role outputs out of (possibly
//! noisy) LLM text.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{DeepResearchError, Result};

/// Strip markdown code fences and surrounding prose from a model
/// response, then parse the first `{...}` object as the requested type.
///
/// LLMs reliably emit JSON-shaped outputs but sometimes wrap them in
/// ```` ```json ... ``` ```` fences or precede them with prose like
/// "Here's the plan: ...". This helper is forgiving of both.
pub fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    let cleaned = strip_fences(text.trim());
    // First, try direct parse — covers the well-behaved case.
    if let Ok(v) = serde_json::from_str::<T>(cleaned) {
        return Ok(v);
    }
    // Otherwise, find the first balanced { ... } and parse that.
    let candidate = first_json_object(cleaned)
        .ok_or_else(|| DeepResearchError::role(format!("no JSON object in model output: {text}")))?;
    serde_json::from_str::<T>(&candidate)
        .map_err(|e| DeepResearchError::role(format!("invalid JSON from model ({e}): {candidate}")))
}

/// Same as [`parse_json`], but returns the parsed [`Value`] without
/// imposing a target type.
#[allow(dead_code)]
pub fn parse_json_value(text: &str) -> Result<Value> {
    parse_json::<Value>(text)
}

fn strip_fences(s: &str) -> &str {
    let t = s.trim();
    // ``` (optionally with `json` after it) at the start, ``` at the end.
    let after_open = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```JSON"))
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let body = after_open.trim_start_matches('\n');
    let inner = body.strip_suffix("```").unwrap_or(body);
    inner.trim()
}

fn first_json_object(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut start: Option<usize> = None;
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(st) = start {
                        return Some(s[st..=i].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Sample {
        a: u32,
        b: String,
    }

    #[test]
    fn parses_bare_json() {
        let v: Sample = parse_json("{\"a\":1,\"b\":\"x\"}").unwrap();
        assert_eq!(v, Sample { a: 1, b: "x".into() });
    }

    #[test]
    fn parses_fenced_json() {
        let v: Sample = parse_json("```json\n{\"a\":2,\"b\":\"y\"}\n```").unwrap();
        assert_eq!(v, Sample { a: 2, b: "y".into() });
    }

    #[test]
    fn parses_embedded_json() {
        let v: Sample = parse_json("Here's the result: {\"a\":3,\"b\":\"z\"} hope it helps").unwrap();
        assert_eq!(v, Sample { a: 3, b: "z".into() });
    }

    #[test]
    fn errors_when_no_object() {
        let r: Result<Sample> = parse_json("no json here");
        assert!(r.is_err());
    }
}
