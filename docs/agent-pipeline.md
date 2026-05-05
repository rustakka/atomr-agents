# Agent pipeline

The atomr-agents `Agent` runs an opinionated per-turn pipeline that
orchestrates strategy resolution, instruction rendering, context
assembly, inference, parallel tool dispatch, and memory storage —
with `AgentMiddleware` wrapping every layer.

## Per-turn pipeline

```
incoming message
  → MemoryStrategy::retrieve     ┐
  → SkillStrategy::applicable    ├─ tokio::join!, share TokenBudget
  → ToolStrategy::select         ┘
  → InstructionStrategy::render
  → ContextAssembler::assemble   (priority-merge under remaining budget)
  → InferenceClient::run         (wraps any atomr-infer ModelRunner)
  → tool-call loop:
      while finish_reason == ToolCalls and iters_left > 0:
          parse provider deltas  → Vec<ParsedToolCall>
          dispatch in parallel   (tokio::JoinSet, order-preserved)
          re-issue ExecuteBatch with extended history
  → MemoryStrategy::store
  → emit Event::AgentTurn (with run_id)
```

## Building an agent

`Agent<I, T, Ms, Sk>` is generic over the four hot-path strategies:

```rust
use std::sync::Arc;
use atomr_agents::prelude::*;
use atomr_agents::agent::{Agent, AgentBudgets, InferenceClient, LocalRunnerClient, Provider};
use atomr_agents::tool::StaticToolStrategy;
use atomr_agents::memory::{InMemoryStore, RecencyMemoryStrategy};
use atomr_agents::skill::StaticSkillStrategy;
use atomr_agents::persona::StaticPersonaStrategy;
use atomr_agents::instruction::{
    ComposedInstructionStrategy, StaticBehaviorStrategy, StaticTaskStrategy,
};
use atomr_agents::observability::EventBus;

let agent = Agent {
    id: AgentId::from("a-1"),
    model: "gpt-4o-mini".into(),
    instructions: ComposedInstructionStrategy::new(
        StaticPersonaStrategy::new("You are a precise assistant."),
        StaticTaskStrategy("Answer concisely.".into()),
        StaticBehaviorStrategy("Always cite sources.".into()),
    ),
    tools:   StaticToolStrategy::new(my_tools),
    memory:  RecencyMemoryStrategy::new(Arc::new(InMemoryStore::new()), 8, 40),
    skills:  StaticSkillStrategy::new(vec![]),
    inference,                  // any Arc<dyn InferenceClient>
    bus:     EventBus::new(),
    max_tool_iterations: 5,
};

let r = agent.run_turn("what's the capital of France?".into(), AgentBudgets::default()).await?;
println!("{}", r.text);
```

## Wiring an InferenceClient

`InferenceClient` wraps any atomr-infer `ModelRunner`. The trait
takes an `ExecuteBatch`, drives the streaming response, and returns
a `TurnResult` (text, usage, finish reason, parsed tool calls):

```rust
use atomr_infer_core::runner::ModelRunner;
use atomr_agents::agent::{InferenceClient, LocalRunnerClient, Provider};

// Any ModelRunner: MockRunner, OpenAiRunner, AnthropicRunner, vLLM, etc.
let inference: Arc<dyn InferenceClient> =
    Arc::new(LocalRunnerClient::new(my_runner, Provider::OpenAi));
```

`Provider::OpenAi` and `Provider::Anthropic` correspond to the two
streaming `tool_call_delta` formats; the parser handles both
natively. New providers add a `Provider` variant and a parser arm.

## Tool calls in parallel

When the model emits multiple tool calls in one assistant turn, the
agent fans them out via `tokio::JoinSet` with order-preserved
aggregation. A turn looks like:

```
Assistant: <text='', tool_calls=[calc(2,3), search("rust")]>
  ↓
JoinSet: {calc(2,3), search("rust")}     ← run concurrently
  ↓ collect, sort by index
[Tool: 5, Tool: <docs>]                   ← injected as Role::Tool messages
  ↓
re-issue ExecuteBatch with the appended assistant + tool messages
  ↓
Assistant: <text='answer: 5', finish=Stop>
```

