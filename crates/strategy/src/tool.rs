use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentContext, Result, TokenBudget, ToolId};

/// Reference to a selectable tool. The strategy returns these; the
/// agent dereferences the handle when it actually invokes one.
#[derive(Clone)]
pub struct ToolRef {
    pub id: ToolId,
    pub name: String,
    pub handle: CallableHandle,
}

#[async_trait]
pub trait ToolStrategy: Send + Sync + 'static {
    async fn select(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<Vec<ToolRef>>;
}
