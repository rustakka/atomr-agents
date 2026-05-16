use std::sync::Arc;

use atomr_agents_channel_core::{
    ChannelEvent, ChannelEventStream, ChannelError, ChannelId, ChannelMessageRecord,
    ChannelProvider, ChannelSpec, ChannelStore, Direction, InMemoryChannelStore, InboundMessage,
    MessageContent, OutboundMessage, PeerId, ProviderAck, ProviderHandle, Result, Thread,
    ThreadId, ThreadRef, ThreadSummary, ThreadTarget,
};
use atomr_agents_observability::EventBus;
use atomr_agents_registry::{ArtifactKind, ArtifactRecord, Registry};
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::builder::ChannelHarnessBuilder;
use crate::inbound::spawn_inbound_loop;
use crate::outbound::{spawn_outbound_worker, OutboundJob};

const EVENT_CHANNEL_CAPACITY: usize = 512;
const INBOUND_CHANNEL_CAPACITY: usize = 256;
const OUTBOUND_CHANNEL_CAPACITY: usize = 256;

pub(crate) struct AttachedProvider {
    pub provider: Arc<dyn ChannelProvider>,
    pub handle: ProviderHandle,
    pub outbound_tx: mpsc::Sender<OutboundJob>,
    pub outbound_join: tokio::task::JoinHandle<()>,
}

pub(crate) struct HarnessInner {
    pub providers: DashMap<ChannelId, AttachedProvider>,
    pub threads: DashMap<ThreadId, Arc<RwLock<Thread>>>,
    pub store: Arc<dyn ChannelStore>,
    pub event_tx: broadcast::Sender<ChannelEvent>,
    pub bus: EventBus,
    pub registry: Option<Arc<Registry>>,
    pub default_policy: atomr_agents_channel_core::ThreadPolicy,
    pub auto_open_target: Option<ThreadTarget>,
    pub inbound_tx: mpsc::Sender<InboundMessage>,
}

/// The channel orchestrator.
pub struct ChannelHarness {
    pub(crate) inner: Arc<HarnessInner>,
    pub(crate) inbound_join: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ChannelHarness {
    /// Start a builder.
    pub fn builder() -> ChannelHarnessBuilder {
        ChannelHarnessBuilder::default()
    }

    /// Quick-start with an in-memory store and no auto-open target.
    pub fn in_memory() -> Self {
        Self::builder().build()
    }

    pub(crate) fn from_parts(
        store: Arc<dyn ChannelStore>,
        registry: Option<Arc<Registry>>,
        default_policy: atomr_agents_channel_core::ThreadPolicy,
        auto_open_target: Option<ThreadTarget>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND_CHANNEL_CAPACITY);
        let inner = Arc::new(HarnessInner {
            providers: DashMap::new(),
            threads: DashMap::new(),
            store,
            event_tx,
            bus: EventBus::new(),
            registry,
            default_policy,
            auto_open_target,
            inbound_tx,
        });
        let join = spawn_inbound_loop(inner.clone(), inbound_rx);
        Self {
            inner,
            inbound_join: parking_lot::Mutex::new(Some(join)),
        }
    }

    /// Subscribe to the event stream. Lagged subscribers skip dropped events silently.
    pub fn events(&self) -> ChannelEventStream {
        ChannelEventStream::new(self.inner.event_tx.subscribe())
    }

    /// Clone the broadcast sender (for the `-web` companion).
    pub fn event_sender(&self) -> broadcast::Sender<ChannelEvent> {
        self.inner.event_tx.clone()
    }

    /// Observability bus — pluggable subscribers consume `Event`s here.
    pub fn bus(&self) -> &EventBus {
        &self.inner.bus
    }

    /// Underlying store.
    pub fn store(&self) -> Arc<dyn ChannelStore> {
        self.inner.store.clone()
    }

    /// Attach a provider under `spec.id`. Starts the provider's task and
    /// the per-channel outbound worker. Persists the spec and (optionally)
    /// publishes a Registry artifact.
    pub async fn attach_provider(
        &self,
        spec: ChannelSpec,
        provider: Arc<dyn ChannelProvider>,
    ) -> Result<()> {
        if self.inner.providers.contains_key(&spec.id) {
            return Err(ChannelError::DuplicateChannel(spec.id.as_str().to_string()));
        }

        self.inner.store.upsert_channel(&spec).await?;

        let handle = provider.start(self.inner.inbound_tx.clone()).await?;

        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_CHANNEL_CAPACITY);
        let outbound_join =
            spawn_outbound_worker(self.inner.clone(), provider.clone(), spec.id.clone(), outbound_rx);

