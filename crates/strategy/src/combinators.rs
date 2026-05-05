use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryChunk, MemoryItem, Result, TokenBudget};

use crate::memory::MemoryStrategy;

/// Compose memory strategies sequentially. Reads are unioned; writes
/// fan out to every member.
pub struct ChainedMemoryStrategy {
    members: Vec<Box<dyn MemoryStrategy>>,
}

impl ChainedMemoryStrategy {
    pub fn new(members: Vec<Box<dyn MemoryStrategy>>) -> Self {
        Self { members }
    }
}

#[async_trait]
impl MemoryStrategy for ChainedMemoryStrategy {
    async fn retrieve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<Vec<MemoryChunk>> {
        let mut out = Vec::new();
        for m in &self.members {
            let chunks = m.retrieve(ctx, budget).await?;
            out.extend(chunks);
        }
        Ok(out)
    }

    async fn store(&self, item: MemoryItem) -> Result<()> {
        for m in &self.members {
            m.store(item.clone()).await?;
        }
        Ok(())
    }
}
