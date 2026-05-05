//! EnsembleRetriever — Reciprocal Rank Fusion across N retrievers.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};

use crate::retriever::{Document, Retriever};

pub struct EnsembleRetriever {
    pub members: Vec<Arc<dyn Retriever>>,
    pub k: f32,
    pub top_k: usize,
}

impl EnsembleRetriever {
    /// Default `k = 60` matches the canonical RRF paper.
    pub fn with_rrf(members: Vec<Arc<dyn Retriever>>, top_k: usize) -> Self {
        Self {
            members,
            k: 60.0,
            top_k,
        }
    }
}

#[async_trait]
impl Retriever for EnsembleRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let mut fused: HashMap<String, (Document, f32)> = HashMap::new();
        for r in &self.members {
            let hits = r.retrieve(query, ctx).await?;
            for (rank, h) in hits.iter().enumerate() {
                let contrib = 1.0 / (self.k + rank as f32 + 1.0);
                fused
                    .entry(h.id.clone())
                    .and_modify(|(_, s)| *s += contrib)
                    .or_insert_with(|| (h.clone(), contrib));
            }
        }
        let mut docs: Vec<Document> = fused
            .into_values()
            .map(|(mut d, s)| {
                d.score = s;
                d
            })
            .collect();
        docs.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        docs.truncate(self.top_k);
        Ok(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bm25::Bm25Retriever;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
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
    async fn rrf_combines_two_retrievers() {
        let a = Bm25Retriever::new(5);
        a.add(Document::new("d1", "rust"));
        a.add(Document::new("d2", "python"));
        let b = Bm25Retriever::new(5);
        b.add(Document::new("d2", "python"));
        b.add(Document::new("d1", "rust"));
        let e = EnsembleRetriever::with_rrf(
            vec![
                Arc::new(a) as Arc<dyn Retriever>,
                Arc::new(b) as Arc<dyn Retriever>,
            ],
            5,
        );
        let hits = e.retrieve("rust", &ctx()).await.unwrap();
        // d1 ranks first in both → highest fused score.
        assert!(!hits.is_empty());
    }
}
