//! Outbound Discord REST API client.
//!
//! Two surfaces:
//!
//! - [`send_message`] — POST `/channels/{channel_id}/messages` with a
//!   bearer-style `Authorization: Bot {token}` header.
//! - [`fetch_media`] — GET an arbitrary Discord attachment URL. Discord
//!   CDN URLs are public; no token is sent on these requests.

use atomr_agents_channel_core::{ChannelError, MessageContent, OutboundMessage, ProviderAck, Result};
use bytes::Bytes;
use chrono::Utc;
use serde_json::{json, Value};

/// Resolve the Discord-native channel id this outbound message should be
/// sent to.
///
/// Resolution order:
/// 1. `msg.reply_to` of the form `"channel:msg"` — use the `channel`
///    portion. (Discord lets you reply with `message_reference` but this
///    crate's surface only carries the channel id explicitly.)
/// 2. The provider's configured `discord_channel_id`.
///
/// Returns `Err(ChannelError::Provider)` when neither is set.
pub(crate) fn resolve_target_channel<'a>(
    msg: &'a OutboundMessage,
    default: Option<&'a str>,
) -> Result<&'a str> {
    if let Some(reply_to) = msg.reply_to.as_deref() {
        if let Some((channel, _)) = reply_to.split_once(':') {
            return Ok(channel);
        }
    }
    default.ok_or_else(|| {
        ChannelError::provider("discord: no target channel (no reply_to, no discord_channel_id)")
    })
}

/// Build the JSON body for a `POST /channels/{c}/messages` request.
///
/// Exposed for tests so we don't need a live HTTP server.
pub(crate) fn build_send_body(content: &MessageContent) -> Value {
    match content {
        MessageContent::Text { text } => json!({ "content": text }),
        MessageContent::Attachment {
            media_ref,
            mime,
            caption,
        } => {
            let caption = caption.clone().unwrap_or_default();
            if mime.starts_with("image/") {
                json!({
                    "content": caption,
                    "embeds": [{
                        "image": { "url": media_ref }
                    }]
                })
            } else {
                let combined = if caption.is_empty() {
                    media_ref.clone()
                } else {
                    format!("{caption}\n{media_ref}")
                };
                json!({ "content": combined })
            }
        }
        MessageContent::Mixed { .. } => json!({ "content": content.as_text() }),
    }
}

/// POST a message to a Discord channel. Returns the provider-native id
/// reported in the response.
pub(crate) async fn send_message(
    client: &reqwest::Client,
    api_base: &str,
    bot_token: &str,
    channel_id: &str,
    msg: &OutboundMessage,
) -> Result<ProviderAck> {
    let url = format!("{api_base}/channels/{channel_id}/messages");
    let body = build_send_body(&msg.content);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bot {bot_token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::transport(format!("discord send: {e}")))?;

    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ChannelError::transport(format!("discord send: read body: {e}")))?;

    if !status.is_success() {
        return Err(ChannelError::provider(format!(
            "discord send returned {status}"
        )));
    }

    let parsed: Value = serde_json::from_slice(&bytes)
        .map_err(|e| ChannelError::provider(format!("discord send: bad JSON: {e}")))?;
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::provider("discord send: response missing id"))?
        .to_string();
    Ok(ProviderAck {
        provider_msg_id: id,
        sent_at: Utc::now(),
    })
}

/// Fetch raw bytes from a Discord CDN URL.
pub(crate) async fn fetch_media(client: &reqwest::Client, media_ref: &str) -> Result<Bytes> {
    let resp = client
        .get(media_ref)
        .send()
        .await
        .map_err(|e| ChannelError::transport(format!("discord fetch_media: {e}")))?;
    if !resp.status().is_success() {
        return Err(ChannelError::provider(format!(
            "discord fetch_media: {}",
            resp.status()
        )));
    }
    resp.bytes()
        .await
        .map_err(|e| ChannelError::transport(format!("discord fetch_media: read body: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_channel_core::{ChannelId, MessageContent, OutboundMessage, PeerId, ThreadId};

    fn outbound(content: MessageContent, reply_to: Option<&str>) -> OutboundMessage {
        OutboundMessage {
            channel_id: ChannelId::from("c"),
            thread_id: ThreadId::from("t"),
            peer: PeerId::from("p"),
            content,
            reply_to: reply_to.map(|s| s.into()),
            idempotency_key: "k".into(),
        }
    }

    #[test]
    fn build_send_body_text() {
        let body = build_send_body(&MessageContent::text("hi"));
        assert_eq!(body, json!({ "content": "hi" }));
    }

    #[test]
    fn build_send_body_image_attachment() {
        let body = build_send_body(&MessageContent::Attachment {
            media_ref: "https://x/cat.png".into(),
            mime: "image/png".into(),
            caption: Some("look".into()),
        });
        assert_eq!(
            body,
            json!({
                "content": "look",
                "embeds": [{"image": {"url": "https://x/cat.png"}}]
            })
        );
    }

    #[test]
    fn build_send_body_non_image_attachment() {
        let body = build_send_body(&MessageContent::Attachment {
            media_ref: "https://x/data.pdf".into(),
            mime: "application/pdf".into(),
            caption: Some("report".into()),
        });
        assert_eq!(body, json!({ "content": "report\nhttps://x/data.pdf" }));
    }

    #[test]
    fn resolve_uses_reply_to_channel_prefix() {
        let m = outbound(MessageContent::text("hi"), Some("9999:msg1"));
        assert_eq!(resolve_target_channel(&m, Some("default")).unwrap(), "9999");
    }

    #[test]
    fn resolve_falls_back_to_default() {
        let m = outbound(MessageContent::text("hi"), None);
        assert_eq!(
            resolve_target_channel(&m, Some("default-c")).unwrap(),
            "default-c"
        );
    }

    #[test]
    fn resolve_errors_when_no_target() {
        let m = outbound(MessageContent::text("hi"), None);
        let err = resolve_target_channel(&m, None).unwrap_err();
        assert!(matches!(err, ChannelError::Provider(_)));
    }
}
