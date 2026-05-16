use serde::{Deserialize, Serialize};

use crate::ids::ChannelId;

/// Which transport this channel is backed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    WhatsApp,
    Signal,
    Discord,
    Memory,
    Custom(String),
}

impl ProviderKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WhatsApp => "whatsapp",
            Self::Signal => "signal",
            Self::Discord => "discord",
            Self::Memory => "memory",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kinds of payloads a channel will *accept on outbound*.
///
/// The orchestrator validates `OutboundMessage::content` against these
/// before calling `provider.send` and rejects with
/// [`ChannelError::CapabilityDenied`](crate::ChannelError::CapabilityDenied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default = "Capabilities::default_text")]
    pub text: bool,
    #[serde(default)]
    pub attachments: bool,
    #[serde(default)]
    pub voice: bool,
    #[serde(default)]
    pub reactions: bool,
    #[serde(default)]
    pub typing: bool,
    #[serde(default)]
    pub threads_native: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::text_only()
    }
}

impl Capabilities {
    fn default_text() -> bool {
        true
    }

    pub fn text_only() -> Self {
        Self {
            text: true,
            attachments: false,
            voice: false,
            reactions: false,
            typing: false,
            threads_native: false,
        }
    }

    pub fn full() -> Self {
        Self {
            text: true,
            attachments: true,
            voice: true,
            reactions: true,
            typing: true,
            threads_native: true,
        }
    }
}

/// Persisted description of a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSpec {
    pub id: ChannelId,
    pub kind: ProviderKind,
    /// Provider-specific configuration. Each provider crate parses this
    /// into its own typed config struct.
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Human-readable description for UI listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ChannelSpec {
    pub fn new(id: impl Into<ChannelId>, kind: ProviderKind) -> Self {
        Self {
            id: id.into(),
            kind,
            config: serde_json::Value::Null,
            capabilities: Capabilities::default(),
            description: None,
        }
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }

    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.capabilities = caps;
        self
    }
}
