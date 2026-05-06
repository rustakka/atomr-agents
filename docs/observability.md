# Observability

How atomr-agents emits, aggregates, and exports the structured
events that power tracing, metrics, replay, and the Studio inspector.

## Event taxonomy

Every observable boundary in the framework emits a typed `Event`:

```rust
pub enum Event {
    StrategyResolved  { strategy, agent_id, elapsed_ms, tokens_used },
    ToolInvoked       { tool_id, args_hash, elapsed_ms, ok },
    /// Per detected tool call before dispatch — distinct from
    /// `ToolInvoked` (post-call). Lets tracers / UIs surface tool
    /// intent in real time.
    ToolCallStreamed  { agent_id, tool_name, arguments_hash, iteration },
    AgentTurn         {
        agent_id, input_tokens, output_tokens,
        // `reasoning_tokens` (o1-style) and `cached_tokens` (Anthropic
        // prompt-cache, OpenAI cached input) are populated when the
        // provider reports them; both default to 0 for runtimes that
        // don't surface them. Both are `#[serde(default)]`.
        reasoning_tokens, cached_tokens,
        finish_reason, elapsed_ms,
    },
    WorkflowStep      { workflow_id, step_id, step_kind, elapsed_ms, ok },
    HarnessIteration  { harness_id, iteration, outcome, budget_remaining_tokens },
    Backpressure      { actor_path, queued, dropped },
}
```

Each event is wrapped in an `EventEnvelope`:

```rust
pub struct EventEnvelope {
    pub timestamp_ms: i64,
    pub correlation_id: Option<String>,
    pub run_id: Option<RunId>,
    pub parent_run_id: Option<RunId>,
    pub tags: Vec<String>,
    pub event: Event,
}
```

The `run_id` / `parent_run_id` pair is the load-bearing field for
LangSmith-style run-tree assembly. `EventBus::emit_run(event,
run_id, parent)` is the canonical emission path inside the agent
pipeline; the bare `EventBus::emit(event)` is fine for ad-hoc
process-local diagnostics.

## EventBus

`EventBus` is a process-local broadcast. Subscribers receive every
emitted event:

```rust
use atomr_agents_observability::EventBus;
use atomr_agents_core::{Event, AgentId};

let bus = EventBus::new();
bus.subscribe(|env| {
    eprintln!("{:?} {:?}", env.run_id, env.event);
});

bus.emit(Event::AgentTurn {
    agent_id: AgentId::from("a-1"),
    input_tokens: 50,
    output_tokens: 12,
    reasoning_tokens: 0,        // o1-style; 0 when the provider doesn't report.
    cached_tokens: 8,           // Anthropic prompt-cache / OpenAI cached input.
    finish_reason: None,
    elapsed_ms: 230,
});
```

## Cost reporting

`reasoning_tokens` and `cached_tokens` flow through from
`atomr_infer_core::tokens::TokenUsage` per-chunk and aggregate into
the per-turn `AgentTurn`. Use them when computing spend:

- **Anthropic prompt-cache** and **OpenAI cached input** charge a
  fraction of the normal input rate; `cached_tokens` lets you bill
  them at the discounted rate instead of double-counting under
  `input_tokens`.
- **o1-style reasoning tokens** are billed at the output rate but
  aren't surfaced in the assistant text — `reasoning_tokens` keeps
  the accounting honest.

Both fields are `#[serde(default)]`, so older event JSON
deserialises unchanged.

## ToolCallStreamed vs ToolInvoked

`Event::ToolCallStreamed` fires when the inference layer parses a
tool call out of the streaming `tool_call_delta` *before* the agent
dispatches it. `Event::ToolInvoked` fires *after* the tool returns.
Subscribe to `ToolCallStreamed` for live "the agent is about to call
X" indicators (Studio-style UIs); subscribe to `ToolInvoked` for
post-mortem latency / success metrics.

```rust
bus.subscribe(|env| match &env.event {
    Event::ToolCallStreamed { tool_name, iteration, .. } =>
        eprintln!("[stream] iter={iteration} tool={tool_name}"),
    Event::ToolInvoked { tool_id, elapsed_ms, ok, .. } =>
        eprintln!("[done] tool={} elapsed={elapsed_ms}ms ok={ok}", tool_id.as_str()),
    _ => {}
});
```

