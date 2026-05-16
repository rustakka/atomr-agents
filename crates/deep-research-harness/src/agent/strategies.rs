//! Shared helpers for assembling an [`atomr_agents_agent::Agent`] from
//! a [`InferenceClientFactory`] + a per-role system prompt + tools.
//!
//! Every `AgentBased{Role}` walks the same recipe:
//!
//! 1. Resolve the model id for the role from `req.llm_overrides` (or
//!    fall back to a struct-level override).
//! 2. Build an [`InferenceClient`] via the caller-supplied factory.
//! 3. Compose a [`ComposedInstructionStrategy`] that bakes the
//!    role-specific system prompt into the persona slot.
//! 4. Wire a [`StaticToolStrategy`] (empty for Pattern B roles;
//!    populated with the relevant `ResearchToolSet` slice + optional
//!    web search for Pattern A roles).
//! 5. Use a tiny `RecencyMemoryStrategy` backed by an in-memory store
//!    plus an empty `StaticSkillStrategy` — both are no-ops at the
//!    scale of a single research run but satisfy the pipeline's type
//!    parameters.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_agent::{Agent, AgentBudgets, InferenceClient};
use atomr_agents_core::{AgentId, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use atomr_agents_deep_research_core::ResearchRequest;
use atomr_agents_instruction::{ComposedInstructionStrategy, StaticBehaviorStrategy, StaticTaskStrategy};
use atomr_agents_memory::{InMemoryStore, RecencyMemoryStrategy};
use atomr_agents_observability::EventBus;
use atomr_agents_persona::StaticPersonaStrategy;
use atomr_agents_skill::StaticSkillStrategy;
use atomr_agents_tool::{DynTool, StaticToolStrategy};

use crate::agent::factory::InferenceClientFactory;
use crate::error::{DeepResearchError, Result};

/// Concrete type of agent every `AgentBased{Role}` builds.
pub(crate) type RoleAgent = Agent<
    ComposedInstructionStrategy<StaticPersonaStrategy, StaticTaskStrategy, StaticBehaviorStrategy>,
    StaticToolStrategy,
    RecencyMemoryStrategy,
    StaticSkillStrategy,
>;

/// Resolve a model id for `role` from the request's overrides, falling
/// back to `role_default`.
pub(crate) fn resolve_model(req: &ResearchRequest, role: &str, role_default: Option<&str>) -> Result<String> {
    req.llm_overrides
        .role_model(role)
        .map(|s| s.to_string())
        .or_else(|| role_default.map(|s| s.to_string()))
        .ok_or_else(|| {
            DeepResearchError::config(format!(
                "no model id configured for role `{role}` (set llm_overrides.per_role[{role:?}] \
                 or llm_overrides.default_model, or pass with_model_id on the AgentBased role)"
            ))
        })
}

/// Build an [`Agent`] wired with `role_id` + `system_prompt` + `tools`.
pub(crate) fn build_agent(
    role_id: &str,
    model: String,
    system_prompt: String,
    inference: Arc<dyn InferenceClient>,
    tools: Vec<DynTool>,
    max_tool_iterations: u32,
) -> RoleAgent {
    let instructions = ComposedInstructionStrategy::new(
        StaticPersonaStrategy::new(system_prompt),
        StaticTaskStrategy(String::new()),
        StaticBehaviorStrategy(String::new()),
    );
    let memory = RecencyMemoryStrategy::new(Arc::new(InMemoryStore::new()), 4, 24);
    let skills = StaticSkillStrategy::new(vec![]);
    let tool_strat = StaticToolStrategy::new(tools);
    Agent {
        id: AgentId::from(role_id),
        model,
        instructions,
        tools: tool_strat,
        memory,
        skills,
        inference,
        bus: EventBus::new(),
        max_tool_iterations: max_tool_iterations.max(1),
    }
}

/// Default per-turn budgets — generous so the harness governs the
/// real ceilings via `ResearchRequest::time_budget` / `token_budget`.
pub(crate) fn default_budgets(req: &ResearchRequest) -> AgentBudgets {
    let token_cap = req.token_budget.unwrap_or(200_000).min(u32::MAX as u64) as u32;
    let time = req.time_budget.unwrap_or_else(|| Duration::from_secs(300));
    AgentBudgets {
        tokens: TokenBudget::new(token_cap),
        time: TimeBudget::new(time),
        money: MoneyBudget::from_usd(5.0),
        iterations: IterationBudget::new(32),
    }
}

/// Convenience: ask the factory for an inference client, mapping the
/// failure into a [`DeepResearchError::Config`].
pub(crate) fn make_inference(
    factory: &Arc<dyn InferenceClientFactory>,
    model: &str,
) -> Result<Arc<dyn InferenceClient>> {
    factory
        .build(model)
        .map_err(|e| DeepResearchError::config(format!("inference factory failed for model `{model}`: {e}")))
}
