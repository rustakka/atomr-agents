//! Persistence surface for channel state.
//!
//! Same dual-backend pattern as
//! [`atomr_agents_meetings_harness::MeetingsStore`]: a single trait
//! that the in-memory default and a feature-gated checkpointer-backed
//! impl both satisfy. The orchestrator holds an `Arc<dyn ChannelStore>`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::content::ChannelMessageRecord;
use crate::error::Result;
use crate::ids::{ChannelId, ThreadId};
use crate::spec::ChannelSpec;
use crate::thread::Thread;

/// Lightweight summary row for listing threads in a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub channel: ChannelId,
    pub peer: String,
    pub target_kind: String,
    pub history_len: usize,
}

impl ThreadSummary {
    pub fn of(t: &Thread) -> Self {
        Self {
            id: t.id.clone(),
            channel: t.channel.clone(),
            peer: t.peer.as_str().to_string(),
            target_kind: t.target.kind().to_string(),
            history_len: t.history.len(),
        }
    }
}

#[async_trait]
pub trait ChannelStore: Send + Sync + 'static {
    // ----- Channels --------------------------------------------------
    async fn upsert_channel(&self, spec: &ChannelSpec) -> Result<()>;
    async fn get_channel(&self, id: &ChannelId) -> Result<Option<ChannelSpec>>;
    async fn list_channels(&self) -> Result<Vec<ChannelSpec>>;
    async fn delete_channel(&self, id: &ChannelId) -> Result<()>;

    // ----- Threads ---------------------------------------------------
    async fn upsert_thread(&self, thread: &Thread) -> Result<()>;
    async fn get_thread(&self, id: &ThreadId) -> Result<Option<Thread>>;
    async fn list_threads(&self, channel: &ChannelId) -> Result<Vec<ThreadSummary>>;
    async fn delete_thread(&self, id: &ThreadId) -> Result<()>;

    // ----- Messages --------------------------------------------------
    async fn append_message(&self, rec: &ChannelMessageRecord) -> Result<()>;
    async fn list_messages(&self, thread: &ThreadId, limit: usize) -> Result<Vec<ChannelMessageRecord>>;

    /// Idempotency / dedup helpers.
    async fn lookup_outbound_by_key(
        &self,
        thread: &ThreadId,
        idempotency_key: &str,
    ) -> Result<Option<String>>;
    async fn has_inbound(&self, channel: &ChannelId, provider_msg_id: &str) -> Result<bool>;
}

#[async_trait]
impl ChannelStore for Arc<dyn ChannelStore> {
    async fn upsert_channel(&self, spec: &ChannelSpec) -> Result<()> {
        (**self).upsert_channel(spec).await
    }
    async fn get_channel(&self, id: &ChannelId) -> Result<Option<ChannelSpec>> {
        (**self).get_channel(id).await
    }
    async fn list_channels(&self) -> Result<Vec<ChannelSpec>> {
        (**self).list_channels().await
    }
    async fn delete_channel(&self, id: &ChannelId) -> Result<()> {
        (**self).delete_channel(id).await
    }
    async fn upsert_thread(&self, thread: &Thread) -> Result<()> {
        (**self).upsert_thread(thread).await
    }
    async fn get_thread(&self, id: &ThreadId) -> Result<Option<Thread>> {
        (**self).get_thread(id).await
    }
    async fn list_threads(&self, channel: &ChannelId) -> Result<Vec<ThreadSummary>> {
        (**self).list_threads(channel).await
    }
    async fn delete_thread(&self, id: &ThreadId) -> Result<()> {
        (**self).delete_thread(id).await
    }
    async fn append_message(&self, rec: &ChannelMessageRecord) -> Result<()> {
        (**self).append_message(rec).await
    }
    async fn list_messages(&self, thread: &ThreadId, limit: usize) -> Result<Vec<ChannelMessageRecord>> {
        (**self).list_messages(thread, limit).await
    }
    async fn lookup_outbound_by_key(
        &self,
        thread: &ThreadId,
        idempotency_key: &str,
    ) -> Result<Option<String>> {
        (**self).lookup_outbound_by_key(thread, idempotency_key).await
    }
    async fn has_inbound(&self, channel: &ChannelId, provider_msg_id: &str) -> Result<bool> {
        (**self).has_inbound(channel, provider_msg_id).await
    }
}

/// Process-local, volatile channel store.
#[derive(Default)]
struct StoreInner {
    channels: HashMap<ChannelId, ChannelSpec>,
    threads: HashMap<ThreadId, Thread>,
    messages: HashMap<ThreadId, Vec<ChannelMessageRecord>>,
    /// `(channel_id, provider_msg_id)` set used for inbound dedup.
    inbound_seen: std::collections::HashSet<(ChannelId, String)>,
}

#[derive(Default, Clone)]
pub struct InMemoryChannelStore {
    inner: Arc<RwLock<StoreInner>>,
}

impl InMemoryChannelStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ChannelStore for InMemoryChannelStore {
    async fn upsert_channel(&self, spec: &ChannelSpec) -> Result<()> {
        self.inner.write().channels.insert(spec.id.clone(), spec.clone());
        Ok(())
    }

    async fn get_channel(&self, id: &ChannelId) -> Result<Option<ChannelSpec>> {
        Ok(self.inner.read().channels.get(id).cloned())
    }

