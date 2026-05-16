//! Parsers for Discord payloads — Gateway `MESSAGE_CREATE` events and
//! Interactions webhook bodies.
//!
//! These are pure functions: they take JSON in and produce
//! [`InboundMessage`]s out (or `None` when the payload should be
//! filtered, e.g. bot messages or unsupported interaction types).

use atomr_agents_channel_core::{
    ChannelError, ChannelId, InboundMessage, MessageContent, PeerId, Result, ThreadId,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Parse the `d` payload of a `MESSAGE_CREATE` Gateway event into an
/// [`InboundMessage`].
///
/// Returns `Ok(None)` if the message should be dropped:
/// - the author is a bot, or
/// - `expect_channel` is `Some(c)` and `d.channel_id != c`.
#[cfg_attr(not(any(feature = "gateway", test)), allow(dead_code))]
pub(crate) fn parse_message_create(
    d: &Value,
    channel_id: &ChannelId,
    expect_channel: Option<&str>,
) -> Result<Option<InboundMessage>> {
    let obj = d
        .as_object()
        .ok_or_else(|| ChannelError::webhook_parse("MESSAGE_CREATE d must be an object"))?;

    // Filter bot messages first so they never reach the agent.
    let author = obj
        .get("author")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ChannelError::webhook_parse("MESSAGE_CREATE missing author"))?;
    if author
        .get("bot")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(None);
    }

    let discord_channel_id = obj
        .get("channel_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::webhook_parse("MESSAGE_CREATE missing channel_id"))?;
    if let Some(expected) = expect_channel {
        if discord_channel_id != expected {
            return Ok(None);
        }
    }

    let provider_msg_id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::webhook_parse("MESSAGE_CREATE missing id"))?
        .to_string();

    let author_id = author
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::webhook_parse("MESSAGE_CREATE author missing id"))?;
    let peer = PeerId::from(author_id);

    let received_at = obj
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(|s| DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc)))
        .transpose()
        .map_err(|e| ChannelError::webhook_parse(format!("invalid timestamp: {e}")))?
        .unwrap_or_else(Utc::now);

    let text = obj
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let attachments = obj
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let content = build_content(&text, attachments)?;

    Ok(Some(InboundMessage {
        channel_id: channel_id.clone(),
        thread_id: ThreadId::for_peer(channel_id, &peer),
        peer,
        provider_msg_id,
        content,
        received_at,
        raw: d.clone(),
    }))
}

#[cfg_attr(not(any(feature = "gateway", test)), allow(dead_code))]
fn build_content(text: &str, attachments: &[Value]) -> Result<MessageContent> {
    let first_attachment = attachments.first().map(build_attachment).transpose()?;

    match (text.is_empty(), first_attachment) {
        (false, None) => Ok(MessageContent::text(text)),
        (true, Some(att)) => Ok(att),
        (false, Some(att)) => Ok(MessageContent::Mixed {
            parts: vec![MessageContent::text(text), att],
        }),
        (true, None) => Ok(MessageContent::text("")),
    }
}

#[cfg_attr(not(any(feature = "gateway", test)), allow(dead_code))]
fn build_attachment(att: &Value) -> Result<MessageContent> {
    let obj = att
        .as_object()
        .ok_or_else(|| ChannelError::webhook_parse("attachment must be an object"))?;
    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::webhook_parse("attachment missing url"))?
        .to_string();
    let mime = obj
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    Ok(MessageContent::Attachment {
        media_ref: url,
        mime,
        caption: None,
    })
}

