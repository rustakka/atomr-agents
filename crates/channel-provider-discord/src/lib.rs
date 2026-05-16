//! Discord channel provider — Gateway WebSocket (default) and
//! Interactions webhook (Ed25519-verified) modes.
//!
//! See [`DiscordConfig`] for the JSON shape parsed from the channel
//! spec and the crate-level README for usage examples.

#![forbid(unsafe_code)]

mod config;
mod parse;
mod rest;
mod verify;

#[cfg(feature = "gateway")]
mod gateway;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_channel_core::{
    Capabilities, ChannelError, ChannelProvider, InboundMessage, OutboundMessage, ProviderAck,
    ProviderHandle, ProviderKind, Result,
};
use bytes::Bytes;
use tokio::sync::mpsc;

pub use crate::config::{
    DiscordConfig, DiscordMode, DEFAULT_API_BASE, DEFAULT_GATEWAY_URL, DEFAULT_INTENTS,
};

/// Discord channel provider.
///
/// Construct with [`DiscordProvider::new`] (typed config) or
/// [`DiscordProvider::from_value`] (parses the same JSON shape stored
/// on `ChannelSpec::config`).
pub struct DiscordProvider {
    config: DiscordConfig,
    http: reqwest::Client,
}

impl DiscordProvider {
    /// Build a provider from a parsed [`DiscordConfig`]. Returns it as
    /// the dyn-trait the harness consumes.
    pub fn new(config: DiscordConfig) -> Arc<dyn ChannelProvider> {
        Arc::new(Self {
            config,
            http: reqwest::Client::new(),
        })
    }

    /// Parse a JSON config and build a provider.
    pub fn from_value(config: serde_json::Value) -> Result<Arc<dyn ChannelProvider>> {
        let cfg = DiscordConfig::from_value(config)?;
        Ok(Self::new(cfg))
    }

    /// Test/inspection helper — read-only access to the parsed config.
    pub fn config(&self) -> &DiscordConfig {
        &self.config
    }
}

#[async_trait]
impl ChannelProvider for DiscordProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Discord
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            text: true,
            attachments: true,
            voice: false,
            reactions: true,
            typing: false,
            threads_native: false,
        }
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<ProviderHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = stop.clone();

        match self.config.mode {
            DiscordMode::Gateway => {
                #[cfg(feature = "gateway")]
                {
                    let config = self.config.clone();
                    let join = tokio::spawn(async move {
                        if let Err(e) = gateway::run(config, inbound_tx, stop_task).await {
                            tracing::warn!(target: "discord", "gateway task ended: {e}");
                        }
                    });
                    Ok(ProviderHandle::new(stop, join))
                }
                #[cfg(not(feature = "gateway"))]
                {
                    let _ = (inbound_tx, stop_task);
                    Err(ChannelError::Unsupported("discord gateway feature disabled"))
                }
            }
            DiscordMode::InteractionsWebhook => {
                // No long-running task: inbound flows through
                // `parse_webhook` when the host calls it from its HTTP
                // handler. We still spawn a tiny task so the
                // ProviderHandle shape stays uniform with other
                // providers (a `stop` flag the harness can flip).
                let _ = inbound_tx;
                let join = tokio::spawn(async move {
                    while !stop_task.load(Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                });
                Ok(ProviderHandle::new(stop, join))
            }
        }
    }

    async fn send(&self, msg: OutboundMessage) -> Result<ProviderAck> {
        let default = self.config.discord_channel_id.as_deref();
        let target = rest::resolve_target_channel(&msg, default)?;
        rest::send_message(&self.http, self.config.api_base(), &self.config.bot_token, target, &msg)
            .await
    }

    async fn fetch_media(&self, media_ref: &str) -> Result<Bytes> {
        rest::fetch_media(&self.http, media_ref).await
    }

    fn verify_webhook(&self, headers: &http::HeaderMap, body: &[u8]) -> Result<()> {
        let pk = self
            .config
            .public_key
            .as_deref()
            .ok_or_else(|| ChannelError::webhook_verify("public_key not configured"))?;
        verify::verify(headers, body, pk)
    }

    fn parse_webhook(
        &self,
        _headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<InboundMessage>> {
        parse::parse_webhook_body(body, &self.config.default_channel_id)
    }
}
