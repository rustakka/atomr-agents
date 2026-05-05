//! MultiQuery: expand the query into N variants, union results.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};

use crate::retriever::{Document, Retriever};

#[async_trait]
pub trait QueryExpander: Send + Sync + 'static {
    async fn expand(&self, query: &str, n: usize) -> Result<Vec<String>>;
}

/// Trivial expander for tests: appends suffix variants.
pub struct StaticExpander {
    pub variants: Vec<String>,
}

#[async_trait]
impl QueryExpander for StaticExpander {
    async fn expand(&self, query: &str, _n: usize) -> Result<Vec<String>> {
        Ok(self.variants.iter().map(|v| format!("{} {}", query, v)).collect())
    }
}

pub struct MultiQueryRetriever {
    pub base: Arc<dyn Retriever>,
    pub expander: Arc<dyn QueryExpander>,
    pub n_variants: usize,
}

impl MultiQueryRetriever {
    pub fn new(base: Arc<dyn Retriever>, expander: Arc<dyn QueryExpander>, n_variants: usize) -> Self {
        Self {
            base,
            expander,
            n_variants,
        }
    }
}

#[async_trait]
impl Retriever for MultiQueryRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let mut queries = self.expander.expand(query, self.n_variants).await?;
        // Always include the original.
        queries.insert(0, query.to_string());
        let mut union: HashMap<String, Document> = HashMap::new();
        for q in queries {
            let hits = self.base.retrieve(&q, ctx).await?;
            for h in hits {
                union
                    .entry(h.id.clone())
                    .and_modify(|existing| {
                        if h.score > existing.score {
                            existing.score = h.score;
                        }
                    })
                    .or_insert(h);
            }
        }
        let mut out: Vec<Document> = union.into_values().collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
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
    async fn multi_query_unions_with_variants() {
        let bm = Bm25Retriever::new(5);
        bm.add(Document::new("d1", "python pandas data analysis"));
        bm.add(Document::new("d2", "rust async runtime tokio"));
        let base: Arc<dyn Retriever> = Arc::new(bm);
        let expander: Arc<dyn QueryExpander> = Arc::new(StaticExpander {
            variants: vec!["pandas".into(), "tokio".into()],
        });
        let m = MultiQueryRetriever::new(base, expander, 2);
        let hits = m.retrieve("data", &ctx()).await.unwrap();
        let ids: std::collections::HashSet<_> = hits.iter().map(|d| d.id.clone()).collect();
        assert!(ids.contains("d1"));
        assert!(ids.contains("d2"));
    }
}
