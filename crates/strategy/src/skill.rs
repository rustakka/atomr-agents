use async_trait::async_trait;
use atomr_agents_core::{AgentContext, Result, SkillId, TokenBudget};

/// Skill reference returned by `SkillStrategy::applicable`.
/// The full skill definition (instructions fragment, tool overlay,
/// sub-agents) lives in `atomr-agents-skill` and is resolved by id.
#[derive(Debug, Clone)]
pub struct SkillRef {
    pub id: SkillId,
    pub name: String,
    pub priority: u8,
}

#[async_trait]
pub trait SkillStrategy: Send + Sync + 'static {
    async fn applicable(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<Vec<SkillRef>>;
}
