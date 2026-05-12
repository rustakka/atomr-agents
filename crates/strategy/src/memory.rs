use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryChunk, MemoryItem, Result, TokenBudget};

#[async_trait]
pub trait MemoryStrategy: Send + Sync + 'static {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Vec<MemoryChunk>>;

    async fn store(&self, item: MemoryItem) -> Result<()>;
}

#[async_trait]
impl MemoryStrategy for Box<dyn MemoryStrategy> {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Vec<MemoryChunk>> {
        (**self).retrieve(ctx, budget).await
    }
    async fn store(&self, item: MemoryItem) -> Result<()> {
        (**self).store(item).await
    }
}
