---
name: atomr-agents-quickstart
description: Use when standing up the first atomr-agents project, picking feature flags for the `atomr-agents` umbrella, building a first `Agent`, or running an end-to-end agent turn against `MockRunner`. Triggers on adding `atomr-agents = ...` to Cargo.toml, writing the first `Agent { ... }` literal, wiring an `InferenceClient`, or asking "how do I get atomr-agents running".
---

# atomr-agents quickstart

A composable agentic framework on the [atomr](https://github.com/rustakka/atomr)
actor runtime. Pluggable strategies, channelled state, durable
checkpointing, parallel tool dispatch, retriever zoo — the LangGraph
+ LangChain feature surface in atomr's actor idiom.

## The 30-second mental model

- **Composition is the unit of work.** Every executable unit
  (prompt, model, parser, tool, retriever, sub-agent, workflow,
  harness) implements `Callable`. `Pipeline` chains them. Decorators
  (`with_retry`, `with_fallbacks`, `with_config`, `with_timeout`)
  layer policies orthogonally.
- **State is channelled.** `StateSchema` declares one channel per
  state key. Each channel carries a reducer
  (`AppendMessages`, `MergeMap`, `LastWriteWins`, `AppendList`,
  `MaxByTimestamp`). `Checkpointer` persists per-super-step
  snapshots; `fork(checkpoint, edits)` creates a divergent run.
- **Strategies are the extension surface.** `InstructionStrategy`,
  `ToolStrategy`, `MemoryStrategy`, `SkillStrategy`, `LoopStrategy`,
  `TerminationStrategy` — generic so the agent's hot path
  monomorphizes; `Box<dyn>` opt-in for runtime config.
- **Tools are parallel and provider-agnostic.** Multiple tool calls
  in one assistant turn dispatch concurrently via `tokio::JoinSet`.
  Streaming `tool_call_delta` parses OpenAI and Anthropic deltas
  natively.

## The minimal consumer Cargo.toml

```toml
[dependencies]
# Defaults: agent + tool + skill + memory + persona + instruction
atomr-agents = "0.1"
atomr-infer  = { version = "0.4", features = ["openai"] }   # or any provider

# Add features as needed:
# atomr-agents = { version = "0.1", features = ["harness", "eval", "embed"] }

# RAG-flavored:
# atomr-agents-retriever = "0.1"
# atomr-agents-ingest    = "0.1"

# Test scaffolding:
# atomr-agents = { version = "0.1", features = ["testkit"] }
```

See [`docs/feature-matrix.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/feature-matrix.md)
for every feature flag and the canonical "shapes" (minimal agent /
RAG / production harness / multi-agent / kitchen-sink).

## Building an agent

```rust
use std::sync::Arc;
use atomr_agents::prelude::*;
use atomr_agents::agent::{Agent, AgentBudgets, InferenceClient, LocalRunnerClient, Provider};
use atomr_agents::tool::{StaticToolStrategy, DynTool};
use atomr_agents::memory::{InMemoryStore, RecencyMemoryStrategy};
use atomr_agents::skill::StaticSkillStrategy;
use atomr_agents::persona::StaticPersonaStrategy;
use atomr_agents::instruction::{
    ComposedInstructionStrategy, StaticBehaviorStrategy, StaticTaskStrategy,
};
use atomr_agents::observability::EventBus;
use atomr_infer_testkit::{MockRunner, MockScript};

let runner = MockRunner::new(MockScript::from_text(["the answer is ", "42"]));
let inference: Arc<dyn InferenceClient> =
    Arc::new(LocalRunnerClient::new(runner, Provider::OpenAi));

let agent = Agent {
    id: AgentId::from("a-1"),
    model: "gpt-4o-mini".into(),
    instructions: ComposedInstructionStrategy::new(
        StaticPersonaStrategy::new("You are a helpful assistant."),
        StaticTaskStrategy("Answer concisely.".into()),
        StaticBehaviorStrategy("Always cite sources.".into()),
    ),
    tools:   StaticToolStrategy::new(Vec::<DynTool>::new()),
    memory:  RecencyMemoryStrategy::new(Arc::new(InMemoryStore::new()), 8, 40),
    skills:  StaticSkillStrategy::new(vec![]),
    inference,
    bus:     EventBus::new(),
    max_tool_iterations: 5,
};

let r = agent.run_turn("hi".into(), AgentBudgets::default()).await?;
println!("{}", r.text);
```

Swap `MockRunner` for any atomr-infer `ModelRunner` (OpenAI,
Anthropic, vLLM, Candle, …) and the same code runs unchanged. The
`Provider` enum tells the streaming tool-call parser which delta
format to expect.

## Composing prompt → model → parser

```rust
use atomr_agents::callable::{Pipeline, with_retry, RetryPolicy};

let pipeline = Pipeline::from(prompt_handle)
    .then(model_handle)
    .then(parser_handle)
    .build();

// Add retries on the model stage:
let resilient_model = with_retry(model_handle, RetryPolicy::default());
let pipeline = Pipeline::from(prompt_handle)
    .then(resilient_model)
    .then(parser_handle)
    .build();

let output = pipeline.call(input, ctx).await?;
```

This is the LCEL `prompt | model | parser` shape.

## When to reach beyond the agent

| You need… | Reach for… |
|---|---|
| Channelled state + checkpointing | `atomr-agents-state` — see `atomr-agents-state` skill |
| Pause/resume / approval flows | `atomr-agents-workflow::Interruptible` — see `atomr-agents-hitl` skill |
| RAG | `atomr-agents-retriever` + `atomr-agents-ingest` — see `atomr-agents-rag` skill |
| Multi-agent topology | `atomr-agents-org` — see `atomr-agents-multi-agent` skill |
| Eval gates in CI | `atomr-agents-eval` + `atomr-agents-registry` — see `atomr-agents-eval` skill |
| Tracing into LangSmith | `atomr-agents-observability` — see `atomr-agents-observability` skill |

## Canonical references

- [`docs/index.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/index.md) — documentation hub
- [`docs/architecture.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/architecture.md) — runtime layout
- [`docs/agent-pipeline.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/agent-pipeline.md) — per-turn pipeline + tool-call loop
- [`docs/feature-matrix.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/feature-matrix.md) — every feature flag
- [`crates/agent/src/pipeline.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/agent/src/pipeline.rs) — agent implementation
- [`crates/harness/examples/research_harness.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/harness/examples/research_harness.rs) — sub-harness composition example

## Common mistakes

- **Forgetting to set `Provider`.** The wrong provider parses
  `tool_call_delta` for the wrong format and tools never fire.
- **Empty `tools` with `max_tool_iterations > 1`.** Wastes inference
  budget; if you have no tools, set `max_tool_iterations = 1`.
- **Using `MockRunner` in production.** It returns canned chunks; the
  agent will appear to work but never actually consult a model.
- **Holding the `Agent` across `.await` while sharing it.** `Agent`
  isn't `Sync` — wrap in `Arc<RwLock<>>` if multiple callers need it.
- **Pulling `cudarc` into a remote-only build.** Add atomr-infer's
  `remote-only` feature to keep GPU deps out of the dep graph.
