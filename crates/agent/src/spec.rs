use std::sync::Arc;
use std::time::Duration;

use atomr_agents_core::{AgentId, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use atomr_agents_instruction::InstructionStrategy;
use atomr_agents_observability::EventBus;
use atomr_agents_strategy::{MemoryStrategy, SkillStrategy, ToolStrategy};
use serde::{Deserialize, Serialize};

use crate::boxed::BoxedAgent;
use crate::inference::InferenceClient;
use crate::r#trait::AgentRef;

/// Static, serializable description of an agent. Used by the
/// registry and Python config; [`AgentSpec::into_agent`]
/// materializes a runnable [`AgentRef`]. Strategies and inference
/// client are passed in (typically constructed from a registry
/// lookup keyed off the spec's `id` / `model`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub id: AgentId,
    pub model: String,
    pub max_iterations: u32,
    pub token_budget: u32,
    pub time_budget_ms: u64,
    pub money_budget_usd: f64,
}

impl AgentSpec {
    pub fn default_budgets(&self) -> (TokenBudget, TimeBudget, MoneyBudget, IterationBudget) {
        (
            TokenBudget::new(self.token_budget),
            TimeBudget::new(Duration::from_millis(self.time_budget_ms)),
            MoneyBudget::from_usd(self.money_budget_usd),
            IterationBudget::new(self.max_iterations),
        )
    }

    /// Materialize a runnable [`AgentRef`] from this static spec
    /// plus a set of object-erased strategies and an inference
    /// client. Typically the strategies are constructed from a
    /// registry lookup keyed off the spec's `id` / `model`.
    pub fn into_agent(
        self,
        instructions: Box<dyn InstructionStrategy>,
        tools: Box<dyn ToolStrategy>,
        memory: Box<dyn MemoryStrategy>,
        skills: Box<dyn SkillStrategy>,
        inference: Arc<dyn InferenceClient>,
    ) -> AgentRef {
        let id = self.id.clone();
        let boxed = BoxedAgent {
            id: self.id,
            model: self.model,
            instructions,
            tools,
            memory,
            skills,
            inference,
            bus: EventBus::new(),
            max_tool_iterations: self.max_iterations,
        };
        AgentRef::new(id, Arc::new(boxed))
    }
}
