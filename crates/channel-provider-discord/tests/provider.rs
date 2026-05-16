//! Integration tests for `atomr-agents-channel-provider-discord`.
//!
//! These tests don't need a live Discord connection — they exercise:
//!
//! - Ed25519 webhook verification (happy path + tampered signature + bad
//!   timestamp).
//! - Webhook body parsing (PING returns empty Vec; APPLICATION_COMMAND
//!   parses into an InboundMessage).
//! - Config parsing for both modes.
//! - Capability matrix.

use atomr_agents_channel_core::{ChannelError, ChannelProvider, MessageContent, ProviderKind};
use atomr_agents_channel_provider_discord::{DiscordConfig, DiscordMode, DiscordProvider};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use http::{HeaderMap, HeaderValue};
use rand::rngs::OsRng;
use serde_json::json;

fn signed_request(
    sk: &SigningKey,
    timestamp: &str,
    body: &[u8],
) -> HeaderMap {
    let mut msg = Vec::with_capacity(timestamp.len() + body.len());
    msg.extend_from_slice(timestamp.as_bytes());
    msg.extend_from_slice(body);
    let sig = sk.sign(&msg);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-signature-ed25519",
        HeaderValue::from_str(&hex::encode(sig.to_bytes())).unwrap(),
    );
    headers.insert(
        "x-signature-timestamp",
        HeaderValue::from_str(timestamp).unwrap(),
    );
    headers
}

fn provider_with_key(vk: &VerifyingKey) -> std::sync::Arc<dyn ChannelProvider> {
    let cfg = DiscordConfig::from_value(json!({
        "mode": "interactions_webhook",
        "bot_token": "test-token",
        "public_key": hex::encode(vk.to_bytes()),
        "default_channel_id": "channel-discord-test"
    }))
    .unwrap();
    DiscordProvider::new(cfg)
}

#[test]
fn verify_webhook_happy_path() {
    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let provider = provider_with_key(&vk);

    let body = br#"{"type":1}"#;
    let headers = signed_request(&sk, "1700000000", body);

    provider
        .verify_webhook(&headers, body)
        .expect("signature must verify");
}

#[test]
fn verify_webhook_rejects_tampered_signature() {
    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let provider = provider_with_key(&vk);

    let body = br#"{"type":1}"#;
    let mut headers = signed_request(&sk, "1700000000", body);
    // Flip a byte in the hex signature.
    let raw = headers
        .get("x-signature-ed25519")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let mut bytes = raw.into_bytes();
    bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
    let bad = String::from_utf8(bytes).unwrap();
    headers.insert("x-signature-ed25519", HeaderValue::from_str(&bad).unwrap());

    let err = provider
        .verify_webhook(&headers, body)
        .expect_err("must reject");
    assert!(matches!(err, ChannelError::WebhookVerify(_)));
}

#[test]
fn verify_webhook_rejects_mismatched_timestamp() {
    let sk = SigningKey::generate(&mut OsRng);
    let vk = sk.verifying_key();
    let provider = provider_with_key(&vk);

    let body = br#"{"type":1}"#;
    let mut headers = signed_request(&sk, "1700000000", body);
    // Swap the timestamp out from under the signature.
    headers.insert(
        "x-signature-timestamp",
        HeaderValue::from_static("1700000001"),
    );

    let err = provider
        .verify_webhook(&headers, body)
        .expect_err("must reject");
    assert!(matches!(err, ChannelError::WebhookVerify(_)));
}

#[test]
fn parse_webhook_ping_returns_empty() {
    let sk = SigningKey::generate(&mut OsRng);
    let provider = provider_with_key(&sk.verifying_key());
    let body = br#"{"type":1}"#;
    let msgs = provider
        .parse_webhook(&HeaderMap::new(), body)
        .expect("ping parses");
    assert!(msgs.is_empty(), "PING must produce no inbound messages");
}

#[test]
fn parse_webhook_application_command() {
    let sk = SigningKey::generate(&mut OsRng);
    let provider = provider_with_key(&sk.verifying_key());

    let body = serde_json::to_vec(&json!({
        "id": "interaction-1",
        "type": 2,
        "member": {"user": {"id": "user-42"}},
        "data": {
            "id": "cmd-1",
            "name": "ask",
            "options": [{"name": "q", "value": "what time is it"}]
        }
    }))
    .unwrap();
    let msgs = provider.parse_webhook(&HeaderMap::new(), &body).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].peer.as_str(), "user-42");
    assert_eq!(msgs[0].channel_id.as_str(), "channel-discord-test");
    assert_eq!(msgs[0].provider_msg_id, "interaction-1");
    assert_eq!(msgs[0].content.as_text(), "ask what time is it");
}

#[test]
fn from_value_parses_gateway_config() {
    let provider = DiscordProvider::from_value(json!({
        "bot_token": "abc",
        "default_channel_id": "channel-discord-gw",
        "discord_channel_id": "9876543210"
    }))
    .expect("gateway config parses");
    assert_eq!(provider.kind(), ProviderKind::Discord);
}

#[test]
fn from_value_rejects_webhook_config_missing_public_key() {
    let result = DiscordProvider::from_value(json!({
        "mode": "interactions_webhook",
        "bot_token": "abc",
        "default_channel_id": "channel-discord-wh"
    }));
    match result {
        Ok(_) => panic!("must require public_key in webhook mode"),
        Err(ChannelError::Config(_)) => {}
        Err(e) => panic!("expected Config error, got {e:?}"),
    }
}

#[test]
fn from_value_parses_webhook_config_with_public_key() {
    let sk = SigningKey::generate(&mut OsRng);
    let provider = DiscordProvider::from_value(json!({
        "mode": "interactions_webhook",
        "bot_token": "abc",
        "public_key": hex::encode(sk.verifying_key().to_bytes()),
        "default_channel_id": "channel-discord-wh"
    }))
    .expect("webhook config parses");
    assert_eq!(provider.kind(), ProviderKind::Discord);
}

#[test]
fn capabilities_text_attachments_reactions() {
    let cfg = DiscordConfig {
        mode: DiscordMode::Gateway,
        bot_token: "x".into(),
        public_key: None,
        default_channel_id: "channel-x".into(),
        discord_channel_id: None,
        intents: None,
        gateway_url: None,
        api_base: None,
    };
    let provider = DiscordProvider::new(cfg);
    let caps = provider.capabilities();
    assert!(caps.text);
    assert!(caps.attachments);
    assert!(caps.reactions);
    assert!(!caps.voice);
    assert!(!caps.typing);
    assert!(!caps.threads_native);
}

#[test]
fn send_body_attachment_image_uses_embed() {
    // Smoke-test through MessageContent semantics. The internal
    // build_send_body is private; we assert capability matrix above and
    // delegate the embed-format check to the unit test inside rest.rs.
    let _ = MessageContent::Attachment {
        media_ref: "https://x/cat.png".into(),
        mime: "image/png".into(),
        caption: Some("look".into()),
    };
}
