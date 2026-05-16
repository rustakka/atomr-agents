//! JSON-RPC framing helpers for the `signal-cli` daemon.
//!
//! signal-cli speaks line-delimited JSON-RPC 2.0 over a Unix socket or
//! TCP connection. This module owns the wire shapes (requests,
//! responses, inbound notifications) plus the `parse_envelope` that
//! turns a `receive` notification into an
//! [`InboundMessage`](atomr_agents_channel_core::InboundMessage).

use atomr_agents_channel_core::{
    ChannelError, ChannelId, InboundMessage, MessageContent, OutboundMessage, PeerId, Result,
    ThreadId,
};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

/// JSON-RPC response envelope (success or error) for one request id.
///
/// We deserialize responses into this shape inside `send()` to keep
/// the success/error split type-checked.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct JsonRpcResponse {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct JsonRpcError {
    #[allow(dead_code)]
    pub code: i64,
    pub message: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub data: Option<Value>,
}

/// Build the JSON-RPC `send` request frame for an
/// [`OutboundMessage`].
///
/// Returns `(id, frame)` — caller writes `frame + "\n"` and awaits the
/// matching response by `id`.
pub(crate) fn build_send_request(
    id: &str,
    account: &str,
    msg: &OutboundMessage,
) -> Result<Value> {
    let mut params = serde_json::Map::new();
    params.insert("account".into(), Value::String(account.into()));
    params.insert(
        "recipient".into(),
        Value::Array(vec![Value::String(msg.peer.as_str().to_string())]),
    );

    let text = collect_text(&msg.content);
    if !text.is_empty() {
        params.insert("message".into(), Value::String(text));
    }
    let attachments = collect_attachments(&msg.content);
    if !attachments.is_empty() {
        params.insert(
            "attachments".into(),
            Value::Array(attachments.into_iter().map(Value::String).collect()),
        );
    }

    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "send",
        "params": Value::Object(params),
    }))
}

fn collect_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { text } => text.clone(),
        MessageContent::Attachment { caption, .. } => caption.clone().unwrap_or_default(),
        MessageContent::Mixed { parts } => {
            let mut out = Vec::new();
            for p in parts {
                let t = match p {
                    MessageContent::Text { text } => text.clone(),
                    MessageContent::Attachment { caption, .. } => {
                        caption.clone().unwrap_or_default()
                    }
                    MessageContent::Mixed { .. } => collect_text(p),
                };
                if !t.is_empty() {
                    out.push(t);
                }
            }
            out.join("\n")
        }
    }
}

fn collect_attachments(content: &MessageContent) -> Vec<String> {
    let mut acc = Vec::new();
    push_attachments(content, &mut acc);
    acc
}

fn push_attachments(content: &MessageContent, acc: &mut Vec<String>) {
    match content {
        MessageContent::Text { .. } => {}
        MessageContent::Attachment { media_ref, .. } => acc.push(media_ref.clone()),
        MessageContent::Mixed { parts } => {
            for p in parts {
                push_attachments(p, acc);
            }
        }
    }
}

