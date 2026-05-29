//! Dense retriever. Two interchangeable backings:
//!
//! * the original [`LongStore`] semantic search (kept for back-compat);
//! * an injectable [`VectorStore`] + [`Embeddings`] pair (FR-10), so any
//!   production backend (in-memory, pgvector, Redis) is swappable at host
//!   time without touching call sites.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};
use atomr_agents_embed::{Embedder, Embeddings, MetadataFilter, VectorStore};
use atomr_agents_memory::{LongStore, Namespace};

use crate::retriever::{Document, Retriever};

/// Internal backing for [`VectorRetriever`].
enum Backing {
    /// Original `LongStore`-backed semantic search.
    Store {
        store: Arc<dyn LongStore>,
        embedder: Arc<dyn Embedder>,
        namespace: Namespace,
    },
    /// Injectable production [`VectorStore`] + [`Embeddings`] (FR-10).
    VectorStore {
        vstore: Arc<dyn VectorStore>,
        embeddings: Arc<dyn Embeddings>,
        filter: MetadataFilter,
    },
}

pub struct VectorRetriever {
    backing: Backing,
    pub top_k: usize,
}

impl VectorRetriever {
    /// Construct from a [`LongStore`] + single-text [`Embedder`] (original
    /// form; existing call sites and tests keep working).
    pub fn new(
        store: Arc<dyn LongStore>,
        embedder: Arc<dyn Embedder>,
        namespace: Namespace,
        top_k: usize,
    ) -> Self {
        Self {
            backing: Backing::Store {
                store,
                embedder,
                namespace,
            },
            top_k,
        }
    }

    /// Construct from an injectable [`VectorStore`] + [`Embeddings`] (FR-10).
    /// A backend-honored [`MetadataFilter`] restricts the candidate set at
    /// query time.
    pub fn from_vector_store(
        vstore: Arc<dyn VectorStore>,
        embeddings: Arc<dyn Embeddings>,
        top_k: usize,
    ) -> Self {
        Self {
            backing: Backing::VectorStore {
                vstore,
                embeddings,
                filter: MetadataFilter::All,
            },
            top_k,
        }
    }

    /// Set the query-time [`MetadataFilter`] (only meaningful for the
    /// [`VectorStore`] backing; ignored for the `LongStore` backing).
    pub fn with_metadata_filter(mut self, filter: MetadataFilter) -> Self {
        if let Backing::VectorStore { filter: f, .. } = &mut self.backing {
            *f = filter;
        }
        self
    }

    /// Convenience: embed and put a document. Only available on the
    /// `LongStore` backing.
    pub async fn upsert_doc(&self, key: &str, text: &str) -> Result<()> {
        match &self.backing {
            Backing::Store {
                store,
                embedder,
                namespace,
            } => {
                let v = embedder.embed(text).await?;
                store
                    .put(namespace, key, serde_json::json!({"text": text}), Some(v))
                    .await
            }
            Backing::VectorStore {
                vstore, embeddings, ..
            } => {
                let mut v = embeddings.embed(vec![text.to_string()]).await?;
                let emb = v.pop().unwrap_or_default();
                vstore
                    .upsert(vec![(key.to_string(), emb, serde_json::json!({"text": text}))])
                    .await
            }
        }
    }
}

#[async_trait]
impl Retriever for VectorRetriever {
    async fn retrieve(&self, query: &str, _ctx: &CallCtx) -> Result<Vec<Document>> {
        match &self.backing {
            Backing::Store {
                store,
                embedder,
                namespace,
            } => {
                let q = embedder.embed(query).await?;
                let hits = store.search(namespace, Some(&q), self.top_k).await?;
                Ok(hits
                    .into_iter()
                    .map(|i| Document {
                        id: i.key.clone(),
                        text: i
                            .value
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        metadata: i.value,
                        score: i.score,
                    })
                    .collect())
            }
            Backing::VectorStore {
                vstore,
                embeddings,
                filter,
            } => {
                let mut q = embeddings.embed(vec![query.to_string()]).await?;
                let emb = q.pop().unwrap_or_default();
                let hits = vstore.query(emb, self.top_k, filter).await?;
                Ok(hits
                    .into_iter()
                    .map(|h| Document {
                        id: h.id,
                        text: h
                            .metadata
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        metadata: h.metadata,
                        score: h.score,
                    })
                    .collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use atomr_agents_embed::{InMemoryVectorStore, MockEmbedder};
    use atomr_agents_memory::InMemoryLongStore;
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(5),
            trace: vec![],
            extensions: Default::default(),
        }
    }

    #[tokio::test]
    async fn vector_retriever_returns_topk_by_cosine() {
        let store: Arc<dyn LongStore> = Arc::new(InMemoryLongStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new(16));
        let ns = Namespace::from_parts(["kb"]);
        let r = VectorRetriever::new(store, embedder, ns, 2);
        r.upsert_doc("d1", "rust language").await.unwrap();
        r.upsert_doc("d2", "python data science").await.unwrap();
        r.upsert_doc("d3", "rust language").await.unwrap();
        let hits = r.retrieve("rust language", &ctx()).await.unwrap();
        assert_eq!(hits.len(), 2);
        // d1 or d3 at rank 0; both share the same hashed embedding text.
        assert!(hits[0].id == "d1" || hits[0].id == "d3");
    }

    #[tokio::test]
    async fn vector_retriever_from_injectable_vector_store() {
        let vstore: Arc<dyn VectorStore> = Arc::new(InMemoryVectorStore::new());
        let embeddings: Arc<dyn Embeddings> = Arc::new(MockEmbedder::new(16));
        let r = VectorRetriever::from_vector_store(vstore, embeddings, 2);
        r.upsert_doc("d1", "rust language").await.unwrap();
        r.upsert_doc("d2", "python data science").await.unwrap();
        r.upsert_doc("d3", "rust language").await.unwrap();
        let hits = r.retrieve("rust language", &ctx()).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].id == "d1" || hits[0].id == "d3");
        assert_eq!(hits[0].text, "rust language");
    }
}
