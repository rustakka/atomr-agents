use serde::{Deserialize, Serialize};

use crate::budget::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use crate::ids::{AgentId, OrgId, TeamId};
use crate::value::Value;

/// Conversation message — mirrors `atomr_infer_core::batch::Message`
/// but lives at this layer so strategies can construct turns without
/// pulling in the full inference crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// What a single agent turn consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnInput {
    pub user: String,
    #[serde(default)]
    pub history: Vec<Message>,
}

/// State the per-turn pipeline reads from. Strategies receive a
/// reference to this; they do not mutate it directly (they return
/// fragments which the `ContextAssembler` merges).
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub agent_id: AgentId,
    pub team_id: Option<TeamId>,
    pub org_id: Option<OrgId>,
    pub turn: TurnInput,
}

impl AgentContext {
    pub fn for_agent(agent_id: AgentId, turn: TurnInput) -> Self {
        Self { agent_id, team_id: None, org_id: None, turn }
    }
}

/// Context passed to `Callable::call`. Carries the budgets so a
/// callable can refuse work it can't afford.
#[derive(Debug, Clone)]
pub struct CallCtx {
    pub agent_id: Option<AgentId>,
    pub tokens: TokenBudget,
    pub time: TimeBudget,
    pub money: MoneyBudget,
    pub iterations: IterationBudget,
    pub trace: Vec<String>,
}

/// Context passed to `Tool::invoke`.
#[derive(Debug, Clone)]
pub struct InvokeCtx {
    pub call: CallCtx,
    pub tool_call_id: String,
    pub raw_args: Value,
}
