//! Outbound HTTP for WhatsApp Cloud API.
//!
//! Two operations live here:
//!
//! 1. **Send.** `POST {api_base}/{phone_number_id}/messages` with a
//!    `messaging_product=whatsapp` JSON body. The response carries
//!    `{"messages":[{"id":"wamid..."}]}` from which we lift the
//!    provider message id.
//! 2. **Fetch media.** A two-step dance: first resolve the media id
//!    to a short-lived download URL (`GET {api_base}/{media_id}`),
//!    then download the bytes from that URL (still bearer-authed).

use atomr_agents_channel_core::{
    ChannelError, MessageContent, OutboundMessage, ProviderAck, Result,
};
use bytes::Bytes;
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

use crate::config::WhatsAppConfig;

/// Build the message body for an outbound send. Returns one or more
/// JSON payloads — `Mixed` content with both an attachment and text
/// becomes multiple sends.
pub(crate) fn build_payloads(msg: &OutboundMessage) -> Vec<Value> {
    let mut out = Vec::new();
    push_payloads(&msg.content, msg.peer.as_str(), &mut out);
    if out.is_empty() {
        // Defensive: a totally empty content tree still emits a
        // (probably-rejected) empty text body, but providers should
        // never construct one. We return Vec so callers can decide.
    }
    out
}

fn push_payloads(content: &MessageContent, to: &str, out: &mut Vec<Value>) {
    match content {
        MessageContent::Text { text } => {
            out.push(json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": { "body": text },
            }));
        }
        MessageContent::Attachment {
            media_ref,
            mime,
            caption,
        } => {
            let kind = attachment_kind(mime);
            let mut payload = json!({ "id": media_ref });
            if let Some(c) = caption {
                payload["caption"] = Value::String(c.clone());
            }
            out.push(json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": kind,
                kind: payload,
            }));
        }
        MessageContent::Mixed { parts } => {
            // Collect leading text into a single body when possible,
            // then append attachments individually.
            let mut text_buf = String::new();
            let mut attachments = Vec::new();
            for p in parts {
                match p {
                    MessageContent::Text { text } => {
                        if !text_buf.is_empty() {
                            text_buf.push('\n');
                        }
                        text_buf.push_str(text);
                    }
                    MessageContent::Attachment { .. } => attachments.push(p),
                    MessageContent::Mixed { .. } => push_payloads(p, to, out),
                }
            }
            if !text_buf.is_empty() {
                out.push(json!({
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": "text",
                    "text": { "body": text_buf },
                }));
            }
            for att in attachments {
                push_payloads(att, to, out);
            }
        }
    }
}

/// Map a mime type to a WhatsApp media kind: `image` / `audio` /
/// `video` / `document`. Stickers go through `image` from the *send*
/// side too (Meta routes them by mime).
pub(crate) fn attachment_kind(mime: &str) -> &'static str {
    let head = mime.split('/').next().unwrap_or("");
    match head {
        "image" => "image",
        "audio" => "audio",
        "video" => "video",
        _ => "document",
    }
}

/// POST one payload. Returns the wamid for the first ack.
pub(crate) async fn send_one(
    http: &Client,
    config: &WhatsAppConfig,
    payload: &Value,
) -> Result<String> {
    let url = format!("{}/{}/messages", config.api_base(), config.phone_number_id);
    let resp = http
        .post(&url)
        .bearer_auth(&config.access_token)
        .json(payload)
        .send()
        .await
        .map_err(|e| ChannelError::transport(format!("whatsapp send: {e}")))?;
    let status = resp.status();
    if status.is_success() {
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::provider(format!("whatsapp ack json: {e}")))?;
        body.get("messages")
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.first())
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                ChannelError::provider(format!(
                    "whatsapp send: missing messages[0].id in {body}"
                ))
            })
    } else {
        let body = resp.text().await.unwrap_or_default();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Err(ChannelError::transport(format!(
                "whatsapp send {status}: {body}"
            )))
        } else {
            Err(ChannelError::provider(format!(
                "whatsapp send {status}: {body}"
            )))
        }
    }
}

