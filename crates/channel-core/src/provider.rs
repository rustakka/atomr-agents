use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::content::{InboundMessage, OutboundMessage, ProviderAck};
use crate::error::{ChannelError, Result};
use crate::spec::{Capabilities, ProviderKind};

/// Cooperative shutdown handle for a provider's long-running task.
///
/// Shape mirrors [`atomr_agents_stt_harness`'s `SessionHandle`](https://docs.rs/atomr-agents-stt-harness):
/// a `stop` flag the task polls and a `JoinHandle` for the spawned task.
/// The orchestrator owns one of these per attached channel and signals
/// shutdown by setting `stop` then awaiting `join`.
pub struct ProviderHandle {
    pub stop: Arc<AtomicBool>,
    pub join: JoinHandle<()>,
}

impl ProviderHandle {
    pub fn new(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
        Self { stop, join }
    }

    /// Convenience: signal stop. Callers still need to `await self.join`
    /// to wait for the task to finish.
    pub fn signal_stop(&self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A provider-specific transport (WhatsApp / Signal / Discord / Memory).
///
/// Implementors live in their own crates (`channel-provider-*`). Each
/// provider:
///
/// - Decides whether inbound is push (webhook → `parse_webhook`),
///   long-poll (`start` spawns a poller), or gateway (`start` opens a WS).
/// - Sends outbound via [`send`](Self::send).
/// - Verifies webhook signatures using whatever provider-specific
///   cryptography applies (HMAC-SHA256 for WhatsApp, Ed25519 for
///   Discord) — keeping crypto out of the web layer.
#[async_trait]
pub trait ChannelProvider: Send + Sync + 'static {
    fn kind(&self) -> ProviderKind;

    fn capabilities(&self) -> Capabilities;

    /// Start any long-running provider task (gateway, poller, …).
    /// Inbound messages flow through `inbound_tx`. Returns a handle
    /// the harness uses to stop / await the task.
    ///
    /// Providers that are purely webhook-driven (WhatsApp) return a
    /// no-op handle whose `join` resolves immediately when `stop` is set.
    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<ProviderHandle>;

    async fn send(&self, msg: OutboundMessage) -> Result<ProviderAck>;

    /// Resolve a provider-native media reference to bytes. Optional.
    async fn fetch_media(&self, _media_ref: &str) -> Result<Bytes> {
        Err(ChannelError::Unsupported("fetch_media"))
    }

    /// Verify a webhook payload. Headers carry signature, timestamp,
    /// and whatever else the provider needs.
    fn verify_webhook(&self, _headers: &http::HeaderMap, _body: &[u8]) -> Result<()> {
        Err(ChannelError::Unsupported("verify_webhook"))
    }

    /// Parse a *verified* webhook body into zero or more InboundMessages.
    fn parse_webhook(
        &self,
        _headers: &http::HeaderMap,
        _body: &[u8],
    ) -> Result<Vec<InboundMessage>> {
        Err(ChannelError::Unsupported("parse_webhook"))
    }
}
