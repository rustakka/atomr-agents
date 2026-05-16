use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{ChannelError, Result};
use crate::ids::{ChannelId, PeerId, ThreadId};
use crate::spec::Capabilities;

/// A piece of content that flows across a channel.
///
/// Attachments carry **provider-native references** (a WhatsApp media
/// id, a Discord attachment URL, a signal-cli file path) — the
/// provider is responsible for resolving them via
/// [`ChannelProvider::fetch_media`](crate::provider::ChannelProvider::fetch_media)
/// when bytes are needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageContent {
    Text { text: String },
    Attachment {
        media_ref: String,
        mime: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    Mixed { parts: Vec<MessageContent> },
}

impl MessageContent {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text { text: s.into() }
    }

    /// Return the textual content of the message, joining any nested
    /// `Mixed` parts and ignoring attachments.
    pub fn as_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Attachment { caption, .. } => caption.clone().unwrap_or_default(),
            Self::Mixed { parts } => parts
                .iter()
                .map(|p| p.as_text())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Short, single-line summary suitable for event payloads / logs.
    pub fn summary(&self) -> String {
        let t = self.as_text();
        if t.is_empty() {
            match self {
                Self::Attachment { mime, .. } => format!("<attachment: {mime}>"),
                Self::Mixed { .. } => "<mixed>".into(),
                Self::Text { .. } => String::new(),
            }
        } else if t.len() <= 80 {
            t
        } else {
            let mut s: String = t.chars().take(77).collect();
            s.push_str("...");
            s
        }
    }

    /// Returns `Err(CapabilityDenied)` if any of the variants in this
    /// content tree is disabled on `caps`.
    pub fn check_capabilities(&self, caps: &Capabilities) -> Result<()> {
        match self {
            Self::Text { .. } => {
                if !caps.text {
                    return Err(ChannelError::CapabilityDenied("text"));
                }
                Ok(())
            }
            Self::Attachment { .. } => {
                if !caps.attachments {
                    return Err(ChannelError::CapabilityDenied("attachments"));
                }
                Ok(())
            }
            Self::Mixed { parts } => {
                for p in parts {
                    p.check_capabilities(caps)?;
                }
                Ok(())
            }
        }
    }
}

/// One inbound message coming off a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub peer: PeerId,
    pub provider_msg_id: String,
    pub content: MessageContent,
    pub received_at: DateTime<Utc>,
    /// Original payload, kept for replay / debugging.
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// One outbound message the harness wants the provider to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub peer: PeerId,
    pub content: MessageContent,
    /// Optional provider-native message id to reply-to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Caller-supplied idempotency key. The harness uses it to
    /// short-circuit duplicate sends.
    pub idempotency_key: String,
}

/// Result of a successful provider send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAck {
    pub provider_msg_id: String,
    pub sent_at: DateTime<Utc>,
}

/// Internal append-only message log entry. Stored per thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessageRecord {
    pub thread_id: ThreadId,
    /// Stable identifier within the harness (uuid).
    pub id: String,
    /// `"inbound"` or `"outbound"`.
    pub direction: Direction,
    pub content: MessageContent,
    /// Provider-native id, if known (inbound: provider_msg_id from
    /// webhook/gateway; outbound: filled in once the send acks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_msg_id: Option<String>,
    /// Outbound only — original idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}
