---
name: atomr-agents-tools
description: Use when authoring or modifying a `Tool` / `RichTool`, designing a tool's `ToolDescriptor` schema, returning a `ToolReturn::Command`, wiring tools into an `Agent`'s `ToolStrategy`, or using built-in tools (`HandoffTool`, `WriteMemoryTool`, `RecallMemoryTool`). Triggers on `impl Tool for`, `impl RichTool for`, `ToolReturn::Command(...)`, `StaticToolStrategy::new(...)`, or `ToolCallParser::feed(...)`.
---

# Authoring tools in atomr-agents

Tools are the agent's hands. atomr-agents has two trait variants —
plain `Tool` (returns `Value`) and `RichTool` (returns `ToolReturn`,
including graph control flow) — plus a streaming `ToolCallParser`
that handles OpenAI and Anthropic deltas natively.

## Mental model

- **`Tool`** is the minimum surface: `descriptor()` advertises what
  the model sees; `invoke(args, ctx)` runs the work.
- **`RichTool`** lets a tool return `ToolReturn::Command(...)` to
  drive graph control flow (handoff to another agent, terminate the
  turn, update workflow state).
- **`ToolStrategy`** is what the agent consults to pick which tools
  to expose this turn. `Static` always exposes the same list;
  `Keyword` substring-matches the user message; `Embedding`
  cosine-ranks across thousands of descriptors.
- **Parallel dispatch** is automatic. When a model emits N tool
  calls in one turn, the agent fans them into `tokio::JoinSet` and
  aggregates by original index.

## Authoring a plain Tool

```rust
use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result, ToolId, Value};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};

struct Calculator {
    descriptor: ToolDescriptor,
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                id: ToolId::from("calculator"),
                name: "calculator".into(),
                description: "Add two integers.".into(),
                schema: ToolSchema(serde_json::json!({
                    "type": "object",
                    "required": ["a", "b"],
                    "properties": {
                        "a": {"type": "integer"},
                        "b": {"type": "integer"},
                    }
                })),
            },
        }
    }
}

#[async_trait]
impl Tool for Calculator {
    fn descriptor(&self) -> &ToolDescriptor { &self.descriptor }
    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let a = args.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
        let b = args.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
        Ok(serde_json::json!({"sum": a + b}))
    }
}
```

The descriptor `schema` is a JSON-Schema fragment. Keep it tight:
the model sees this verbatim and uses it to construct the call.

## RichTool — driving graph control flow

```rust
use atomr_agents_tool::{RichTool, ToolControl, ToolReturn};

struct Handoff { /* ... */ }

#[async_trait]
impl RichTool for Handoff {
    fn descriptor(&self) -> &ToolDescriptor { /* ... */ }
    async fn invoke_rich(&self, args: Value, _ctx: &InvokeCtx) -> Result<ToolReturn> {
        let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("specialist");
        Ok(ToolReturn::Command(ToolControl::Handoff {
            target: target.into(),
            payload: args.get("payload").cloned().unwrap_or(Value::Null),
        }))
    }
}
```

`RichTool` blanket-`impl`s `Tool`, so any `RichTool` works wherever a
`Tool` is required. The agent / workflow layer interprets
`ToolReturn::Command` to:

- **`ToolControl::Handoff { target, payload }`** — set the
  `ActiveAgent` slot in a swarm, or invoke the named team child in a
  supervisor topology.
- **`ToolControl::Done(value)`** — terminate the current turn early
  with `value` as the final result.
- **`ToolControl::Update(vec![(key, value), …])`** — write workflow
  channels via their reducers.

## Built-in tools

`atomr-agents-tool` ships `HandoffTool`. `atomr-agents-memory`
ships `WriteMemoryTool` / `UpdateMemoryTool` / `RecallMemoryTool`:

```rust
use atomr_agents_memory::{RecallMemoryTool, UpdateMemoryTool, WriteMemoryTool};
use atomr_agents_tool::HandoffTool;

let tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(WriteMemoryTool::new(store.clone())),
    Arc::new(UpdateMemoryTool::new(store.clone())),
    Arc::new(RecallMemoryTool::new(store.clone())),
    Arc::new(HandoffTool::new("specialist")),
    Arc::new(Calculator::new()),
];
```

