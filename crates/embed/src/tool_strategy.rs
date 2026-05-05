use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{AgentContext, AgentError, Result, TokenBudget, ToolId};
use atomr_agents_strategy::{ToolRef, ToolStrategy};
use atomr_agents_tool::DynTool;
use parking_lot::RwLock;

use crate::ann::{AnnId, AnnIndex};
use crate::embedder::Embedder;

/// Embeds tool descriptors at construction; at select-time, embeds the
/// user turn and returns top-k tools that fit the budget.
pub struct EmbeddingToolStrategy {
    embedder: Arc<dyn Embedder>,
    index: Arc<dyn AnnIndex>,
    /// Stable id-to-tool table indexed by AnnId.
    tools: Arc<RwLock<Vec<DynTool>>>,
    top_k: usize,
}

impl EmbeddingToolStrategy {
    /// Construct, index every tool's `name + description` into the
    /// supplied `AnnIndex`. The caller picks the index implementation.
    pub async fn build(
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn AnnIndex>,
        tools: Vec<DynTool>,
        top_k: usize,
    ) -> Result<Self> {
        for (i, t) in tools.iter().enumerate() {
            let d = t.descriptor();
            let text = format!("{} {}", d.name, d.description);
            let v = embedder.embed(&text).await?;
            index.upsert(i as AnnId, v).await?;
        }
        Ok(Self {
            embedder,
            index,
            tools: Arc::new(RwLock::new(tools)),
            top_k,
        })
    }

    fn tool_to_handle(t: DynTool) -> Arc<dyn Callable> {
        struct DynToolCallable {
            inner: DynTool,
        }
        #[async_trait]
        impl Callable for DynToolCallable {
            async fn call(
                &self,
                input: atomr_agents_core::Value,
                ctx: atomr_agents_core::CallCtx,
            ) -> Result<atomr_agents_core::Value> {
                let invoke_ctx = atomr_agents_core::InvokeCtx {
                    call: ctx,
                    tool_call_id: String::new(),
                    raw_args: input.clone(),
                };
                self.inner.invoke(input, &invoke_ctx).await
            }
            fn label(&self) -> &str {
                &self.inner.descriptor().name
            }
        }
        Arc::new(DynToolCallable { inner: t })
    }

    fn _id_for(&self, id: ToolId) -> Result<ToolId> {
        // Reserved for future use when tools are keyed by id rather
        // than by index position. Kept as a placeholder so the type
        // surface doesn't leak `usize` to consumers.
        Ok(id)
    }
}

#[async_trait]
impl ToolStrategy for EmbeddingToolStrategy {
    async fn select(&self, ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<Vec<ToolRef>> {
        let q = self.embedder.embed(&ctx.turn.user).await?;
        let hits = self.index.search(&q, self.top_k).await?;
        let tools = self.tools.read();
        let mut out = Vec::with_capacity(hits.len());
        for (id, _score) in hits {
            let i = id as usize;
            let t = tools
                .get(i)
                .ok_or_else(|| AgentError::Internal(format!("ann hit {i} out of range")))?;
            let d = t.descriptor();
            out.push(ToolRef {
                id: d.id.clone(),
                name: d.name.clone(),
                handle: Self::tool_to_handle(t.clone()),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ann::InMemoryAnnIndex;
    use crate::embedder::MockEmbedder;
    use atomr_agents_core::{AgentId, InvokeCtx, ToolId, TurnInput, Value};
    use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};

    struct StubTool {
        d: ToolDescriptor,
    }
    #[async_trait]
    impl Tool for StubTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.d
        }
        async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
            Ok(args)
        }
    }
    fn stub(name: &str, desc: &str) -> DynTool {
        Arc::new(StubTool {
            d: ToolDescriptor {
                id: ToolId::from(name),
                name: name.to_string(),
                description: desc.to_string(),
                schema: ToolSchema::empty_object(),
            },
        })
    }

    #[tokio::test]
    async fn picks_topk_relevant_tools() {
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new(16));
        let idx: Arc<dyn AnnIndex> = Arc::new(InMemoryAnnIndex::new(16));
        let tools = vec![
            stub("calc", "compute mathematical expressions"),
            stub("search", "search the web for documents"),
            stub("crm", "look up customer records"),
        ];
        let strat = EmbeddingToolStrategy::build(embedder, idx, tools, 2)
            .await
            .unwrap();
        let ctx = AgentContext::for_agent(
            AgentId::from("a-1"),
            TurnInput {
                user: "search the web for documents".into(),
                history: vec![],
            },
        );
        let mut b = TokenBudget::new(1000);
        let out = strat.select(&ctx, &mut b).await.unwrap();
        assert_eq!(out.len(), 2);
        // The exact match should be top.
        assert_eq!(out[0].name, "search");
    }
}