/// Parse an Interactions webhook body into zero or more inbound messages.
///
/// Discord interaction types:
/// - `1` PING — handshake; return empty vec.
/// - `2` APPLICATION_COMMAND — slash command; the "text" is the command
///   name plus any string options.
/// - `3` MESSAGE_COMPONENT — button / select; the "text" is the
///   `custom_id`.
///
/// All other types are ignored (empty vec).
pub(crate) fn parse_webhook_body(
    body: &[u8],
    channel_id: &ChannelId,
) -> Result<Vec<InboundMessage>> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| ChannelError::webhook_parse(format!("invalid JSON: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| ChannelError::webhook_parse("interaction must be an object"))?;
    let ty = obj
        .get("type")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ChannelError::webhook_parse("interaction missing type"))?;

    if ty == 1 {
        return Ok(Vec::new());
    }
    if ty != 2 && ty != 3 {
        return Ok(Vec::new());
    }

    let provider_msg_id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::webhook_parse("interaction missing id"))?
        .to_string();

    let peer_id = obj
        .get("member")
        .and_then(|v| v.get("user"))
        .and_then(|u| u.get("id"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            obj.get("user")
                .and_then(|u| u.get("id"))
                .and_then(|v| v.as_str())
        })
        .ok_or_else(|| ChannelError::webhook_parse("interaction missing user id"))?
        .to_string();
    let peer = PeerId::from(peer_id);

    let text = if ty == 2 {
        application_command_text(obj.get("data"))
    } else {
        // MESSAGE_COMPONENT
        obj.get("data")
            .and_then(|d| d.get("custom_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let received_at = Utc::now();

    let msg = InboundMessage {
        channel_id: channel_id.clone(),
        thread_id: ThreadId::for_peer(channel_id, &peer),
        peer,
        provider_msg_id,
        content: MessageContent::text(text),
        received_at,
        raw: v,
    };
    Ok(vec![msg])
}

fn application_command_text(data: Option<&Value>) -> String {
    let Some(data) = data.and_then(|d| d.as_object()) else {
        return String::new();
    };
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let options: Vec<String> = data
        .get("options")
        .and_then(|v| v.as_array())
        .map(|opts| {
            opts.iter()
                .filter_map(|opt| {
                    let value = opt.get("value")?;
                    Some(match value {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if options.is_empty() {
        name
    } else if name.is_empty() {
        options.join(" ")
    } else {
        format!("{name} {}", options.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn channel() -> ChannelId {
        ChannelId::from("channel-discord-demo")
    }

    #[test]
    fn parse_message_create_text_only() {
        let d = json!({
            "id": "1234567890",
            "channel_id": "9876543210",
            "guild_id": "5555555555",
            "author": {"id": "1111", "username": "alice", "bot": false},
            "content": "hello bot",
            "timestamp": "2024-01-01T12:00:00.000000+00:00",
            "attachments": []
        });
        let out = parse_message_create(&d, &channel(), None).unwrap().unwrap();
        assert_eq!(out.peer.as_str(), "1111");
        assert_eq!(out.provider_msg_id, "1234567890");
        match out.content {
            MessageContent::Text { text } => assert_eq!(text, "hello bot"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn parse_message_create_attachment_only() {
        let d = json!({
            "id": "1",
            "channel_id": "c",
            "author": {"id": "u", "bot": false},
            "content": "",
            "timestamp": "2024-01-01T12:00:00.000000+00:00",
            "attachments": [{
                "id": "9999",
                "url": "https://cdn.discordapp.com/.../foo.png",
                "filename": "foo.png",
                "content_type": "image/png"
            }]
        });
        let out = parse_message_create(&d, &channel(), None).unwrap().unwrap();
        match out.content {
            MessageContent::Attachment {
                media_ref, mime, ..
            } => {
                assert_eq!(media_ref, "https://cdn.discordapp.com/.../foo.png");
                assert_eq!(mime, "image/png");
            }
            other => panic!("expected attachment, got {other:?}"),
        }
    }

    #[test]
    fn parse_message_create_mixed() {
        let d = json!({
            "id": "1",
            "channel_id": "c",
            "author": {"id": "u", "bot": false},
            "content": "look at this",
            "timestamp": "2024-01-01T12:00:00.000000+00:00",
            "attachments": [{
                "id": "1",
                "url": "https://x/foo.png",
                "content_type": "image/png"
            }]
        });
        let out = parse_message_create(&d, &channel(), None).unwrap().unwrap();
        match out.content {
            MessageContent::Mixed { parts } => assert_eq!(parts.len(), 2),
            other => panic!("expected mixed, got {other:?}"),
        }
    }

    #[test]
    fn parse_message_create_drops_bots() {
        let d = json!({
            "id": "1",
            "channel_id": "c",
            "author": {"id": "u", "bot": true},
            "content": "hello",
            "timestamp": "2024-01-01T12:00:00.000000+00:00",
            "attachments": []
        });
        let out = parse_message_create(&d, &channel(), None).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn parse_message_create_filters_channel() {
        let d = json!({
            "id": "1",
            "channel_id": "other",
            "author": {"id": "u", "bot": false},
            "content": "hello",
            "timestamp": "2024-01-01T12:00:00.000000+00:00",
            "attachments": []
        });
        let out = parse_message_create(&d, &channel(), Some("expected")).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn parse_webhook_ping_empty() {
        let body = br#"{"type": 1}"#;
        let out = parse_webhook_body(body, &channel()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn parse_webhook_application_command() {
        let body = br#"{
            "id": "interaction-id",
            "type": 2,
            "member": {"user": {"id": "user-id"}},
            "data": {
                "id": "cmd-id",
                "name": "ask",
                "options": [{"name": "q", "value": "what is the weather"}]
            }
        }"#;
        let msgs = parse_webhook_body(body, &channel()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].peer.as_str(), "user-id");
        assert_eq!(msgs[0].content.as_text(), "ask what is the weather");
    }
}