`max_tool_iterations` caps the inner loop; `IterationBudget` on the
context provides a second guard.

## RichTool + ToolReturn

Plain tools return `Value` (serialized into the next `Role::Tool`
message). `RichTool` returns a richer `ToolReturn`:

```rust
pub enum ToolReturn {
    Content(Value),
    ContentAndArtifact { content: Value, artifact: Value },
    Command(ToolControl),  // Handoff / Done / Update
}
```

`Command(ToolControl::Handoff { target, payload })` is the substrate
for multi-agent handoff (see [Multi-agent
patterns](multi-agent-patterns.md)). `HandoffTool` ships in
`agents-tool` as the canonical helper.

## AgentMiddleware

Middleware wraps the agent loop with optional hooks:

```rust
#[async_trait]
pub trait AgentMiddleware: Send + Sync + 'static {
    async fn before_agent(&self, _agent_id: &AgentId, _user: &str) -> Result<()> { Ok(()) }
    async fn before_model_call(&self, _batch: &mut ExecuteBatch) -> Result<()> { Ok(()) }
    async fn after_model_call(&self, _result: &mut TurnResult) -> Result<()> { Ok(()) }
    async fn before_tool_call(&self, _name: &str, _args: &mut Value) -> Result<()> { Ok(()) }
    async fn after_tool_call(&self, _name: &str, _result: &mut Result<Value>) -> Result<()> { Ok(()) }
    async fn after_agent(&self, _result: &mut TurnResult) -> Result<()> { Ok(()) }
    async fn dynamic_prompt(&self, _agent_id: &AgentId, _user: &str) -> Result<Option<String>> { Ok(None) }
}
```

`MiddlewareStack` runs `before_*` hooks in registration order and
`after_*` hooks in reverse order — the standard Tower convention. A
`Some(_)` from `dynamic_prompt` overrides the rendered system
prompt for this turn (last middleware wins).

### Stock middlewares

| Middleware | Purpose |
|---|---|
| `LoggingMiddleware` | append a structured log line per phase |
| `RateLimitMiddleware { capacity, refill_per_sec }` | token-bucket gate on `before_model_call` |
| `RedactionMiddleware { patterns, replacement }` | strip PII patterns from outgoing user messages |
| `ToolErrorRecoveryMiddleware` | convert tool errors to `{tool_error: true, …}` payloads so the model can recover instead of bubbling up |

```rust
agent.middleware = MiddlewareStack::new()
    .push(Arc::new(LoggingMiddleware::new()))
    .push(Arc::new(RateLimitMiddleware::new(10, 5)))
    .push(Arc::new(RedactionMiddleware::new(
        vec!["secret".into(), "api_key=".into()],
        "[redacted]",
    )))
    .push(Arc::new(ToolErrorRecoveryMiddleware));
```

## Budgets

Every turn consumes from four budgets carried in `AgentBudgets`:

| Budget | Default | Notes |
|---|---|---|
| `TokenBudget` | 8192 | split among parallel resolutions; `ContextAssembler` evicts low-priority fragments under pressure |
| `TimeBudget` | 30s | not enforced by the agent itself; `WithTimeout` decorator does it |
| `MoneyBudget` | $1.00 | informational; tools / middlewares can consume it |
| `IterationBudget` | per-turn iteration cap | bounds the inner tool-call loop |

Strategies declare what they consume via `&mut TokenBudget`; the
`ContextAssembler` packs the rendered fragments into the remaining
budget by priority (system prompt = 9, memory = 5, history = 7,
tools = 6 by default).

## Where to go from here

- [State and checkpointing](state-and-checkpointing.md) — store
  cross-turn state via `LongStore` + `RecallMemoryTool`.
- [Retrieval and ingestion](retrieval-and-ingestion.md) — wire a
  retriever into a tool or directly into the `ToolStrategy`.
- [Workflows and HITL](workflows-and-hitl.md) — when a single agent
  isn't enough; hand off to a workflow with an interrupt.
- [Multi-agent patterns](multi-agent-patterns.md) — supervisor
  routes between specialist agents; `HandoffTool` as the seam.
