use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryChunk, MemoryItem, Result, TokenBudget};

#[async_trait]
pub trait MemoryStrategy: Send + Sync + 'static {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Vec<MemoryChunk>>;

    async fn store(&self, item: MemoryItem) -> Result<()>;
}