## ToolStrategy — picking which tools matter

```rust
use atomr_agents_tool::{KeywordToolStrategy, StaticToolStrategy};

// Always expose every tool:
let strat = StaticToolStrategy::new(tools.clone());

// Substring-match against the user message; cap at 5:
let strat = KeywordToolStrategy::new(tools.clone(), 5);

// Embedding-based (top-k cosine over descriptors); needs an Embedder + AnnIndex:
use atomr_agents_embed::{EmbeddingToolStrategy, InMemoryAnnIndex, MockEmbedder};
let strat = EmbeddingToolStrategy::build(
    Arc::new(MockEmbedder::new(16)),
    Arc::new(InMemoryAnnIndex::new(16)),
    tools,
    /* top_k */ 5,
).await?;
```

Embedding-based selection is what scales when you have thousands of
tool descriptors but only the relevant top-k can fit the prompt.

## Tool-call parsing

The streaming `ToolCallParser` accumulates provider-specific
`tool_call_delta` JSON across chunks:

```rust
use atomr_agents_tool::{ParsedToolCall, Provider, ToolCallParser};

let mut parser = ToolCallParser::new(Provider::OpenAi);  // or Provider::Anthropic

// Feed each TokenChunk.tool_call_delta as it arrives:
parser.feed(&chunk.tool_call_delta.unwrap())?;

// On stream end:
let calls: Vec<ParsedToolCall> = parser.finish();
for call in &calls {
    let args = call.arguments()?;  // parse args_raw → Value
    println!("{} ({}) args={:?}", call.name, call.id, args);
}
```

The agent's per-turn pipeline already runs this internally — you
only need it directly when building a custom inference client.

## Parallel dispatch

When a model emits multiple tool calls in one assistant turn, the
agent fans them into `tokio::JoinSet`. Order is preserved on
aggregation, so `Role::Tool` messages are appended in the same
order the model emitted them — even if `search` finished before
`calc`.

This means tools should be:

1. **Independent** — they all see the same input snapshot; mutations
   that depend on each other won't cascade within a single turn.
2. **Idempotent under same args** — retries on transient errors are
   safe.
3. **Bounded** — a slow tool blocks the join, not just itself.

For tools with side effects (e.g. card charges), wrap with
idempotency keys inside the tool, not outside.

## Exposing a `RichTool` to the agent

`StaticToolStrategy` accepts `DynTool = Arc<dyn Tool>`. Since
`RichTool` blanket-impls `Tool`, the same wiring works:

```rust
use atomr_agents_tool::DynTool;

let mixed: Vec<DynTool> = vec![
    Arc::new(Calculator::new()),       // plain Tool
    Arc::new(Handoff::new("L2")),       // RichTool
];
let strat = StaticToolStrategy::new(mixed);
```

The agent loop sees `Tool::invoke` (which projects `RichTool`'s
`ToolReturn::Content`); for richer behavior wire a custom strategy
that downcasts to `RichTool` and dispatches based on `ToolReturn`.

## Canonical references

- [`docs/agent-pipeline.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/agent-pipeline.md) — tool-call loop
- [`docs/multi-agent-patterns.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/multi-agent-patterns.md) — `HandoffTool` flows
- [`crates/tool/src/tool_return.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/tool/src/tool_return.rs)
- [`crates/tool/src/parser.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/tool/src/parser.rs) — provider parsers

## Common mistakes

- **Verbose descriptors.** The model pays per token reading them.
  Trim descriptions to what disambiguates this tool from siblings.
- **Mutable state inside `Tool`.** Tools are `Sync`; use `Arc<RwLock>`
  or `Mutex`, not raw `&mut`.
- **`ToolReturn::Command(Done(...))` to escape from inside a parallel
  fan-out.** Other concurrent tool calls still finish; their results
  are still appended.
- **Wrong `Provider`.** OpenAI's `tool_calls[].function.arguments`
  vs. Anthropic's `content_block_delta.partial_json` — wrong arm
  silently produces empty calls.
- **Plain `serde_json::Value` arg parsing in `Tool::invoke`.**
  Validate types defensively; the model will emit garbage args
  occasionally.
