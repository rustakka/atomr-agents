//! Instruction strategy + the (Persona, Task, Behavior) composer +
//! ChatPromptTemplate / FewShotChatTemplate.

mod prompt_template;

pub use prompt_template::{
    ChatPromptTemplate, ChatPromptTemplateBuilder, Example, ExampleSelector, FewShotChatTemplate,
    LengthBasedSelector, MessageTemplate, MessagesPlaceholder, RenderedMessage, SemanticSimilaritySelector,
    StringTemplate,
};

use async_trait::async_trait;
use atomr_agents_context::{ContextAssembler, ContextFragment, RenderedContext};
use atomr_agents_core::{AgentContext, Result, TokenBudget};
use atomr_agents_persona::{PersonaStrategy, RenderedPersona};

/// What `InstructionStrategy::render` returns. The agent's per-turn
/// pipeline feeds this into the `ContextAssembler`.
#[derive(Debug, Clone, Default)]
pub struct RenderedInstructions {
    pub system_prompt: String,
    pub estimated_tokens: u32,
}

#[async_trait]
pub trait InstructionStrategy: Send + Sync + 'static {
    async fn render(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<RenderedInstructions>;
}

/// Resolves `task` for `ComposedInstructionStrategy`.
#[async_trait]
pub trait TaskStrategy: Send + Sync + 'static {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<String>;
}

/// Resolves `behavior` for `ComposedInstructionStrategy`.
#[async_trait]
pub trait BehaviorStrategy: Send + Sync + 'static {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<String>;
}

/// Pluggable assembler so callers can pick how the three slots merge.
pub trait InstructionAssembler: Send + Sync + 'static {
    fn assemble(
        &self,
        persona: RenderedPersona,
        task: String,
        behavior: String,
        budget: &mut TokenBudget,
    ) -> Result<RenderedInstructions>;
}

/// Default assembler: priority is task > persona > behavior. Falls
/// back gracefully under budget pressure via `ContextAssembler`.
pub struct DefaultAssembler;

impl InstructionAssembler for DefaultAssembler {
    fn assemble(
        &self,
        persona: RenderedPersona,
        task: String,
        behavior: String,
        budget: &mut TokenBudget,
    ) -> Result<RenderedInstructions> {
        let frags = vec![
            ContextFragment {
                source: "task",
                priority: 9,
                estimated_tokens: ((task.chars().count() + 3) / 4) as u32,
                text: task,
            },
            ContextFragment {
                source: "persona",
                priority: 6,
                estimated_tokens: persona.estimated_tokens,
                text: persona.identity,
            },
            ContextFragment {
                source: "behavior",
                priority: 4,
                estimated_tokens: ((behavior.chars().count() + 3) / 4) as u32,
                text: behavior,
            },
        ];
        let r: RenderedContext = ContextAssembler::assemble(frags, budget)?;
        Ok(RenderedInstructions {
            system_prompt: r.join("\n\n"),
            estimated_tokens: r.total_tokens,
        })
    }
}

pub struct ComposedInstructionStrategy<P, T, B>
where
    P: PersonaStrategy,
    T: TaskStrategy,
    B: BehaviorStrategy,
{
    pub persona: P,
    pub task: T,
    pub behavior: B,
    pub assembler: Box<dyn InstructionAssembler>,
}

impl<P, T, B> ComposedInstructionStrategy<P, T, B>
where
    P: PersonaStrategy,
    T: TaskStrategy,
    B: BehaviorStrategy,
{
    pub fn new(persona: P, task: T, behavior: B) -> Self {
        Self {
            persona,
            task,
            behavior,
            assembler: Box::new(DefaultAssembler),
        }
    }
}

#[async_trait]
impl<P, T, B> InstructionStrategy for ComposedInstructionStrategy<P, T, B>
where
    P: PersonaStrategy,
    T: TaskStrategy,
    B: BehaviorStrategy,
{
    async fn render(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<RenderedInstructions> {
        // Persona / task / behavior share the parent budget cooperatively.
        let mut subs = budget.split(3);
        let (mut bp, mut bt, mut bb) = (subs.remove(0), subs.remove(0), subs.remove(0));
        let bp_initial = bp.remaining;
        let bt_initial = bt.remaining;
        let bb_initial = bb.remaining;
        let (persona, task, behavior) = tokio::join!(
            self.persona.resolve(ctx, &mut bp),
            self.task.resolve(ctx, &mut bt),
            self.behavior.resolve(ctx, &mut bb),
        );
        let persona = persona?;
        let task = task?;
        let behavior = behavior?;
        let consumed = bp_initial.saturating_sub(bp.remaining)
            + bt_initial.saturating_sub(bt.remaining)
            + bb_initial.saturating_sub(bb.remaining);
        let take = consumed.min(budget.remaining);
        budget.consume(take).ok();
        self.assembler.assemble(persona, task, behavior, budget)
    }
}

// ---- Convenience strategies ----

pub struct StaticTaskStrategy(pub String);

#[async_trait]
impl TaskStrategy for StaticTaskStrategy {
    async fn resolve(&self, _ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<String> {
        Ok(self.0.clone())
    }
}

pub struct StaticBehaviorStrategy(pub String);

#[async_trait]
impl BehaviorStrategy for StaticBehaviorStrategy {
    async fn resolve(&self, _ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<String> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{AgentId, TurnInput};
    use atomr_agents_persona::StaticPersonaStrategy;

    #[tokio::test]
    async fn composed_assembles_task_persona_behavior() {
        let composed = ComposedInstructionStrategy::new(
            StaticPersonaStrategy::new("You are a helpful assistant."),
            StaticTaskStrategy("Answer the user's question.".into()),
            StaticBehaviorStrategy("Be concise.".into()),
        );
        let ctx = AgentContext::for_agent(
            AgentId::from("a-1"),
            TurnInput {
                user: "hi".into(),
                history: vec![],
            },
        );
        let mut b = TokenBudget::new(2000);
        let r = composed.render(&ctx, &mut b).await.unwrap();
        assert!(r.system_prompt.contains("Answer"));
        assert!(r.system_prompt.contains("helpful"));
        assert!(r.system_prompt.contains("concise"));
    }

    #[tokio::test]
    async fn under_budget_pressure_task_survives() {
        let composed = ComposedInstructionStrategy::new(
            StaticPersonaStrategy::new("Long persona ".repeat(50).trim_end().to_string()),
            StaticTaskStrategy("Solve the user's request.".into()),
            StaticBehaviorStrategy("Long behavior ".repeat(50).trim_end().to_string()),
        );
        let ctx = AgentContext::for_agent(
            AgentId::from("a-1"),
            TurnInput {
                user: "x".into(),
                history: vec![],
            },
        );
        // Tight budget — only ~10 tokens of room after persona/behavior split.
        let mut b = TokenBudget::new(40);
        let r = composed.render(&ctx, &mut b).await.unwrap();
        assert!(
            r.system_prompt.contains("Solve"),
            "task fragment should survive budget pressure: {:?}",
            r.system_prompt
        );
    }
}
