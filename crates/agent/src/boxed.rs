//! Object-erased agent: a config-driven counterpart to the typed
//! `Agent<I,T,Ms,Sk>`. Useful when the strategy concrete types
//! aren't known at the construction site (e.g. Python config
//! loaders or registry-driven instantiation).
//!
//! The hot path is unchanged for the typed `Agent<I,T,Ms,Sk>` —
//! both forms funnel into [`crate::pipeline::run_turn_impl`]. Only
//! the strategy method calls inside that impl become indirect, and
//! they happen ~4 times per turn (not in a tight loop).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentId, CallCtx, Result};
use atomr_agents_instruction::InstructionStrategy;
use atomr_agents_observability::EventBus;
use atomr_agents_strategy::{MemoryStrategy, SkillStrategy, ToolStrategy};

use crate::inference::{InferenceClient, TurnResult};
use crate::pipeline::{run_turn_impl, AgentBudgets};
use crate::r#trait::AgentDispatch;

/// Fully-erased agent. Mirrors the field shape of
/// [`crate::Agent`] but stores each strategy as a trait object so
/// callers without the concrete strategy types can still construct
/// a runnable agent.
pub struct BoxedAgent {
    pub id: AgentId,
    pub model: String,
    pub instructions: Box<dyn InstructionStrategy>,
    pub tools: Box<dyn ToolStrategy>,
    pub memory: Box<dyn MemoryStrategy>,
    pub skills: Box<dyn SkillStrategy>,
    pub inference: Arc<dyn InferenceClient>,
    pub bus: EventBus,
    pub max_tool_iterations: u32,
}

impl BoxedAgent {
    /// One full agent turn — see [`crate::Agent::run_turn`].
    pub async fn run_turn(&self, user: String, budgets: AgentBudgets) -> Result<TurnResult> {
        run_turn_impl(
            &self.id,
            &self.model,
            &*self.instructions,
            &*self.tools,
            &*self.memory,
            &*self.skills,
            &self.inference,
            &self.bus,
            self.max_tool_iterations,
            user,
            budgets,
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use atomr_agents_core::{
        InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, ToolId, Value,
    };
    use atomr_agents_instruction::{ComposedInstructionStrategy, StaticBehaviorStrategy, StaticTaskStrategy};
    use atomr_agents_memory::{InMemoryStore, RecencyMemoryStrategy};
    use atomr_agents_persona::StaticPersonaStrategy;
    use atomr_agents_skill::StaticSkillStrategy;
    use atomr_agents_tool::{DynTool, Provider, StaticToolStrategy, Tool, ToolDescriptor, ToolSchema};

    use crate::inference::LocalRunnerClient;
    use atomr_infer_testkit::{MockRunner, MockScript};

    /// Trivial calculator for the BoxedAgent end-to-end test (mirrors
    /// the typed `Agent` test fixture in `pipeline.rs`).
    struct CalculatorTool {
        d: ToolDescriptor,
    }
    impl CalculatorTool {
        fn new() -> Self {
            Self {
                d: ToolDescriptor {
                    id: ToolId::from("calculator"),
                    name: "calculator".into(),
                    description: "evaluate simple arithmetic".into(),
                    schema: ToolSchema::empty_object(),
                },
            }
        }
    }
    #[async_trait]
    impl Tool for CalculatorTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.d
        }
        async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
            let a = args.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
            let b = args.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(serde_json::json!({"sum": a + b}))
        }
    }

    fn build_boxed_agent(runner: MockRunner) -> BoxedAgent {
        let store = Arc::new(InMemoryStore::new());
        let mem = RecencyMemoryStrategy::new(store, 5, 30);
        let tools: Vec<DynTool> = vec![Arc::new(CalculatorTool::new())];
        let tool_strat = StaticToolStrategy::new(tools);
        let instr = ComposedInstructionStrategy::new(
            StaticPersonaStrategy::new("You are a calculator assistant."),
            StaticTaskStrategy("Use tools to answer arithmetic questions.".into()),
            StaticBehaviorStrategy("Reply tersely.".into()),
        );
        let skill_strat = StaticSkillStrategy::new(vec![]);
        let inference: Arc<dyn InferenceClient> = Arc::new(LocalRunnerClient::new(runner, Provider::OpenAi));
        BoxedAgent {
            id: AgentId::from("boxed-1"),
            model: "mock".into(),
            instructions: Box::new(instr),
            tools: Box::new(tool_strat),
            memory: Box::new(mem),
            skills: Box::new(skill_strat),
            inference,
            bus: EventBus::new(),
            max_tool_iterations: 3,
        }
    }

    #[tokio::test]
    async fn boxed_agent_runs_simple_text_turn() {
        let runner = MockRunner::new(MockScript::from_text(["the answer is ", "42"]));
        let agent = build_boxed_agent(runner);
        let r = agent
            .run_turn(
                "what's 1+2".into(),
                AgentBudgets {
                    tokens: TokenBudget::new(10_000),
                    time: TimeBudget::new(std::time::Duration::from_secs(30)),
                    money: MoneyBudget::from_usd(1.0),
                    iterations: IterationBudget::new(5),
                },
            )
            .await
            .unwrap();
        assert!(r.text.contains("42"));
        assert_eq!(r.usage.output_tokens, 2);
    }

    #[tokio::test]
    async fn boxed_agent_dispatches_through_trait() {
        let runner = MockRunner::new(MockScript::from_text(["pong"]));
        let agent = build_boxed_agent(runner);
        let ctx = CallCtx {
            agent_id: Some(agent.id.clone()),
            tokens: TokenBudget::new(10_000),
            time: TimeBudget::new(std::time::Duration::from_secs(30)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(5),
            trace: vec![],
        };
        let r = AgentDispatch::dispatch(&agent, "ping".into(), ctx).await.unwrap();
        assert_eq!(r.text, "pong");
    }
}
