//! Typed configuration for [`DiscordProvider`](crate::DiscordProvider).
//!
//! Parsed from the `config` field of a
//! [`ChannelSpec`](atomr_agents_channel_core::ChannelSpec). Two modes are
//! supported:
//!
//! - **Gateway** (default): the provider opens a Discord Gateway WebSocket
//!   and consumes `MESSAGE_CREATE` events. Inbound flows through the
//!   provider's spawned task.
//! - **InteractionsWebhook**: Discord POSTs interactions to a webhook
//!   endpoint exposed by the host application. Inbound flows through
//!   [`ChannelProvider::parse_webhook`](atomr_agents_channel_core::ChannelProvider::parse_webhook)
//!   after the payload has been Ed25519-verified.

use atomr_agents_channel_core::{ChannelError, ChannelId, Result};
use serde::{Deserialize, Serialize};

/// Default Discord REST API base.
pub const DEFAULT_API_BASE: &str = "https://discord.com/api/v10";

/// Default Discord Gateway URL with JSON encoding on API v10.
pub const DEFAULT_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Default intents bitmask: `GUILD_MESSAGES (1 << 9) | MESSAGE_CONTENT (1 << 15) = 33280`.
///
/// This is the minimum required to receive guild text messages with their
/// content (the content intent is a privileged intent in Discord and must
/// also be enabled in the Discord developer portal).
pub const DEFAULT_INTENTS: u64 = (1 << 9) | (1 << 15);

/// Which mode the Discord provider runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscordMode {
    /// Open a Gateway WebSocket and consume `MESSAGE_CREATE` events.
    Gateway,
    /// Receive interactions via webhook POST (Ed25519 signature
    /// verification on each request).
    InteractionsWebhook,
}

impl Default for DiscordMode {
    fn default() -> Self {
        Self::Gateway
    }
}

/// Parsed Discord provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Selection of inbound transport. Defaults to [`DiscordMode::Gateway`].
    #[serde(default)]
    pub mode: DiscordMode,

    /// Bot token. Used as `Authorization: Bot {bot_token}` on every REST
    /// call and as the `token` field in the Gateway IDENTIFY payload.
    pub bot_token: String,

    /// Hex-encoded Ed25519 application public key. Required when
    /// [`mode`](Self::mode) is [`DiscordMode::InteractionsWebhook`] so
    /// inbound interactions can be verified.
    #[serde(default)]
    pub public_key: Option<String>,

    /// The channel id this provider is bound to. Inbound events arrive
    /// without `atomr`-level channel context, so we stamp them with this.
    pub default_channel_id: ChannelId,

    /// Discord-native channel id used for both inbound filtering (drop
    /// events from other channels in Gateway mode) and outbound routing
    /// (where `send()` posts when no `reply_to` is provided).
    #[serde(default)]
    pub discord_channel_id: Option<String>,

    /// Gateway intents bitmask. Defaults to [`DEFAULT_INTENTS`].
    #[serde(default)]
    pub intents: Option<u64>,

    /// Gateway URL. Defaults to [`DEFAULT_GATEWAY_URL`].
    #[serde(default)]
    pub gateway_url: Option<String>,

    /// Discord REST API base. Defaults to [`DEFAULT_API_BASE`].
    #[serde(default)]
    pub api_base: Option<String>,
}

impl DiscordConfig {
    /// Parse the JSON config shape.
    pub fn from_value(value: serde_json::Value) -> Result<Self> {
        let cfg: Self = serde_json::from_value(value)
            .map_err(|e| ChannelError::Config(format!("discord config: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate cross-field invariants.
    pub fn validate(&self) -> Result<()> {
        if self.mode == DiscordMode::InteractionsWebhook && self.public_key.is_none() {
            return Err(ChannelError::Config(
                "discord config: public_key is required when mode = interactions_webhook"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Resolved API base — caller-supplied or [`DEFAULT_API_BASE`].
    pub fn api_base(&self) -> &str {
        self.api_base.as_deref().unwrap_or(DEFAULT_API_BASE)
    }

    /// Resolved gateway URL.
    pub fn gateway_url(&self) -> &str {
        self.gateway_url.as_deref().unwrap_or(DEFAULT_GATEWAY_URL)
    }

    /// Resolved intents bitmask.
    pub fn intents(&self) -> u64 {
        self.intents.unwrap_or(DEFAULT_INTENTS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_gateway_config() {
        let cfg = DiscordConfig::from_value(json!({
            "bot_token": "abc",
            "default_channel_id": "channel-discord-demo",
            "discord_channel_id": "9876543210"
        }))
        .unwrap();
        assert_eq!(cfg.mode, DiscordMode::Gateway);
        assert_eq!(cfg.bot_token, "abc");
        assert_eq!(cfg.discord_channel_id.as_deref(), Some("9876543210"));
        assert_eq!(cfg.intents(), DEFAULT_INTENTS);
        assert_eq!(cfg.api_base(), DEFAULT_API_BASE);
        assert_eq!(cfg.gateway_url(), DEFAULT_GATEWAY_URL);
    }

    #[test]
    fn parses_webhook_config_with_public_key() {
        let cfg = DiscordConfig::from_value(json!({
            "mode": "interactions_webhook",
            "bot_token": "abc",
            "public_key": "00".repeat(32),
            "default_channel_id": "channel-discord-demo"
        }))
        .unwrap();
        assert_eq!(cfg.mode, DiscordMode::InteractionsWebhook);
        assert!(cfg.public_key.is_some());
    }

    #[test]
    fn rejects_webhook_config_missing_public_key() {
        let err = DiscordConfig::from_value(json!({
            "mode": "interactions_webhook",
            "bot_token": "abc",
            "default_channel_id": "channel-discord-demo"
        }))
        .unwrap_err();
        assert!(matches!(err, ChannelError::Config(_)));
    }
}
