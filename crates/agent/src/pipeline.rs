//! Per-turn pipeline implementation.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use atomr_agents_context::{ContextAssembler, ContextFragment};
use atomr_agents_core::{
    AgentContext, AgentError, AgentId, CallCtx, Event, IterationBudget, Json, MemoryItem, MemoryKind,
    MemoryNamespace, MoneyBudget, Result, TimeBudget, TokenBudget, ToolId, TurnInput,
};
use atomr_agents_instruction::InstructionStrategy;
use atomr_agents_observability::EventBus;
use atomr_agents_strategy::{MemoryStrategy, SkillStrategy, ToolStrategy};
use atomr_infer_core::batch::{ExecuteBatch, Message as InferMsg, MessageContent, Role, SamplingParams};

use crate::inference::{InferenceClient, TurnResult};
use crate::r#trait::AgentDispatch;

/// Generic agent. Strategy types are monomorphized for the hot
/// path; a `BoxedAgent` form (using `Box<dyn>` for each slot) is
/// produced from `AgentSpec`.
pub struct Agent<I, T, Ms, Sk>
where
    I: InstructionStrategy,
    T: ToolStrategy,
    Ms: MemoryStrategy,
    Sk: SkillStrategy,
{
    pub id: AgentId,
    pub model: String,
    pub instructions: I,
    pub tools: T,
    pub memory: Ms,
    pub skills: Sk,
    pub inference: Arc<dyn InferenceClient>,
    pub bus: EventBus,
    pub max_tool_iterations: u32,
}

