//! In-process channel provider used for tests and as a worked example.
//!
//! `InMemoryProvider` is a [`ChannelProvider`] that emits inbound
//! messages from a caller-controlled [`tokio::sync::mpsc::Sender`] (the
//! "inbox") and records outbound sends to a broadcast log a test can
//! subscribe to. No external network is involved.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::{broadcast, mpsc};

use crate::content::{InboundMessage, OutboundMessage, ProviderAck};
use crate::error::{ChannelError, Result};
use crate::ids::ChannelId;
use crate::provider::{ChannelProvider, ProviderHandle};
use crate::spec::{Capabilities, ProviderKind};

/// Inbox of a memory provider. Callers (tests, the in-process bridge)
/// push fully-formed [`InboundMessage`]s through this; the provider
/// task forwards them to the harness's inbound channel.
#[derive(Clone)]
pub struct MemoryInbox {
    tx: mpsc::UnboundedSender<InboundMessage>,
}

impl MemoryInbox {
    /// Push an inbound. Returns an error only if the provider has been
    /// stopped (the receiving end has dropped).
    pub fn push(&self, msg: InboundMessage) -> Result<()> {
        self.tx
            .send(msg)
            .map_err(|_| ChannelError::transport("memory inbox closed"))
    }
}

/// In-process [`ChannelProvider`].
pub struct InMemoryProvider {
    channel_id: ChannelId,
    capabilities: Capabilities,
    inbox_tx: mpsc::UnboundedSender<InboundMessage>,
    inbox_rx: Mutex<Option<mpsc::UnboundedReceiver<InboundMessage>>>,
    sent_tx: broadcast::Sender<OutboundMessage>,
    media: Mutex<std::collections::HashMap<String, Bytes>>,
    counter: AtomicU64,
}

impl InMemoryProvider {
    pub fn new(channel_id: ChannelId) -> Self {
        Self::with_capabilities(channel_id, Capabilities::text_only())
    }

    pub fn with_capabilities(channel_id: ChannelId, capabilities: Capabilities) -> Self {
        let (inbox_tx, inbox_rx) = mpsc::unbounded_channel();
        let (sent_tx, _) = broadcast::channel(256);
        Self {
            channel_id,
            capabilities,
            inbox_tx,
            inbox_rx: Mutex::new(Some(inbox_rx)),
            sent_tx,
            media: Mutex::new(Default::default()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }

    pub fn inbox(&self) -> MemoryInbox {
        MemoryInbox {
            tx: self.inbox_tx.clone(),
        }
    }

    /// Subscribe to the outbound log. Each `send()` fans out one item.
    pub fn sent_log(&self) -> broadcast::Receiver<OutboundMessage> {
        self.sent_tx.subscribe()
    }

    /// Register bytes under a media ref so `fetch_media` returns them.
    pub fn insert_media(&self, media_ref: impl Into<String>, bytes: Bytes) {
        self.media.lock().insert(media_ref.into(), bytes);
    }

    fn next_provider_msg_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("mem-{n}")
    }
}

#[async_trait]
impl ChannelProvider for InMemoryProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Memory
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<ProviderHandle> {
        let rx = self.inbox_rx.lock().take().ok_or_else(|| {
            ChannelError::provider("InMemoryProvider already started")
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = stop.clone();
        let join = tokio::spawn(async move {
            let mut rx = rx;
            loop {
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        if stop_task.load(Ordering::Relaxed) { break; }
                    }
                    next = rx.recv() => {
                        match next {
                            Some(msg) => {
                                if inbound_tx.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });
        Ok(ProviderHandle::new(stop, join))
    }

    async fn send(&self, msg: OutboundMessage) -> Result<ProviderAck> {
        let _ = self.sent_tx.send(msg.clone());
        Ok(ProviderAck {
            provider_msg_id: self.next_provider_msg_id(),
            sent_at: Utc::now(),
        })
    }

    async fn fetch_media(&self, media_ref: &str) -> Result<Bytes> {
        self.media
            .lock()
            .get(media_ref)
            .cloned()
            .ok_or_else(|| ChannelError::provider(format!("unknown media_ref: {media_ref}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::MessageContent;
    use crate::ids::{PeerId, ThreadId};

    #[tokio::test]
    async fn start_forwards_inbound_until_stop() {
        let p = InMemoryProvider::new(ChannelId::from("memory:t"));
        let inbox = p.inbox();
        let (tx, mut rx) = mpsc::channel(8);
        let handle = p.start(tx).await.unwrap();

        let m = InboundMessage {
            channel_id: ChannelId::from("memory:t"),
            thread_id: ThreadId::from("t1"),
            peer: PeerId::from("alice"),
            provider_msg_id: "pmid-1".into(),
            content: MessageContent::text("hello"),
            received_at: Utc::now(),
            raw: serde_json::Value::Null,
        };
        inbox.push(m).unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.provider_msg_id, "pmid-1");

        handle.signal_stop();
        handle.join.await.unwrap();
    }

    #[tokio::test]
    async fn send_records_outbound() {
        let p = InMemoryProvider::new(ChannelId::from("memory:t"));
        let mut log = p.sent_log();
        let ack = p
            .send(OutboundMessage {
                channel_id: ChannelId::from("memory:t"),
                thread_id: ThreadId::from("t1"),
                peer: PeerId::from("alice"),
                content: MessageContent::text("hi"),
                reply_to: None,
                idempotency_key: "k1".into(),
            })
            .await
            .unwrap();
        assert_eq!(ack.provider_msg_id, "mem-0");
        let entry = tokio::time::timeout(std::time::Duration::from_millis(200), log.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.idempotency_key, "k1");
    }
}
