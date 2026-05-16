//! Typed configuration for [`WhatsAppProvider`](crate::WhatsAppProvider).
//!
//! Parsed from the `config` field of a
//! [`ChannelSpec`](atomr_agents_channel_core::ChannelSpec). Field names
//! mirror the canonical WhatsApp Business Cloud API surface: a phone
//! number id (path component of the send URL), a long-lived access
//! token, the app secret used to verify webhook signatures, the channel
//! id this provider is bound to so inbound webhooks can be tagged with
//! it, and an optional Graph API base for testing against fakes.

use atomr_agents_channel_core::ChannelId;
use serde::{Deserialize, Serialize};

/// Default Graph API base. WhatsApp Cloud API lives under v18.0.
pub const DEFAULT_API_BASE: &str = "https://graph.facebook.com/v18.0";

/// Strongly-typed config. Parsed from the channel spec's `config`
/// `serde_json::Value`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    /// The phone number id (a numeric string assigned by Meta). Used
    /// as a path component in the send URL: `{api_base}/{phone_number_id}/messages`.
    pub phone_number_id: String,

    /// Bearer token used on every outbound + media request.
    pub access_token: String,

    /// App secret used to verify the `X-Hub-Signature-256` header on
    /// inbound webhooks via HMAC-SHA256.
    pub app_secret: String,

    /// The channel id this provider is bound to. Inbound webhooks
    /// arrive without channel context, so we stamp them with this.
    pub default_channel_id: ChannelId,

    /// Graph API base. Defaults to [`DEFAULT_API_BASE`].
    #[serde(default)]
    pub api_base: Option<String>,
}

impl WhatsAppConfig {
    /// Resolved API base — caller-supplied or [`DEFAULT_API_BASE`].
    pub fn api_base(&self) -> &str {
        self.api_base.as_deref().unwrap_or(DEFAULT_API_BASE)
    }
}
