//! Streaming partial-JSON parser.
//!
//! Feed token deltas; the parser emits the most-recent best-effort
//! `Value` parse after each chunk. Useful when a model is mid-stream
//! and the caller wants to render fields as they finalize.

use atomr_agents_core::{Result, Value};

#[derive(Default)]
pub struct StreamingPartialJsonParser {
    buffer: String,
}

impl StreamingPartialJsonParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of raw output. Returns the most-recent partial
    /// parse if one can be produced; `Ok(None)` when not enough has
    /// arrived yet.
    pub fn feed(&mut self, chunk: &str) -> Result<Option<Value>> {
        self.buffer.push_str(chunk);
        Ok(try_parse_partial(&self.buffer))
    }

    pub fn finish(self) -> Result<Value> {
        match try_parse_partial(&self.buffer) {
            Some(v) => Ok(v),
            None => Err(atomr_agents_core::AgentError::Tool(
                "streaming json: no parseable content".into(),
            )),
        }
    }
}

/// Best-effort partial parser. Walks the buffer balancing braces
/// and brackets; returns the longest prefix that parses cleanly.
fn try_parse_partial(buf: &str) -> Option<Value> {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Try the full buffer first.
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Some(v);
    }
    // Walk back, attempting to close at the last balanced point.
    let mut depth_obj = 0i32;
    let mut depth_arr = 0i32;
    let mut in_string = false;
    let mut last_close = None;
    let bytes = trimmed.as_bytes();
    let mut prev = 0u8;
    for (i, c) in bytes.iter().enumerate() {
        if in_string {
            if *c == b'"' && prev != b'\\' {
                in_string = false;
            }
            prev = *c;
            continue;
        }
        match *c {
            b'"' => in_string = true,
            b'{' => depth_obj += 1,
            b'}' => {
                depth_obj -= 1;
                if depth_obj == 0 && depth_arr == 0 {
                    last_close = Some(i + 1);
                }
            }
            b'[' => depth_arr += 1,
            b']' => {
                depth_arr -= 1;
                if depth_obj == 0 && depth_arr == 0 {
                    last_close = Some(i + 1);
                }
            }
            _ => {}
        }
        prev = *c;
    }
    if let Some(end) = last_close {
        if let Ok(v) = serde_json::from_str(&trimmed[..end]) {
            return Some(v);
        }
    }
    // Try repairing by closing open structures at the current point.
    let mut repaired = trimmed.to_string();
    while depth_obj > 0 {
        repaired.push('}');
        depth_obj -= 1;
    }
    while depth_arr > 0 {
        repaired.push(']');
        depth_arr -= 1;
    }
    serde_json::from_str(&repaired).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_partial_object_after_first_field() {
        let mut p = StreamingPartialJsonParser::new();
        let v = p.feed(r#"{"name": "Alice""#).unwrap().unwrap();
        assert_eq!(v["name"], "Alice");
    }

    #[test]
    fn refines_value_as_more_arrives() {
        let mut p = StreamingPartialJsonParser::new();
        let _ = p.feed(r#"{"items": [1, 2"#).unwrap();
        let v = p.feed(r#", 3]}"#).unwrap().unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn finish_returns_final_value() {
        let mut p = StreamingPartialJsonParser::new();
        let _ = p.feed(r#"{"k":"v"}"#).unwrap();
        assert_eq!(p.finish().unwrap()["k"], "v");
    }
}
