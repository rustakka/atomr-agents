use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryChunk, MemoryItem, MemoryNamespace, Result, TokenBudget};
use atomr_agents_strategy::MemoryStrategy;

use crate::store::MemoryStore;

/// Returns the N most-recent items for the agent's namespace, packed
/// to fit the budget.
pub struct RecencyMemoryStrategy {
    store: Arc<dyn MemoryStore>,
    limit: usize,
    /// Average tokens per item (used for budget accounting).
    tokens_per_item: u32,
}

impl RecencyMemoryStrategy {
    pub fn new(store: Arc<dyn MemoryStore>, limit: usize, tokens_per_item: u32) -> Self {
        Self {
            store,
            limit,
            tokens_per_item,
        }
    }
}

#[async_trait]
impl MemoryStrategy for RecencyMemoryStrategy {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Vec<MemoryChunk>> {
        let ns = MemoryNamespace::Agent(ctx.agent_id.clone());
        let items = self.store.list(&ns, self.limit).await?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            if budget.remaining < self.tokens_per_item {
                break;
            }
            budget.consume(self.tokens_per_item)?;
            out.push(MemoryChunk {
                source_id: item.id.clone(),
                text: format_payload(&item),
                score: 1.0,
                estimated_tokens: self.tokens_per_item,
            });
        }
        Ok(out)
    }

    async fn store(&self, item: MemoryItem) -> Result<()> {
        self.store.put(item).await
    }
}

fn format_payload(i: &MemoryItem) -> String {
    serde_json::to_string(&i.payload).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{AgentId, MemoryKind, TurnInput};
    use std::sync::Arc;

    use crate::store::InMemoryStore;

    fn item(id: &str, ts: i64, agent: &AgentId) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            kind: MemoryKind::Episodic,
            namespace: MemoryNamespace::Agent(agent.clone()),
            payload: serde_json::json!({"text": id}),
            timestamp_ms: ts,
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn recency_returns_newest_first_within_budget() {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let agent = AgentId::from("a-1");
        for (i, ts) in [(1, 100), (2, 200), (3, 300)].iter() {
            store.put(item(&format!("m-{i}"), *ts, &agent)).await.unwrap();
        }
        let strat = RecencyMemoryStrategy::new(store, 5, 50);
        let mut b = TokenBudget::new(120);
        let ctx = AgentContext::for_agent(
            agent,
            TurnInput {
                user: "what happened?".into(),
                history: vec![],
            },
        );
        let chunks = strat.retrieve(&ctx, &mut b).await.unwrap();
        // 120 / 50 = 2 chunks fit.
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].source_id, "m-3");
        assert_eq!(chunks[1].source_id, "m-2");
        // 20 tokens left after 2 * 50 consumed.
        assert_eq!(b.remaining, 20);
    }
}
