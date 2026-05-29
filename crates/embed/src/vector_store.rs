//! Embeddings batch trait + an in-memory [`VectorStore`].
//!
//! The [`VectorStore`] / [`Hit`] / [`MetadataFilter`] contracts live in
//! `atomr-agents-memory` (the lowest crate both this embeddings layer and
//! the production backends can share without a dependency cycle); they are
//! re-exported here for ergonomics. The [`Embeddings`] batch trait and the
//! [`InMemoryVectorStore`] live here because they build on [`Embedder`].
//!
//! Motivation (production semantic layer): formalize the embeddings
//! contract the retriever zoo and vector backends program against, and
//! provide a cosine-similarity in-memory store for tests and small hot
//! tiers that applies [`MetadataFilter`] at query time.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, Value};
use parking_lot::RwLock;

use crate::embedder::Embedder;

pub use atomr_agents_memory::{cosine, Hit, MetadataFilter, VectorStore};

/// Batch embeddings interface. A blanket impl bridges any existing
/// single-text [`Embedder`], so all current embedders satisfy
/// [`Embeddings`] for free.
#[async_trait]
pub trait Embeddings: Send + Sync + 'static {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
}

#[async_trait]
impl<E: Embedder + ?Sized> Embeddings for E {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        Embedder::embed_batch(self, &texts).await
    }
}

/// A stored row: `(id, embedding, metadata)`.
type Row = (String, Vec<f32>, Value);

/// In-memory cosine-similarity vector store. Applies [`MetadataFilter`]
/// at query time before ranking. Suitable for tests and small hot tiers.
#[derive(Default, Clone)]
pub struct InMemoryVectorStore {
    inner: Arc<RwLock<Vec<Row>>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn upsert(&self, items: Vec<(String, Vec<f32>, Value)>) -> Result<()> {
        let mut g = self.inner.write();
        for (id, vec, meta) in items {
            if let Some(slot) = g.iter_mut().find(|(i, _, _)| *i == id) {
                slot.1 = vec;
                slot.2 = meta;
            } else {
                g.push((id, vec, meta));
            }
        }
        Ok(())
    }

    async fn query(&self, embedding: Vec<f32>, k: usize, filter: &MetadataFilter) -> Result<Vec<Hit>> {
        let g = self.inner.read();
        let mut scored: Vec<Hit> = g
            .iter()
            .filter(|(_, _, meta)| filter.matches(meta))
            .map(|(id, vec, meta)| Hit {
                id: id.clone(),
                score: cosine(&embedding, vec),
                metadata: meta.clone(),
            })
            .collect();
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    async fn delete(&self, ids: Vec<String>) -> Result<()> {
        let mut g = self.inner.write();
        g.retain(|(id, _, _)| !ids.contains(id));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::MockEmbedder;

    #[tokio::test]
    async fn in_memory_store_roundtrip_with_filter() {
        let store = InMemoryVectorStore::new();
        store
            .upsert(vec![
                ("a".into(), vec![1.0, 0.0], serde_json::json!({"lang": "rust"})),
                ("b".into(), vec![0.0, 1.0], serde_json::json!({"lang": "python"})),
            ])
            .await
            .unwrap();
        // Filter restricts to rust before ranking.
        let hits = store
            .query(
                vec![0.0, 1.0],
                5,
                &MetadataFilter::Eq("lang".into(), serde_json::json!("rust")),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a");

        // Delete.
        store.delete(vec!["a".into()]).await.unwrap();
        let hits = store.query(vec![1.0, 0.0], 5, &MetadataFilter::All).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "b");
    }

    #[tokio::test]
    async fn embeddings_blanket_impl_bridges_embedder() {
        let e = MockEmbedder::new(8);
        let v = Embeddings::embed(&e, vec!["hello".into(), "world".into()]).await.unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].len(), 8);
    }
}