        let attached = AttachedProvider {
            provider: provider.clone(),
            handle,
            outbound_tx,
            outbound_join,
        };
        self.inner.providers.insert(spec.id.clone(), attached);

        let _ = self.inner.event_tx.send(ChannelEvent::ProviderConnected {
            provider: provider.kind(),
            channel_id: spec.id.clone(),
        });

        if let Some(registry) = &self.inner.registry {
            let payload = serde_json::to_value(&spec).unwrap_or(serde_json::Value::Null);
            let record = ArtifactRecord {
                kind: ArtifactKind::Channel,
                id: spec.id.as_str().to_string(),
                version: semver::Version::new(0, 1, 0),
                payload,
                published_at_ms: Utc::now().timestamp_millis(),
                baseline_pass_rate: None,
                current_pass_rate: None,
            };
            registry.publish(record);
        }

        Ok(())
    }

    /// Detach a provider. Signals stop, awaits the provider task and
    /// drains the outbound worker. Channel state in the store is left intact.
    pub async fn detach_provider(&self, id: &ChannelId) -> Result<()> {
        let Some((_, attached)) = self.inner.providers.remove(id) else {
            return Err(ChannelError::UnknownChannel(id.as_str().to_string()));
        };
        attached.handle.signal_stop();
        let _ = attached.handle.join.await;
        drop(attached.outbound_tx);
        let _ = attached.outbound_join.await;

        let kind = attached.provider.kind();
        let _ = self.inner.event_tx.send(ChannelEvent::ProviderDisconnected {
            provider: kind,
            channel_id: id.clone(),
            reason: "detached".into(),
        });
        Ok(())
    }

    pub fn list_attached(&self) -> Vec<ChannelId> {
        self.inner
            .providers
            .iter()
            .map(|e| e.key().clone())
            .collect()
    }

    pub async fn list_channels(&self) -> Result<Vec<ChannelSpec>> {
        self.inner.store.list_channels().await
    }

    pub async fn get_channel(&self, id: &ChannelId) -> Result<Option<ChannelSpec>> {
        self.inner.store.get_channel(id).await
    }

    /// Open a thread between `peer` and `target` on `channel`. If a
    /// thread for this `(channel, peer)` already exists, the existing
    /// thread's target is replaced with the new one.
    pub async fn open_thread(
        &self,
        channel: &ChannelId,
        peer: PeerId,
        target: ThreadTarget,
    ) -> Result<ThreadRef> {
        // Verify the channel exists (must be attached).
        if !self.inner.providers.contains_key(channel) {
            return Err(ChannelError::UnknownChannel(channel.as_str().to_string()));
        }
        let id = ThreadId::for_peer(channel, &peer);
        let mut thread = Thread::new(channel.clone(), peer.clone(), target);
        thread.policy = self.inner.default_policy;

        // If we already have a thread cached for this peer, preserve its
        // history but replace the target with the new binding.
        if let Some(existing_ref) = self.inner.threads.get(&id) {
            let mut g = existing_ref.write();
            g.target = thread.target.clone();
        } else {
            self.inner
                .threads
                .insert(id.clone(), Arc::new(RwLock::new(thread.clone())));
        }
        self.inner.store.upsert_thread(&thread).await?;

        let _ = self.inner.event_tx.send(ChannelEvent::ThreadOpened {
            thread_id: id.clone(),
            channel_id: channel.clone(),
            peer,
        });

        Ok(ThreadRef::from_arc(
            self.inner.threads.get(&id).unwrap().clone(),
        ))
    }

    /// Close a thread. Removes its cached handle and drops its store row.
    pub async fn close_thread(&self, id: &ThreadId) -> Result<()> {
        self.inner.threads.remove(id);
        self.inner.store.delete_thread(id).await?;
        let _ = self.inner.event_tx.send(ChannelEvent::ThreadClosed {
            thread_id: id.clone(),
            reason: "closed".into(),
        });
        Ok(())
    }

    /// Look up a thread by id. Returns a [`ThreadRef`] if open.
    pub fn thread(&self, id: &ThreadId) -> Option<ThreadRef> {
        self.inner.threads.get(id).map(|r| ThreadRef::from_arc(r.value().clone()))
    }

    pub async fn list_threads(&self, channel: &ChannelId) -> Result<Vec<ThreadSummary>> {
        self.inner.store.list_threads(channel).await
    }

    pub async fn list_messages(
        &self,
        thread: &ThreadId,
        limit: usize,
    ) -> Result<Vec<ChannelMessageRecord>> {
        self.inner.store.list_messages(thread, limit).await
    }

    /// Admin send — bypasses the bound target and pushes an outbound
    /// message into the channel's worker. Returns once the provider has
    /// acked (or the worker errors).
    pub async fn send(
        &self,
        thread_id: &ThreadId,
        content: MessageContent,
    ) -> Result<ProviderAck> {
        let thread = self
            .inner
            .threads
            .get(thread_id)
            .ok_or_else(|| ChannelError::UnknownThread(thread_id.as_str().to_string()))?;
        let (channel, peer) = {
            let g = thread.read();
            (g.channel.clone(), g.peer.clone())
        };
        // Validate capabilities.
        let spec = self
            .inner
            .store
            .get_channel(&channel)
            .await?
            .ok_or_else(|| ChannelError::UnknownChannel(channel.as_str().to_string()))?;
        content.check_capabilities(&spec.capabilities)?;

        let idempotency_key = format!("admin-{}", Uuid::new_v4());
        let outbound = OutboundMessage {
            channel_id: channel.clone(),
            thread_id: thread_id.clone(),
            peer: peer.clone(),
            content: content.clone(),
            reply_to: None,
            idempotency_key: idempotency_key.clone(),
        };
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let job = OutboundJob {
            outbound,
            ack: Some(ack_tx),
        };
        let attached = self
            .inner
            .providers
            .get(&channel)
            .ok_or_else(|| ChannelError::UnknownChannel(channel.as_str().to_string()))?;
        attached
            .outbound_tx
            .send(job)
            .await
            .map_err(|_| ChannelError::transport("outbound queue closed"))?;
        drop(attached);
        ack_rx
            .await
            .map_err(|_| ChannelError::transport("outbound worker dropped reply"))?
    }

    /// Process a verified webhook payload. The harness asks the
    /// provider to parse it, then injects the resulting inbound message(s)
    /// onto the inbound queue.
    pub async fn ingest_webhook(
        &self,
        channel: &ChannelId,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<usize> {
        let attached = self
            .inner
            .providers
            .get(channel)
            .ok_or_else(|| ChannelError::UnknownChannel(channel.as_str().to_string()))?;
        let provider = attached.provider.clone();
        drop(attached);

        provider.verify_webhook(headers, body)?;
        let messages = provider.parse_webhook(headers, body)?;
        let n = messages.len();
        for m in messages {
            self.inner
                .inbound_tx
                .send(m)
                .await
                .map_err(|_| ChannelError::transport("inbound queue closed"))?;
        }
        Ok(n)
    }

    /// Synchronously inject a fully-formed inbound message. Used by
    /// tests and by providers that surface inbound through their own
    /// API (the in-memory provider already does this internally via the
    /// inbox; this is the fallback path).
    pub async fn ingest(&self, msg: InboundMessage) -> Result<()> {
        self.inner
            .inbound_tx
            .send(msg)
            .await
            .map_err(|_| ChannelError::transport("inbound queue closed"))
    }

    /// Shut everything down. Detaches every provider, awaits workers,
    /// and joins the inbound loop.
    pub async fn shutdown(&self) -> Result<()> {
        let ids: Vec<_> = self.inner.providers.iter().map(|e| e.key().clone()).collect();
        for id in ids {
            self.detach_provider(&id).await?;
        }
        // Close the inbound channel by dropping all senders held in
        // attached providers; the inbound loop will exit. We hold a tx
        // in `inner.inbound_tx` though — drop a clone here is not enough.
        // The loop also exits when all senders are dropped, which
        // happens when `self.inner` is dropped. For graceful shutdown,
        // join the loop on the next `drop`.
        let maybe_join = self.inbound_join.lock().take();
        if let Some(join) = maybe_join {
            join.abort();
            let _ = join.await;
        }
        Ok(())
    }
}

/// Convenience: build a [`ChannelMessageRecord`] for logging.
pub(crate) fn record_for(
    thread_id: &ThreadId,
    direction: Direction,
    content: MessageContent,
    provider_msg_id: Option<String>,
    idempotency_key: Option<String>,
) -> ChannelMessageRecord {
    ChannelMessageRecord {
        thread_id: thread_id.clone(),
        id: Uuid::new_v4().to_string(),
        direction,
        content,
        provider_msg_id,
        idempotency_key,
        at: Utc::now(),
    }
}

#[doc(hidden)]
pub fn __default_store() -> Arc<dyn ChannelStore> {
    Arc::new(InMemoryChannelStore::new())
}

impl Default for ChannelHarness {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl Drop for ChannelHarness {
    fn drop(&mut self) {
        if let Some(join) = self.inbound_join.lock().take() {
            join.abort();
        }
    }
}

