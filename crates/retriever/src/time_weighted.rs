//! TimeWeightedRetriever — recency-decayed scoring on top of any
//! base retriever. Each document carries a `ts_ms` field in metadata.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};

use crate::retriever::{Document, Retriever};

pub struct TimeWeightedRetriever {
    pub base: Arc<dyn Retriever>,
    /// Decay rate; higher = stronger recency preference.
    pub decay_rate: f32,
}

impl TimeWeightedRetriever {
    pub fn new(base: Arc<dyn Retriever>, decay_rate: f32) -> Self {
        Self { base, decay_rate }
    }
}

#[async_trait]
impl Retriever for TimeWeightedRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let mut docs = self.base.retrieve(query, ctx).await?;
        let now = now_ms();
        for d in &mut docs {
            let age_h = d
                .metadata
                .get("ts_ms")
                .and_then(|v| v.as_i64())
                .map(|t| ((now - t).max(0) as f32) / (1000.0 * 60.0 * 60.0))
                .unwrap_or(0.0);
            let decay = (-self.decay_rate * age_h).exp();
            d.score *= decay;
        }
        docs.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(docs)
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
    async fn time_weighted_prefers_recent() {
        let bm = Bm25Retriever::new(5);
        let now = now_ms();
        let mut old = Document::new("old", "rust");
        old.metadata = serde_json::json!({"ts_ms": now - 1000 * 60 * 60 * 24 * 30});
        let mut fresh = Document::new("fresh", "rust");
        fresh.metadata = serde_json::json!({"ts_ms": now});
        bm.add(old);
        bm.add(fresh);
        let r = TimeWeightedRetriever::new(Arc::new(bm), 0.05);
        let hits = r.retrieve("rust", &ctx()).await.unwrap();
        assert_eq!(hits[0].id, "fresh");
    }
}
