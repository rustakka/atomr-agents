use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryChunk, MemoryItem, Result, TokenBudget};
use atomr_agents_strategy::MemoryStrategy;

/// Wraps another `MemoryStrategy` and emits a single summary chunk
/// instead of the underlying chunks when the byte size exceeds a
/// threshold. v0: trivial concatenate-and-truncate; an LLM-summarizer
/// variant lands once `agents-agent` is in place.
pub struct SummarizingMemoryStrategy<I: MemoryStrategy> {
    inner: I,
    max_summary_tokens: u32,
}

impl<I: MemoryStrategy> SummarizingMemoryStrategy<I> {
    pub fn new(inner: I, max_summary_tokens: u32) -> Self {
        Self { inner, max_summary_tokens }
    }
}

#[async_trait]
impl<I: MemoryStrategy> MemoryStrategy for SummarizingMemoryStrategy<I> {
    async fn retrieve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<Vec<MemoryChunk>> {
        let chunks = self.inner.retrieve(ctx, budget).await?;
        if chunks.is_empty() {
            return Ok(chunks);
        }
        let total_tokens: u32 = chunks.iter().map(|c| c.estimated_tokens).sum();
        if total_tokens <= self.max_summary_tokens {
            return Ok(chunks);
        }
        // Compress: concatenate the chunks' text and truncate to
        // `max_summary_tokens` * ~4 chars per token (rough).
        let joined: String = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let max_chars = (self.max_summary_tokens as usize).saturating_mul(4);
        let truncated = if joined.chars().count() > max_chars {
            joined.chars().take(max_chars).collect()
        } else {
            joined
        };
        Ok(vec![MemoryChunk {
            source_id: "summary".into(),
            text: truncated,
            score: 1.0,
            estimated_tokens: self.max_summary_tokens,
        }])
    }

    async fn store(&self, item: MemoryItem) -> Result<()> {
        self.inner.store(item).await
    }
}
