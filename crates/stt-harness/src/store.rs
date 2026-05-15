//! Conversation persistence.
//!
//! [`ConversationStore`] is the trait the web layer and any caller use
//! to list, fetch, and mutate conversations. [`InMemoryConversationStore`]
//! is the always-available default. The `state` feature adds
//! [`CheckpointerConversationStore`], which routes through
//! `crates/state`'s `Checkpointer` so persistence honours whatever
//! backend is configured (in-memory, SQLite, Postgres).
//!
//! Speaker-label edits persist because [`ConversationStore::rename_speaker`]
//! reads the conversation, applies the rename, and writes it back —
//! through whichever backend is in use.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::BackendKind;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::conversation::SttConversation;
use crate::error::Result;

/// A lightweight summary row for conversation-list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub language: Option<String>,
    pub turn_count: usize,
    pub speaker_count: usize,
    pub total_audio_secs: f32,
    pub backend: Option<BackendKind>,
}

impl ConversationSummary {
    /// Derive a summary from a full conversation.
    pub fn of(conv: &SttConversation) -> Self {
        Self {
            id: conv.id.clone(),
            language: conv.language.clone(),
            turn_count: conv.turns.len(),
            speaker_count: conv.speaker_ids().len(),
            total_audio_secs: conv.total_audio_secs,
            backend: conv.backend.clone(),
        }
    }
}

/// Persistence surface for STT conversations.
#[async_trait]
pub trait ConversationStore: Send + Sync + 'static {
    /// Insert or replace a conversation.
    async fn put(&self, conv: &SttConversation) -> Result<()>;

    /// Fetch a conversation by id.
    async fn get(&self, id: &str) -> Result<Option<SttConversation>>;

    /// List every stored conversation as a summary row.
    async fn list(&self) -> Result<Vec<ConversationSummary>>;

    /// Delete a conversation. Deleting a missing id is not an error.
    async fn delete(&self, id: &str) -> Result<()>;

    /// Rename a speaker and persist the change. Returns the updated
    /// conversation, or `None` if the id was not found. The default
    /// implementation is read-modify-write, so the edit lands in
    /// whatever backend the store wraps.
    async fn rename_speaker(
        &self,
        id: &str,
        speaker_id: u8,
        label: String,
    ) -> Result<Option<SttConversation>> {
        let Some(mut conv) = self.get(id).await? else {
            return Ok(None);
        };
        conv.rename_speaker(speaker_id, label);
        self.put(&conv).await?;
        Ok(Some(conv))
    }
}

#[async_trait]
impl ConversationStore for Arc<dyn ConversationStore> {
    async fn put(&self, conv: &SttConversation) -> Result<()> {
        (**self).put(conv).await
    }
    async fn get(&self, id: &str) -> Result<Option<SttConversation>> {
        (**self).get(id).await
    }
    async fn list(&self) -> Result<Vec<ConversationSummary>> {
        (**self).list().await
    }
    async fn delete(&self, id: &str) -> Result<()> {
        (**self).delete(id).await
    }
    async fn rename_speaker(
        &self,
        id: &str,
        speaker_id: u8,
        label: String,
    ) -> Result<Option<SttConversation>> {
        (**self).rename_speaker(id, speaker_id, label).await
    }
}

/// Process-local, volatile conversation store. The default when no
/// persistence backend is configured.
#[derive(Clone, Default)]
pub struct InMemoryConversationStore {
    inner: Arc<RwLock<HashMap<String, SttConversation>>>,
}

impl InMemoryConversationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ConversationStore for InMemoryConversationStore {
    async fn put(&self, conv: &SttConversation) -> Result<()> {
        self.inner.write().insert(conv.id.clone(), conv.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<SttConversation>> {
        Ok(self.inner.read().get(id).cloned())
    }

    async fn list(&self) -> Result<Vec<ConversationSummary>> {
        let mut rows: Vec<ConversationSummary> =
            self.inner.read().values().map(ConversationSummary::of).collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.inner.write().remove(id);
        Ok(())
    }
}

#[cfg(feature = "state")]
mod checkpointer_store;
#[cfg(feature = "state")]
pub use checkpointer_store::CheckpointerConversationStore;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::SttConversation;

    fn conv(id: &str) -> SttConversation {
        let mut c = SttConversation::new(id);
        c.append_agent_reply("hello");
        c
    }

    #[tokio::test]
    async fn in_memory_put_get_list_delete() {
        let store = InMemoryConversationStore::new();
        store.put(&conv("a")).await.unwrap();
        store.put(&conv("b")).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 2);
        assert!(store.get("a").await.unwrap().is_some());
        store.delete("a").await.unwrap();
        assert!(store.get("a").await.unwrap().is_none());
        assert_eq!(store.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rename_speaker_persists_through_the_store() {
        let store = InMemoryConversationStore::new();
        let mut c = SttConversation::new("c1");
        // a diarized turn for speaker 0
        c.commit_segment(atomr_agents_stt_core::Segment {
            text: "hi".into(),
            start_ms: 0,
            end_ms: 0,
            words: vec![],
            speaker: Some(atomr_agents_stt_core::SpeakerTag { id: 0, label: None }),
            confidence: None,
        });
        store.put(&c).await.unwrap();

        let updated = store
            .rename_speaker("c1", 0, "Alice".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.effective_label(0), "Alice");

        // A fresh read sees the persisted rename.
        let reloaded = store.get("c1").await.unwrap().unwrap();
        assert_eq!(reloaded.effective_label(0), "Alice");
    }
}
