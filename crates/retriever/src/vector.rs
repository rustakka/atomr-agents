//! Dense retriever backed by `LongStore` semantic search.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};
use atomr_agents_embed::Embedder;
use atomr_agents_memory::{LongStore, Namespace};

use crate::retriever::{Document, Retriever};

pub struct VectorRetriever {
    pub store: Arc<dyn LongStore>,
    pub embedder: Arc<dyn Embedder>,
    pub namespace: Namespace,
    pub top_k: usize,
}

impl VectorRetriever {
    pub fn new(
        store: Arc<dyn LongStore>,
        embedder: Arc<dyn Embedder>,
        namespace: Namespace,
        top_k: usize,
    ) -> Self {
        Self { store, embedder, namespace, top_k }
    }

    /// Convenience: embed and put a document under the configured namespace.
    pub async fn upsert_doc(&self, key: &str, text: &str) -> Result<()> {
        let v = self.embedder.embed(text).await?;
        self.store
            .put(
                &self.namespace,
                key,
                serde_json::json!({"text": text}),
                Some(v),
            )
            .await
    }
}

#[async_trait]
impl Retriever for VectorRetriever {
    async fn retrieve(&self, query: &str, _ctx: &CallCtx) -> Result<Vec<Document>> {
        let q = self.embedder.embed(query).await?;
        let hits = self.store.search(&self.namespace, Some(&q), self.top_k).await?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use atomr_agents_embed::MockEmbedder;
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
}