/// Drive the full send: build payloads, post each, return the ack for
/// the *last* one (so callers see the provider id of the trailing
/// part).
pub(crate) async fn send(
    http: &Client,
    config: &WhatsAppConfig,
    msg: OutboundMessage,
) -> Result<ProviderAck> {
    let payloads = build_payloads(&msg);
    if payloads.is_empty() {
        return Err(ChannelError::provider("whatsapp send: empty content"));
    }
    let mut last_id = String::new();
    for payload in &payloads {
        last_id = send_one(http, config, payload).await?;
    }
    Ok(ProviderAck {
        provider_msg_id: last_id,
        sent_at: Utc::now(),
    })
}

/// Two-step media fetch: resolve the media id to a download URL, then
/// stream the bytes.
pub(crate) async fn fetch_media(
    http: &Client,
    config: &WhatsAppConfig,
    media_ref: &str,
) -> Result<Bytes> {
    let resolve_url = format!("{}/{}", config.api_base(), media_ref);
    let meta: Value = http
        .get(&resolve_url)
        .bearer_auth(&config.access_token)
        .send()
        .await
        .map_err(|e| ChannelError::transport(format!("whatsapp media resolve: {e}")))?
        .error_for_status()
        .map_err(|e| ChannelError::provider(format!("whatsapp media resolve: {e}")))?
        .json()
        .await
        .map_err(|e| ChannelError::provider(format!("whatsapp media resolve json: {e}")))?;
    let download_url = meta
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ChannelError::provider("whatsapp media: missing url"))?;
    let bytes = http
        .get(download_url)
        .bearer_auth(&config.access_token)
        .send()
        .await
        .map_err(|e| ChannelError::transport(format!("whatsapp media download: {e}")))?
        .error_for_status()
        .map_err(|e| ChannelError::provider(format!("whatsapp media download: {e}")))?
        .bytes()
        .await
        .map_err(|e| ChannelError::transport(format!("whatsapp media bytes: {e}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_channel_core::{ChannelId, MessageContent, PeerId, ThreadId};

    fn outbound(content: MessageContent) -> OutboundMessage {
        OutboundMessage {
            channel_id: ChannelId::from("c"),
            thread_id: ThreadId::from("t"),
            peer: PeerId::from("15551234567"),
            content,
            reply_to: None,
            idempotency_key: "k".into(),
        }
    }

    #[test]
    fn build_text_payload() {
        let payloads = build_payloads(&outbound(MessageContent::text("hi")));
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["messaging_product"], "whatsapp");
        assert_eq!(payloads[0]["to"], "15551234567");
        assert_eq!(payloads[0]["type"], "text");
        assert_eq!(payloads[0]["text"]["body"], "hi");
    }

    #[test]
    fn build_image_payload() {
        let payloads = build_payloads(&outbound(MessageContent::Attachment {
            media_ref: "mid-1".into(),
            mime: "image/jpeg".into(),
            caption: Some("cap".into()),
        }));
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["type"], "image");
        assert_eq!(payloads[0]["image"]["id"], "mid-1");
        assert_eq!(payloads[0]["image"]["caption"], "cap");
    }

    #[test]
    fn build_video_payload() {
        let payloads = build_payloads(&outbound(MessageContent::Attachment {
            media_ref: "mid-2".into(),
            mime: "video/mp4".into(),
            caption: None,
        }));
        assert_eq!(payloads[0]["type"], "video");
        assert_eq!(payloads[0]["video"]["id"], "mid-2");
        assert!(payloads[0]["video"].get("caption").is_none());
    }

    #[test]
    fn build_audio_payload() {
        let payloads = build_payloads(&outbound(MessageContent::Attachment {
            media_ref: "mid-3".into(),
            mime: "audio/ogg".into(),
            caption: None,
        }));
        assert_eq!(payloads[0]["type"], "audio");
    }

    #[test]
    fn build_document_payload_for_unknown_mime() {
        let payloads = build_payloads(&outbound(MessageContent::Attachment {
            media_ref: "mid-4".into(),
            mime: "application/pdf".into(),
            caption: None,
        }));
        assert_eq!(payloads[0]["type"], "document");
    }

    #[test]
    fn build_mixed_text_then_image() {
        let payloads = build_payloads(&outbound(MessageContent::Mixed {
            parts: vec![
                MessageContent::text("hello"),
                MessageContent::Attachment {
                    media_ref: "mid-5".into(),
                    mime: "image/png".into(),
                    caption: None,
                },
            ],
        }));
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0]["type"], "text");
        assert_eq!(payloads[0]["text"]["body"], "hello");
        assert_eq!(payloads[1]["type"], "image");
    }
}
