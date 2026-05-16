//! WhatsApp Business Cloud API provider for atomr-agents channels.
//!
//! Implements [`atomr_agents_channel_core::ChannelProvider`] against
//! Meta's Cloud API:
//!
//! * Outbound `send()` posts to `{api_base}/{phone_number_id}/messages`
//!   with a bearer-authed `messaging_product=whatsapp` JSON body.
//! * `fetch_media()` resolves a media id to a short-lived download URL
//!   then streams the bytes.
//! * `verify_webhook()` validates the `X-Hub-Signature-256` header via
//!   constant-time HMAC-SHA256 against the app secret.
//! * `parse_webhook()` walks the `entry[].changes[].value.messages[]`
//!   tree and lifts text + media messages into [`InboundMessage`]s;
//!   unknown types are silently skipped.
//!
//! WhatsApp is a webhook-driven provider, so [`ChannelProvider::start`]
//! spawns a no-op task that polls the cooperative `stop` flag — no
//! gateway, no long-poll.
//!
//! ## Quick start
//!
//! ```
//! use atomr_agents_channel_provider_whatsapp::WhatsAppProvider;
//! use serde_json::json;
//!
//! let provider = WhatsAppProvider::from_value(json!({
//!     "phone_number_id": "1234567890",
//!     "access_token": "EAAB...",
//!     "app_secret": "shhh",
//!     "default_channel_id": "channel-wa-prod",
//! })).expect("valid config");
//! assert_eq!(provider.kind().as_str(), "whatsapp");
//! ```

#![forbid(unsafe_code)]

mod client;
mod config;
mod parse;
mod webhook;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_channel_core::{
    Capabilities, ChannelError, ChannelProvider, InboundMessage, OutboundMessage,
    ProviderAck, ProviderHandle, ProviderKind, Result,
};
use bytes::Bytes;
use reqwest::Client;
use tokio::sync::mpsc;

pub use config::{WhatsAppConfig, DEFAULT_API_BASE};

/// WhatsApp Business Cloud API provider. Construct via
/// [`WhatsAppProvider::new`] or [`WhatsAppProvider::from_value`].
pub struct WhatsAppProvider {
    config: WhatsAppConfig,
    http: Client,
}

impl WhatsAppProvider {
    /// Wrap a typed config behind the [`ChannelProvider`] trait object.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(config: WhatsAppConfig) -> Arc<dyn ChannelProvider> {
        Arc::new(Self {
            config,
            http: Client::new(),
        })
    }

    /// Parse the channel spec's `config` JSON into [`WhatsAppConfig`]
    /// then build the provider. Used by `channel-harness` as the
    /// contract entry point.
    pub fn from_value(config: serde_json::Value) -> Result<Arc<dyn ChannelProvider>> {
        let config: WhatsAppConfig = serde_json::from_value(config)
            .map_err(|e| ChannelError::config(format!("whatsapp config: {e}")))?;
        Ok(Self::new(config))
    }
}

