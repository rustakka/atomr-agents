//! WhatsApp inbound webhook payload parser.
//!
//! The Meta-delivered shape is deeply nested:
//!
//! ```json
//! {
//!   "entry": [{
//!     "changes": [{
//!       "value": {
//!         "messages": [{
//!           "from": "15551234567",
//!           "id": "wamid.abc",
//!           "timestamp": "1700000000",
//!           "type": "text",
//!           "text": { "body": "hello" }
//!         }]
//!       }
//!     }]
//!   }]
//! }
//! ```
//!
//! This module walks `entry[].changes[].value.messages[]` and lifts
//! each message into an [`InboundMessage`]. Unsupported types
//! (reactions, system, …) are silently skipped — they are *not* an
//! error.

use atomr_agents_channel_core::{
    ChannelError, ChannelId, InboundMessage, MessageContent, PeerId, Result, ThreadId,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Parse a *verified* JSON body into zero or more [`InboundMessage`]s.
pub(crate) fn parse_body(body: &[u8], default_channel_id: &ChannelId) -> Result<Vec<InboundMessage>> {
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| ChannelError::webhook_parse(format!("invalid json: {e}")))?;

    let mut out = Vec::new();
    let Some(entries) = root.get("entry").and_then(Value::as_array) else {
        return Ok(out);
    };
    for entry in entries {
        let Some(changes) = entry.get("changes").and_then(Value::as_array) else {
            continue;
        };
        for change in changes {
            let Some(value) = change.get("value") else {
                continue;
            };
            let Some(messages) = value.get("messages").and_then(Value::as_array) else {
                continue;
            };
            for msg in messages {
                if let Some(inbound) = lift_message(msg, default_channel_id) {
                    out.push(inbound);
                }
            }
        }
    }
    Ok(out)
}

fn lift_message(msg: &Value, default_channel_id: &ChannelId) -> Option<InboundMessage> {
    let from = msg.get("from").and_then(Value::as_str)?;
    let id = msg.get("id").and_then(Value::as_str)?;
    let kind = msg.get("type").and_then(Value::as_str)?;

    let content = match kind {
        "text" => {
            let body = msg
                .get("text")
                .and_then(|t| t.get("body"))
                .and_then(Value::as_str)
                .unwrap_or("");
            MessageContent::text(body)
        }
        "image" | "audio" | "video" | "document" | "sticker" => {
            let obj = msg.get(kind)?;
            let media_ref = obj.get("id").and_then(Value::as_str)?.to_owned();
            let mime = obj
                .get("mime_type")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| default_mime_for(kind).to_owned());
            let caption = obj
                .get("caption")
                .and_then(Value::as_str)
                .map(str::to_owned);
            MessageContent::Attachment {
                media_ref,
                mime,
                caption,
            }
        }
        _ => return None,
    };

    let received_at = msg
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        .unwrap_or_else(Utc::now);

    let peer = PeerId::from(from);
    let thread_id = ThreadId::for_peer(default_channel_id, &peer);

    Some(InboundMessage {
        channel_id: default_channel_id.clone(),
        thread_id,
        peer,
        provider_msg_id: id.to_owned(),
        content,
        received_at,
        raw: msg.clone(),
    })
}

fn default_mime_for(kind: &str) -> &'static str {
    match kind {
        "image" => "image/jpeg",
        "audio" => "audio/ogg",
        "video" => "video/mp4",
        "sticker" => "image/webp",
        // "document" and anything else routed here.
        _ => "application/octet-stream",
    }
}