    async fn list_channels(&self) -> Result<Vec<ChannelSpec>> {
        let mut v: Vec<_> = self.inner.read().channels.values().cloned().collect();
        v.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(v)
    }

    async fn delete_channel(&self, id: &ChannelId) -> Result<()> {
        let mut g = self.inner.write();
        g.channels.remove(id);
        let drop_threads: Vec<_> = g
            .threads
            .iter()
            .filter(|(_, t)| &t.channel == id)
            .map(|(tid, _)| tid.clone())
            .collect();
        for tid in drop_threads {
            g.threads.remove(&tid);
            g.messages.remove(&tid);
        }
        g.inbound_seen.retain(|(cid, _)| cid != id);
        Ok(())
    }

    async fn upsert_thread(&self, thread: &Thread) -> Result<()> {
        self.inner.write().threads.insert(thread.id.clone(), thread.clone());
        Ok(())
    }

    async fn get_thread(&self, id: &ThreadId) -> Result<Option<Thread>> {
        Ok(self.inner.read().threads.get(id).cloned())
    }

    async fn list_threads(&self, channel: &ChannelId) -> Result<Vec<ThreadSummary>> {
        let mut v: Vec<_> = self
            .inner
            .read()
            .threads
            .values()
            .filter(|t| &t.channel == channel)
            .map(ThreadSummary::of)
            .collect();
        v.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(v)
    }

    async fn delete_thread(&self, id: &ThreadId) -> Result<()> {
        let mut g = self.inner.write();
        g.threads.remove(id);
        g.messages.remove(id);
        Ok(())
    }

    async fn append_message(&self, rec: &ChannelMessageRecord) -> Result<()> {
        let mut g = self.inner.write();
        if let Some(pid) = &rec.provider_msg_id {
            if matches!(rec.direction, crate::content::Direction::Inbound) {
                let channel = g.threads.get(&rec.thread_id).map(|t| t.channel.clone());
                if let Some(channel) = channel {
                    g.inbound_seen.insert((channel, pid.clone()));
                }
            }
        }
        g.messages
            .entry(rec.thread_id.clone())
            .or_default()
            .push(rec.clone());
        Ok(())
    }

    async fn list_messages(&self, thread: &ThreadId, limit: usize) -> Result<Vec<ChannelMessageRecord>> {
        let g = self.inner.read();
        let v = g
            .messages
            .get(thread)
            .map(|v| {
                let take = if limit == 0 || limit > v.len() {
                    v.len()
                } else {
                    limit
                };
                v[v.len() - take..].to_vec()
            })
            .unwrap_or_default();
        Ok(v)
    }

    async fn lookup_outbound_by_key(
        &self,
        thread: &ThreadId,
        idempotency_key: &str,
    ) -> Result<Option<String>> {
        Ok(self.inner.read().messages.get(thread).and_then(|v| {
            v.iter()
                .find(|r| r.idempotency_key.as_deref() == Some(idempotency_key))
                .and_then(|r| r.provider_msg_id.clone())
        }))
    }

    async fn has_inbound(&self, channel: &ChannelId, provider_msg_id: &str) -> Result<bool> {
        Ok(self
            .inner
            .read()
            .inbound_seen
            .contains(&(channel.clone(), provider_msg_id.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{Direction, MessageContent};
    use crate::ids::PeerId;
    use crate::spec::{Capabilities, ProviderKind};
    use crate::target::ThreadTarget;
    use atomr_agents_callable::FnCallable;
    use std::sync::Arc;

    fn fake_thread(channel: &ChannelId, peer: &str) -> Thread {
        let handle: atomr_agents_callable::CallableHandle =
            Arc::new(FnCallable::new(|v, _ctx| async move { Ok(v) }));
        Thread::new(
            channel.clone(),
            PeerId::from(peer),
            ThreadTarget::callable(handle),
        )
    }

    #[tokio::test]
    async fn channel_round_trip() {
        let s = InMemoryChannelStore::new();
        let spec = ChannelSpec::new(ChannelId::from("memory:dev"), ProviderKind::Memory)
            .with_capabilities(Capabilities::text_only());
        s.upsert_channel(&spec).await.unwrap();
        assert_eq!(s.list_channels().await.unwrap().len(), 1);
        s.delete_channel(&spec.id).await.unwrap();
        assert!(s.list_channels().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn thread_round_trip_and_message_log() {
        let s = InMemoryChannelStore::new();
        let chan = ChannelId::from("memory:dev");
        let t = fake_thread(&chan, "alice");
        s.upsert_channel(
            &ChannelSpec::new(chan.clone(), ProviderKind::Memory),
        )
        .await
        .unwrap();
        s.upsert_thread(&t).await.unwrap();
        assert_eq!(s.list_threads(&chan).await.unwrap().len(), 1);

        let rec = ChannelMessageRecord {
            thread_id: t.id.clone(),
            id: "m1".into(),
            direction: Direction::Inbound,
            content: MessageContent::text("hi"),
            provider_msg_id: Some("pmid-1".into()),
            idempotency_key: None,
            at: chrono::Utc::now(),
        };
        s.append_message(&rec).await.unwrap();
        assert!(s.has_inbound(&chan, "pmid-1").await.unwrap());
        assert_eq!(s.list_messages(&t.id, 0).await.unwrap().len(), 1);
    }
}