#[async_trait]
impl ChannelProvider for WhatsAppProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::WhatsApp
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            text: true,
            attachments: true,
            voice: false,
            reactions: false,
            typing: false,
            threads_native: false,
        }
    }

    async fn start(&self, _inbound_tx: mpsc::Sender<InboundMessage>) -> Result<ProviderHandle> {
        // WhatsApp is webhook-driven: nothing to do here beyond honor
        // the stop flag so the harness can join the task cleanly.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = stop.clone();
        let join = tokio::spawn(async move {
            loop {
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        Ok(ProviderHandle::new(stop, join))
    }

    async fn send(&self, msg: OutboundMessage) -> Result<ProviderAck> {
        client::send(&self.http, &self.config, msg).await
    }

    async fn fetch_media(&self, media_ref: &str) -> Result<Bytes> {
        client::fetch_media(&self.http, &self.config, media_ref).await
    }

    fn verify_webhook(&self, headers: &http::HeaderMap, body: &[u8]) -> Result<()> {
        webhook::verify(headers, body, self.config.app_secret.as_bytes())
    }

    fn parse_webhook(
        &self,
        _headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<InboundMessage>> {
        parse::parse_body(body, &self.config.default_channel_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_channel_core::{ChannelId, MessageContent};
    use http::{HeaderMap, HeaderValue};
    use serde_json::json;

    fn cfg() -> WhatsAppConfig {
        WhatsAppConfig {
            phone_number_id: "111".into(),
            access_token: "tok".into(),
            app_secret: "secret".into(),
            default_channel_id: ChannelId::from("channel-wa"),
            api_base: None,
        }
    }

    #[test]
    fn from_value_parses_config() {
        let provider = WhatsAppProvider::from_value(json!({
            "phone_number_id": "111",
            "access_token": "tok",
            "app_secret": "secret",
            "default_channel_id": "channel-wa",
        }))
        .expect("config parses");
        assert_eq!(provider.kind(), ProviderKind::WhatsApp);
        let caps = provider.capabilities();
        assert!(caps.text);
        assert!(caps.attachments);
        assert!(!caps.voice);
    }

    #[test]
    fn from_value_rejects_missing_fields() {
        let result = WhatsAppProvider::from_value(json!({
            "phone_number_id": "111",
        }));
        match result {
            Ok(_) => panic!("expected error for missing fields"),
            Err(ChannelError::Config(_)) => {}
            Err(other) => panic!("expected Config error, got {other:?}"),
        }
    }

    fn sign(body: &[u8], secret: &[u8]) -> String {
        format!("sha256={}", webhook::compute_signature(secret, body))
    }

    fn signed_headers(body: &[u8], secret: &[u8]) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            webhook::SIGNATURE_HEADER,
            HeaderValue::from_str(&sign(body, secret)).unwrap(),
        );
        h
    }

    #[test]
    fn hmac_verify_happy_path() {
        let provider = WhatsAppProvider::new(cfg());
        let body = br#"{"entry":[]}"#;
        let headers = signed_headers(body, b"secret");
        provider
            .verify_webhook(&headers, body)
            .expect("signature must verify");
    }

    #[test]
    fn hmac_verify_rejects_bad_signature() {
        let provider = WhatsAppProvider::new(cfg());
        let body = br#"{"entry":[]}"#;
        let mut headers = signed_headers(body, b"secret");
        // Tamper: replace last char of the hex sig.
        let raw = headers
            .get(webhook::SIGNATURE_HEADER)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let mut bytes = raw.into_bytes();
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        headers.insert(
            webhook::SIGNATURE_HEADER,
            HeaderValue::from_bytes(&bytes).unwrap(),
        );
        let err = provider
            .verify_webhook(&headers, body)
            .expect_err("tampered sig must fail");
        assert!(matches!(err, ChannelError::WebhookVerify(_)));
    }

    fn empty_headers() -> HeaderMap {
        HeaderMap::new()
    }

    #[test]
    fn parse_webhook_text_message() {
        let provider = WhatsAppProvider::new(cfg());
        let body = serde_json::to_vec(&json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551234567",
                            "id": "wamid.abc",
                            "timestamp": "1700000000",
                            "type": "text",
                            "text": { "body": "hello" }
                        }]
                    }
                }]
            }]
        }))
        .unwrap();
        let out = provider.parse_webhook(&empty_headers(), &body).unwrap();
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m.peer.as_str(), "15551234567");
        assert_eq!(m.provider_msg_id, "wamid.abc");
        match &m.content {
            MessageContent::Text { text } => assert_eq!(text, "hello"),
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(m.channel_id.as_str(), "channel-wa");
        assert_eq!(m.received_at.timestamp(), 1_700_000_000);
    }

    #[test]
    fn parse_webhook_image_attachment() {
        let provider = WhatsAppProvider::new(cfg());
        let body = serde_json::to_vec(&json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551234567",
                            "id": "wamid.img",
                            "timestamp": "1700000001",
                            "type": "image",
                            "image": {
                                "id": "media-xyz",
                                "mime_type": "image/png",
                                "caption": "look"
                            }
                        }]
                    }
                }]
            }]
        }))
        .unwrap();
        let out = provider.parse_webhook(&empty_headers(), &body).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0].content {
            MessageContent::Attachment {
                media_ref,
                mime,
                caption,
            } => {
                assert_eq!(media_ref, "media-xyz");
                assert_eq!(mime, "image/png");
                assert_eq!(caption.as_deref(), Some("look"));
            }
            other => panic!("expected attachment, got {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_skips_unknown_type() {
        let provider = WhatsAppProvider::new(cfg());
        let body = serde_json::to_vec(&json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "15551234567",
                            "id": "wamid.r",
                            "timestamp": "1700000002",
                            "type": "reaction",
                            "reaction": { "message_id": "x", "emoji": "👍" }
                        }]
                    }
                }]
            }]
        }))
        .unwrap();
        let out = provider.parse_webhook(&empty_headers(), &body).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn parse_webhook_handles_no_messages_array() {
        let provider = WhatsAppProvider::new(cfg());
        let body = serde_json::to_vec(&json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "statuses": [{ "id": "wamid.x", "status": "delivered" }]
                    }
                }]
            }]
        }))
        .unwrap();
        let out = provider.parse_webhook(&empty_headers(), &body).unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn start_returns_handle_that_stops_cleanly() {
        let provider = WhatsAppProvider::new(cfg());
        let (tx, _rx) = mpsc::channel(8);
        let handle = provider.start(tx).await.expect("start ok");
        handle.signal_stop();
        // Joining must not hang.
        tokio::time::timeout(Duration::from_millis(500), handle.join)
            .await
            .expect("join in time")
            .expect("task ok");
    }
}
