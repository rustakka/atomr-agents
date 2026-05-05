use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{AgentContext, Result};

/// One concrete routing target. Used by Org/Department/Team to fan
/// out a request.
#[derive(Clone)]
pub struct RoutingTarget {
    pub label: String,
    pub handle: CallableHandle,
}

#[async_trait]
pub trait RoutingStrategy: Send + Sync + 'static {
    async fn route(&self, ctx: &AgentContext) -> Result<RoutingTarget>;
}
