//! Type-erased agent for config-driven instantiation.
//!
//! `Agent<I, T, Ms, Sk>` is monomorphic over four strategy traits — fast
//! to dispatch, awkward to construct from a builder. `BoxedAgent`
//! wraps an `Agent` whose strategy slots are `Box<dyn Trait>`, so a
//! Python binding / config loader can build an agent without spelling
//! out the concrete generic parameters at every call site.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentId, CallCtx, Result};
use atomr_agents_instruction::InstructionStrategy;
use atomr_agents_observability::EventBus;
use atomr_agents_strategy::{MemoryStrategy, SkillStrategy, ToolStrategy};

use crate::inference::{InferenceClient, TurnResult};
use crate::pipeline::{Agent, AgentBudgets};
use crate::r#trait::{AgentDispatch, AgentRef};

/// Convenient `Box<dyn>` alias for each strategy slot.
pub type BoxedInstruction = Box<dyn InstructionStrategy>;
pub type BoxedTools = Box<dyn ToolStrategy>;
pub type BoxedMemory = Box<dyn MemoryStrategy>;
pub type BoxedSkills = Box<dyn SkillStrategy>;

/// A type-erased agent. Implements `AgentDispatch` (and therefore
/// `Callable` via `AgentRef`).
pub struct BoxedAgent {
    pub(crate) inner: Agent<BoxedInstruction, BoxedTools, BoxedMemory, BoxedSkills>,
}

impl BoxedAgent {
    /// Build a boxed agent from its strategy slots, inference client,
    /// event bus, and tool-iteration cap.
    pub fn new(
        id: AgentId,
        model: String,
        instructions: BoxedInstruction,
        tools: BoxedTools,
        memory: BoxedMemory,
        skills: BoxedSkills,
        inference: Arc<dyn InferenceClient>,
        bus: EventBus,
        max_tool_iterations: u32,
    ) -> Self {
        Self {
            inner: Agent {
                id,
                model,
                instructions,
                tools,
                memory,
                skills,
                inference,
                bus,
                max_tool_iterations,
            },
        }
    }

    pub fn id(&self) -> &AgentId {
        &self.inner.id
    }

    /// Drive one turn through the per-turn pipeline.
    pub async fn run_turn(&self, user: String, budgets: AgentBudgets) -> Result<TurnResult> {
        self.inner.run_turn(user, budgets).await
    }

    /// Wrap as an `AgentRef` for use as a `Callable`.
    pub fn into_ref(self) -> AgentRef {
        let id = self.inner.id.clone();
        AgentRef::new(id, Arc::new(self))
    }
}

#[async_trait]
impl AgentDispatch for BoxedAgent {
    async fn dispatch(&self, user: String, ctx: CallCtx) -> Result<TurnResult> {
        self.run_turn(
            user,
            AgentBudgets {
                tokens: ctx.tokens,
                time: ctx.time,
                money: ctx.money,
                iterations: ctx.iterations,
            },
        )
        .await
    }
}