Subscribers are `Fn(&EventEnvelope) + Send + Sync + 'static`; each
event is dispatched synchronously to every sink. For high-throughput
production, plumb the bus into atomr's telemetry exporter (see
[atomr's observability docs](https://github.com/rustakka/atomr/blob/main/docs/observability.md)).

## RunTreeBuilder

`RunTreeBuilder` is an `EventBus` subscriber that aggregates events
into a parent-child run tree:

```rust
use std::sync::Arc;
use atomr_agents_observability::{EventBus, RunTreeBuilder};

let bus = EventBus::new();
let builder = Arc::new(RunTreeBuilder::new());
builder.clone().attach(&bus);

// … run agents / workflows / harnesses against this bus …

let roots = builder.roots();          // top-level runs (no parent)
let snapshot = builder.snapshot();    // every node by RunId
let one = builder.get(&run_id);       // a specific node
```

Each `RunNode` carries:

```rust
pub struct RunNode {
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
    pub kind: RunKind,            // Chain / Llm / Tool / Retriever / Parser / Agent / Workflow / Harness / Other
    pub name: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub tags: Vec<String>,
    pub events: Vec<EventEnvelope>,
    pub children: Vec<RunId>,
    pub error: Option<String>,
}
```

## Tracers

A `Tracer` consumes the run tree and exports it. Every shipped
tracer takes the `Arc<RunTreeBuilder>` plus a `TracerSink`:

```rust
#[async_trait]
pub trait Tracer: Send + Sync + 'static {
    async fn on_event(&self, _env: &EventEnvelope) -> Result<()> { Ok(()) }
    async fn flush(&self) -> Result<()>;
}
```

### StdoutTracer

Pretty-prints the run tree. Good for local development:

```rust
use atomr_agents_observability::StdoutTracer;

let tracer = StdoutTracer::new(builder.clone());
tracer.flush().await?;
// - agent:a-1 [Agent] 230 ms
//   - tool:calc [Tool] 5 ms
//   - tool:search [Tool] 22 ms
```

### JsonlTracer

One JSON-line per node. Useful for offline analysis or for piping
into a log aggregator:

```rust
use atomr_agents_observability::{FileSink, JsonlTracer, MemorySink};

// In-memory (tests):
let (tracer, sink) = JsonlTracer::in_memory(builder.clone());
tracer.flush().await?;
let lines: Vec<String> = sink.lines.lock().clone();

// File:
let sink = Arc::new(FileSink::new("./traces.jsonl"));
let tracer = JsonlTracer::new(builder.clone(), sink);
tracer.flush().await?;
```

### LangSmithTracer

LangSmith-shaped run records. The shipped exporter writes a JSON
line per run with `id` / `name` / `run_type` /
`start_time_ms` / `end_time_ms` / `parent_run_id` / `tags` / `error`
/ `project`. Wire your own HTTP sink for real LangSmith ingestion;
the `MemorySink` variant is for unit tests:

```rust
use atomr_agents_observability::LangSmithTracer;

let (tracer, sink) = LangSmithTracer::in_memory(builder.clone(), "my-project");
tracer.flush().await?;
// sink.lines now has one JSON record per node, ready to POST.
```

## Wiring the agent

The agent's `EventBus` is a constructor field. Once attached, every
`run_turn` emits `Event::AgentTurn`, and every tool dispatch emits
`Event::ToolInvoked`. To get a run tree:

```rust
let bus = EventBus::new();
let builder = Arc::new(RunTreeBuilder::new());
builder.clone().attach(&bus);

let agent = Agent { /* … */, bus: bus.clone(), /* … */ };
let _ = agent.run_turn(/* … */).await?;

let tracer = StdoutTracer::new(builder.clone());
tracer.flush().await?;
```

Workflows and harnesses follow the same shape — pass the same
`EventBus` instance to all of them and the run-tree builder fuses
the events automatically by `run_id`.

## RunId discipline

`RunId` is a newtype wrapping a string. Generate per-call with
`RunId::new()` (UUID-based) or pass a stable id (`RunId::from("my-run-1")`)
when you want callers to correlate against an external trace id.

The agent pipeline currently emits events without `run_id` by
default — wire `EventBus::emit_run(event, run_id, parent)` from your
middleware (`AgentMiddleware::before_agent` is a good place to start
a run, `after_agent` to close it).

## Where to go from here

- [Architecture](architecture.md) — where the observability layer
  sits in the crate stack.
- [Eval](eval.md) — eval suites that consume the same event stream
  for replay-based regression testing.
- [Workflows and HITL](workflows-and-hitl.md) — the Studio inspector
  uses run trees + checkpoints to render the read+resume UI.