impl<I, T, Ms, Sk> Agent<I, T, Ms, Sk>
where
    I: InstructionStrategy,
    T: ToolStrategy,
    Ms: MemoryStrategy,
    Sk: SkillStrategy,
{
    /// One full agent turn. Drives the per-turn pipeline (memory +
    /// skill + tool resolution → instruction render → context
    /// assembly → inference → tool-call loop → memory store).
    pub async fn run_turn(&self, user: String, budgets: AgentBudgets) -> Result<TurnResult> {
        let start = Instant::now();
        let agent_ctx = AgentContext::for_agent(
            self.id.clone(),
            TurnInput {
                user: user.clone(),
                history: vec![],
            },
        );
        let AgentBudgets {
            mut tokens,
            time,
            money,
            mut iterations,
        } = budgets;

        // 1. Parallel strategy resolution.
        let mut subs = tokens.split(3);
        let (mut bm, mut bs, mut bt) = (subs.remove(0), subs.remove(0), subs.remove(0));
        let bm0 = bm.remaining;
        let bs0 = bs.remaining;
        let bt0 = bt.remaining;
        let (mem, skills, tool_refs) = tokio::join!(
            self.memory.retrieve(&agent_ctx, &mut bm),
            self.skills.applicable(&agent_ctx, &mut bs),
            self.tools.select(&agent_ctx, &mut bt),
        );
        let mem = mem?;
        let _skills = skills?;
        let tool_refs = tool_refs?;
        let consumed = bm0.saturating_sub(bm.remaining)
            + bs0.saturating_sub(bs.remaining)
            + bt0.saturating_sub(bt.remaining);
        tokens.consume(consumed.min(tokens.remaining)).ok();

        // 2. Render instructions.
        let mut instr_budget = tokens.split(2).remove(0);
        let r_instr = self.instructions.render(&agent_ctx, &mut instr_budget).await?;
        tokens
            .consume(r_instr.estimated_tokens.min(tokens.remaining))
            .ok();

        // 3. Assemble final context (system prompt + recalled memory).
        let mut frags = vec![ContextFragment {
            source: "system",
            priority: 9,
            estimated_tokens: r_instr.estimated_tokens,
            text: r_instr.system_prompt.clone(),
        }];
        for c in &mem {
            frags.push(ContextFragment {
                source: "memory",
                priority: 5,
                estimated_tokens: c.estimated_tokens,
                text: c.text.clone(),
            });
        }
        let assembled = ContextAssembler::assemble(frags, &mut tokens)?;

        // 4. Build initial messages.
        let mut messages: Vec<InferMsg> = Vec::new();
        messages.push(InferMsg {
            role: Role::System,
            content: MessageContent::Text(assembled.join("\n\n")),
        });
        messages.push(InferMsg {
            role: Role::User,
            content: MessageContent::Text(user.clone()),
        });

        // 5. Tool-call loop.
        let mut final_text = String::new();
        let mut final_usage = atomr_infer_core::tokens::TokenUsage::default();
        let mut final_finish = None;
        for _iter in 0..self.max_tool_iterations.max(1) {
            iterations.consume_one()?;
            let batch = ExecuteBatch {
                request_id: format!("turn-{}", uuid_str()),
                model: self.model.clone(),
                messages: messages.clone(),
                sampling: SamplingParams::default(),
                stream: true,
                estimated_tokens: tokens.remaining,
            };
            let r = self.inference.run(batch).await?;
            final_text = r.text.clone();
            final_usage.add(r.usage);
            final_finish = r.finish_reason;
            // Stop conditions.
            if r.tool_calls.is_empty()
                || r.finish_reason != Some(atomr_infer_core::tokens::FinishReason::ToolCalls)
            {
                break;
            }
            // Append the assistant's tool-call turn (for provider
            // history coherence) and dispatch each tool — concurrently
            // when multiple are emitted, order-preserved on aggregation.
            messages.push(InferMsg {
                role: Role::Assistant,
                content: MessageContent::Text(r.text.clone()),
            });
            let mut handles: Vec<tokio::task::JoinHandle<Result<(usize, String, Json::Value, u64, u64)>>> =
                Vec::with_capacity(r.tool_calls.len());
            for (idx, call) in r.tool_calls.iter().enumerate() {
                let tool_ref = tool_refs
                    .iter()
                    .find(|t| t.name == call.name)
                    .ok_or_else(|| AgentError::Tool(format!("unknown tool: {}", call.name)))?;
                let args = call.arguments().unwrap_or(Json::Value::Null);
                let invoke_ctx = CallCtx {
                    agent_id: Some(self.id.clone()),
                    tokens,
                    time,
                    money,
                    iterations,
                    trace: vec![format!("tool:{}", call.name)],
                };
                let handle = tool_ref.handle.clone();
                let name = call.name.clone();
                let args_for_task = args.clone();
                handles.push(tokio::spawn(async move {
                    let t0 = Instant::now();
                    let result = handle.call(args_for_task.clone(), invoke_ctx).await?;
                    Ok::<_, AgentError>((
                        idx,
                        name,
                        result,
                        hash_value(&args_for_task),
                        t0.elapsed().as_millis() as u64,
                    ))
                }));
            }
            let mut results: Vec<(usize, String, Json::Value, u64, u64)> = Vec::with_capacity(handles.len());
            for h in handles {
                let pair = h.await.map_err(|e| AgentError::Internal(e.to_string()))??;
                results.push(pair);
            }
            results.sort_by_key(|(i, _, _, _, _)| *i);
            for (_, name, result, args_hash, elapsed_ms) in results {
                self.bus.emit(Event::ToolInvoked {
                    tool_id: ToolId::from(name.as_str()),
                    args_hash,
                    elapsed_ms,
                    ok: true,
                });
                messages.push(InferMsg {
                    role: Role::Tool,
                    content: MessageContent::Text(serde_json::to_string(&result).unwrap_or_default()),
                });
            }
        }

        // 6. Memory store.
        let item = MemoryItem {
            id: format!("turn-{}", uuid_str()),
            kind: MemoryKind::Episodic,
            namespace: MemoryNamespace::Agent(self.id.clone()),
            payload: serde_json::json!({"user": user, "assistant": final_text}),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            tags: vec![],
        };
        self.memory.store(item).await.ok();

        // 7. Emit AgentTurn event.
        self.bus.emit(Event::AgentTurn {
            agent_id: self.id.clone(),
            input_tokens: final_usage.input_tokens,
            output_tokens: final_usage.output_tokens,
            finish_reason: final_finish,
            elapsed_ms: start.elapsed().as_millis() as u64,
        });

        Ok(TurnResult {
            text: final_text,
            usage: final_usage,
            finish_reason: final_finish,
            tool_calls: vec![],
        })
    }
}

#[async_trait]
impl<I, T, Ms, Sk> AgentDispatch for Agent<I, T, Ms, Sk>
where
    I: InstructionStrategy,
    T: ToolStrategy,
    Ms: MemoryStrategy,
    Sk: SkillStrategy,
{
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

#[derive(Debug, Clone, Copy)]
pub struct AgentBudgets {
    pub tokens: TokenBudget,
    pub time: TimeBudget,
    pub money: MoneyBudget,
    pub iterations: IterationBudget,
}

fn uuid_str() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{:016x}", N.fetch_add(1, Ordering::Relaxed))
}