/// Parse a `receive` notification envelope into an
/// [`InboundMessage`].
///
/// `envelope` is the value of `params.envelope` from a signal-cli
/// notification:
///
/// ```json
/// {
///   "source":"+15559876543","sourceUuid":"abc-def","timestamp":1700000000456,
///   "dataMessage":{"message":"hello","timestamp":1700000000456,
///                  "attachments":[{"id":"att1","contentType":"image/jpeg"}]}
/// }
/// ```
///
/// Returns `Ok(None)` if the envelope has no parsable content (e.g.
/// it's a typing/receipt notification rather than a data message).
pub(crate) fn parse_envelope(
    envelope: &Value,
    default_channel_id: &ChannelId,
) -> Result<Option<InboundMessage>> {
    let peer_src = envelope
        .get("sourceUuid")
        .and_then(Value::as_str)
        .or_else(|| envelope.get("source").and_then(Value::as_str))
        .unwrap_or("");
    if peer_src.is_empty() {
        return Err(ChannelError::webhook_parse(
            "envelope has no source/sourceUuid",
        ));
    }
    let peer = PeerId::from(peer_src);

    let envelope_ts = envelope.get("timestamp").and_then(Value::as_i64);

    let data = match envelope.get("dataMessage") {
        Some(v) if !v.is_null() => v,
        _ => return Ok(None),
    };

    let data_ts = data.get("timestamp").and_then(Value::as_i64);
    let ts_millis = data_ts.or(envelope_ts);

    let text = data
        .get("message")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_default();

    let attachments: Vec<&Value> = data
        .get("attachments")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    let content: MessageContent = if attachments.is_empty() {
        if text.is_empty() {
            return Ok(None);
        }
        MessageContent::text(text)
    } else if attachments.len() == 1 {
        let only = attachments[0];
        let media_ref = only
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| only.get("filename").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let mime = only
            .get("contentType")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream")
            .to_string();
        let caption = if text.is_empty() { None } else { Some(text) };
        MessageContent::Attachment {
            media_ref,
            mime,
            caption,
        }
    } else {
        let mut parts: Vec<MessageContent> = Vec::with_capacity(attachments.len() + 1);
        if !text.is_empty() {
            parts.push(MessageContent::text(text));
        }
        for a in attachments {
            let media_ref = a
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| a.get("filename").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let mime = a
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string();
            parts.push(MessageContent::Attachment {
                media_ref,
                mime,
                caption: None,
            });
        }
        MessageContent::Mixed { parts }
    };

    let provider_msg_id = ts_millis
        .map(|t| t.to_string())
        .unwrap_or_else(|| Utc::now().timestamp_millis().to_string());

    let received_at = ts_millis
        .and_then(|t| Utc.timestamp_millis_opt(t).single())
        .unwrap_or_else(Utc::now);

    let channel_id = default_channel_id.clone();
    let thread_id = ThreadId::for_peer(&channel_id, &peer);

    Ok(Some(InboundMessage {
        channel_id,
        thread_id,
        peer,
        provider_msg_id,
        content,
        received_at,
        raw: envelope.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_channel_core::MessageContent;
    use serde_json::json;

    #[test]
    fn parses_text_envelope() {
        let env = json!({
            "source": "+15559876543",
            "sourceUuid": "abc-def",
            "timestamp": 1700000000456_i64,
            "dataMessage": {
                "message": "hello there",
                "timestamp": 1700000000456_i64
            }
        });
        let cid = ChannelId::from("signal:demo");
        let m = parse_envelope(&env, &cid).unwrap().expect("inbound");
        assert_eq!(m.peer.as_str(), "abc-def");
        assert_eq!(m.provider_msg_id, "1700000000456");
        assert_eq!(m.channel_id, cid);
        match m.content {
            MessageContent::Text { text } => assert_eq!(text, "hello there"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn parses_attachment_envelope() {
        let env = json!({
            "source": "+15559876543",
            "timestamp": 1700000000456_i64,
            "dataMessage": {
                "message": "",
                "attachments": [
                    {"id": "att-1", "contentType": "image/jpeg"}
                ]
            }
        });
        let cid = ChannelId::from("signal:demo");
        let m = parse_envelope(&env, &cid).unwrap().expect("inbound");
        match m.content {
            MessageContent::Attachment { media_ref, mime, caption } => {
                assert_eq!(media_ref, "att-1");
                assert_eq!(mime, "image/jpeg");
                assert!(caption.is_none());
            }
            _ => panic!("expected attachment"),
        }
        // Falls back to envelope source when sourceUuid is missing.
        assert_eq!(m.peer.as_str(), "+15559876543");
    }

    #[test]
    fn parses_mixed_envelope() {
        let env = json!({
            "sourceUuid": "abc",
            "timestamp": 1700000000456_i64,
            "dataMessage": {
                "message": "see attached",
                "attachments": [
                    {"id": "a", "contentType": "image/jpeg"},
                    {"id": "b", "contentType": "image/png"}
                ]
            }
        });
        let cid = ChannelId::from("signal:demo");
        let m = parse_envelope(&env, &cid).unwrap().expect("inbound");
        match m.content {
            MessageContent::Mixed { parts } => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(parts[0], MessageContent::Text { .. }));
                assert!(matches!(parts[1], MessageContent::Attachment { .. }));
                assert!(matches!(parts[2], MessageContent::Attachment { .. }));
            }
            _ => panic!("expected mixed"),
        }
    }

    #[test]
    fn envelope_with_no_data_message_returns_none() {
        let env = json!({
            "source": "+1",
            "timestamp": 1,
            "typingMessage": {"action": "STARTED"}
        });
        let cid = ChannelId::from("signal:demo");
        assert!(parse_envelope(&env, &cid).unwrap().is_none());
    }

    #[test]
    fn empty_text_no_attachment_returns_none() {
        let env = json!({
            "source": "+1",
            "timestamp": 1,
            "dataMessage": {"message": "", "timestamp": 1}
        });
        let cid = ChannelId::from("signal:demo");
        assert!(parse_envelope(&env, &cid).unwrap().is_none());
    }

    #[test]
    fn build_send_request_contains_expected_fields() {
        use atomr_agents_channel_core::{OutboundMessage, PeerId, ThreadId};
        let out = OutboundMessage {
            channel_id: ChannelId::from("signal:demo"),
            thread_id: ThreadId::from("t"),
            peer: PeerId::from("+15551234567"),
            content: MessageContent::text("hello"),
            reply_to: None,
            idempotency_key: "k1".into(),
        };
        let frame = build_send_request("req-1", "+15550000001", &out).unwrap();
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], "req-1");
        assert_eq!(frame["method"], "send");
        assert_eq!(frame["params"]["account"], "+15550000001");
        assert_eq!(frame["params"]["recipient"][0], "+15551234567");
        assert_eq!(frame["params"]["message"], "hello");
        assert!(frame["params"].get("attachments").is_none());
    }

    #[test]
    fn build_send_request_with_attachment() {
        use atomr_agents_channel_core::{OutboundMessage, PeerId, ThreadId};
        let out = OutboundMessage {
            channel_id: ChannelId::from("signal:demo"),
            thread_id: ThreadId::from("t"),
            peer: PeerId::from("+15551234567"),
            content: MessageContent::Attachment {
                media_ref: "/tmp/cat.jpg".into(),
                mime: "image/jpeg".into(),
                caption: Some("look".into()),
            },
            reply_to: None,
            idempotency_key: "k2".into(),
        };
        let frame = build_send_request("req-2", "+15550000001", &out).unwrap();
        assert_eq!(frame["params"]["message"], "look");
        assert_eq!(frame["params"]["attachments"][0], "/tmp/cat.jpg");
    }
}
