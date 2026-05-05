use atomr_agents_core::{AgentId, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Static, serializable description of an agent. Used by the
/// registry and Python config; `Agent::from_spec` materializes the
/// runtime form.
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
}
