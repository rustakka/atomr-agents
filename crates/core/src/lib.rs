//! Core types for the atomr-agents framework.

mod budget;
mod context;
mod error;
mod event;
mod ids;
mod memory;
mod value;

pub use budget::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
pub use context::{AgentContext, CallCtx, InvokeCtx, Message, MessageRole, TurnInput};
pub use error::{AgentError, Result};
pub use event::{Event, EventEnvelope};
pub use ids::{
    AgentId, DepartmentId, HarnessId, OrgId, PersonaId, RunId, SkillId, TeamId, ToolId,
    ToolSetId, WorkflowId,
};
pub use memory::{MemoryChunk, MemoryItem, MemoryKind, MemoryNamespace};
pub use value::{Json, Value};

/// Re-exports of token/usage types from `atomr_infer_core` so downstream
/// crates have a single import path for them.
pub mod inference {
    pub use atomr_infer_core::tokens::{FinishReason, TokenChunk, TokenUsage, Tokens};
}