fn hash_value(v: &Json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use async_trait::async_trait;
    use atomr_agents_core::{InvokeCtx, Value};
    use atomr_agents_instruction::{ComposedInstructionStrategy, StaticBehaviorStrategy, StaticTaskStrategy};
    use atomr_agents_memory::{InMemoryStore, RecencyMemoryStrategy};
    use atomr_agents_persona::StaticPersonaStrategy;
    use atomr_agents_skill::StaticSkillStrategy;
    use atomr_agents_tool::{DynTool, Provider, StaticToolStrategy, Tool, ToolDescriptor, ToolSchema};
    use atomr_infer_testkit::{MockRunner, MockScript};

    use crate::inference::LocalRunnerClient;

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

    fn build_agent(
        runner: MockRunner,
    ) -> Agent<
        ComposedInstructionStrategy<StaticPersonaStrategy, StaticTaskStrategy, StaticBehaviorStrategy>,
        StaticToolStrategy,
        RecencyMemoryStrategy,
        StaticSkillStrategy,
    > {
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
        Agent {
            id: AgentId::from("a-1"),
            model: "mock".into(),
            instructions: instr,
            tools: tool_strat,
            memory: mem,
            skills: skill_strat,
            inference,
            bus: EventBus::new(),
            max_tool_iterations: 3,
        }
    }

    #[tokio::test]
    async fn agent_runs_simple_text_turn() {
        let runner = MockRunner::new(MockScript::from_text(["the answer is ", "42"]));
        let agent = build_agent(runner);
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

    // ----- Tool-call loop test --------------------------------------

    use std::sync::Mutex as StdMutex;

    use atomr_infer_core::batch::ExecuteBatch as IBatch;
    use atomr_infer_core::error::{InferenceError, InferenceResult};
    use atomr_infer_core::runner::{ModelRunner, RunHandle, SessionRebuildCause};
    use atomr_infer_core::runtime::{RuntimeKind, TransportKind};
    use atomr_infer_core::tokens::{FinishReason, TokenChunk, TokenUsage as IUsage};
    use futures::stream::{self, BoxStream, StreamExt};

    /// Two-step mock: first call returns a tool-call asking calculator(2,3);
    /// second call returns the final text "answer: 5".
    struct ToolLoopMock {
        step: StdMutex<u32>,
    }
    impl ToolLoopMock {
        fn new() -> Self {
            Self {
                step: StdMutex::new(0),
            }
        }
    }
    #[async_trait]
    impl ModelRunner for ToolLoopMock {
        async fn execute(&mut self, batch: IBatch) -> InferenceResult<RunHandle> {
            let mut s = self.step.lock().unwrap();
            *s += 1;
            let request_id = batch.request_id.clone();
            let chunks: Vec<TokenChunk> = if *s == 1 {
                vec![TokenChunk {
                    request_id: request_id.clone(),
                    text_delta: String::new(),
                    tool_call_delta: Some(serde_json::json!({
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "calculator", "arguments": "{\"a\":2,\"b\":3}"}
                        }]
                    })),
                    usage: Some(IUsage {
                        input_tokens: 5,
                        output_tokens: 0,
                        ..Default::default()
                    }),
                    finish_reason: Some(FinishReason::ToolCalls),
                }]
            } else {
                vec![TokenChunk {
                    request_id: request_id.clone(),
                    text_delta: "answer: 5".into(),
                    tool_call_delta: None,
                    usage: Some(IUsage {
                        input_tokens: 5,
                        output_tokens: 3,
                        ..Default::default()
                    }),
                    finish_reason: Some(FinishReason::Stop),
                }]
            };
            let stream: BoxStream<'static, InferenceResult<TokenChunk>> =
                stream::iter(chunks.into_iter().map(Ok::<_, InferenceError>)).boxed();
            Ok(RunHandle::streaming(stream))
        }
        async fn rebuild_session(&mut self, _: SessionRebuildCause) -> InferenceResult<()> {
            Ok(())
        }
        fn runtime_kind(&self) -> RuntimeKind {
            RuntimeKind::Custom("tool-loop-mock".into())
        }
        fn transport_kind(&self) -> TransportKind {
            TransportKind::LocalGpu
        }
    }

    #[tokio::test]
    async fn agent_drives_tool_call_loop() {
        let store = Arc::new(InMemoryStore::new());
        let mem = RecencyMemoryStrategy::new(store, 5, 30);
        let tools: Vec<DynTool> = vec![Arc::new(CalculatorTool::new())];
        let tool_strat = StaticToolStrategy::new(tools);
        let instr = ComposedInstructionStrategy::new(
            StaticPersonaStrategy::new("You are a calculator assistant."),
            StaticTaskStrategy("Use tools.".into()),
            StaticBehaviorStrategy("Reply tersely.".into()),
        );
        let skill_strat = StaticSkillStrategy::new(vec![]);
        let inference: Arc<dyn InferenceClient> =
            Arc::new(LocalRunnerClient::new(ToolLoopMock::new(), Provider::OpenAi));
        let agent: Agent<_, _, _, _> = Agent {
            id: AgentId::from("a-2"),
            model: "mock".into(),
            instructions: instr,
            tools: tool_strat,
            memory: mem,
            skills: skill_strat,
            inference,
            bus: EventBus::new(),
            max_tool_iterations: 3,
        };
        let r = agent
            .run_turn(
                "what is 2+3".into(),
                AgentBudgets {
                    tokens: TokenBudget::new(10_000),
                    time: TimeBudget::new(std::time::Duration::from_secs(30)),
                    money: MoneyBudget::from_usd(1.0),
                    iterations: IterationBudget::new(5),
                },
            )
            .await
            .unwrap();
        assert_eq!(r.text, "answer: 5");
        assert_eq!(r.finish_reason, Some(FinishReason::Stop));
    }
}
