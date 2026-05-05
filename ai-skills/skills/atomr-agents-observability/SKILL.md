---
name: atomr-agents-observability
description: Use when wiring tracing into an agent / workflow / harness — `EventBus` subscribers, `RunTreeBuilder` for parent-child run trees, `StdoutTracer` / `JsonlTracer` / `LangSmithTracer` exporters, or propagating `RunId` / `parent_run_id` through events. Triggers on `EventBus::new`, `RunTreeBuilder::new`, `LangSmithTracer::new`, `bus.subscribe(...)`, or `bus.emit_run(...)`.
---

# Observability in atomr-agents

Every observable boundary (strategy resolution, tool invocation,
agent turn, workflow step, harness iteration, backpressure) emits a
typed `Event`. The `EventBus` broadcasts to subscribers; a
`RunTreeBuilder` aggregates events into a parent-child run tree;
`Tracer` exporters consume the tree for downstream sinks
(LangSmith, file, stdout, OpenTelemetry).

## Mental model

- **`Event`** is the typed taxonomy (`StrategyResolved`,
  `ToolInvoked`, `AgentTurn`, `WorkflowStep`, `HarnessIteration`,
  `Backpressure`).
- **`EventEnvelope`** wraps an event with `timestamp_ms`,
  `correlation_id`, `run_id`, `parent_run_id`, and `tags`. The
  `run_id` / `parent_run_id` pair lets a tree builder assemble the
  parent-child structure.
- **`EventBus`** is process-local broadcast. Subscribers are
  `Fn(&EventEnvelope) + Send + Sync + 'static` closures.
- **`RunTreeBuilder`** is itself an `EventBus` subscriber. It
  accumulates events into `RunNode`s indexed by `RunId`.
- **`Tracer`** + **`TracerSink`** is the export plane. Stock sinks:
  `MemorySink` (tests), `FileSink` (one-line JSONL on disk).

## Subscribing to events

```rust
use atomr_agents_observability::EventBus;
use atomr_agents_core::{AgentId, Event};

let bus = EventBus::new();
bus.subscribe(|env| {
    eprintln!("[{}] {:?}", env.timestamp_ms, env.event);
});

bus.emit(Event::AgentTurn {
    agent_id: AgentId::from("a-1"),
    input_tokens:  50,
    output_tokens: 12,
    finish_reason: None,
    elapsed_ms: 230,
});
```

## Emitting with run-id context

```rust
use atomr_agents_core::RunId;

let parent = RunId::new();
let child = RunId::new();

bus.emit_run(
    Event::AgentTurn { /* ... */ },
    parent.clone(),
    None,                       // top-level, no parent_run_id
);
bus.emit_run(
    Event::ToolInvoked { /* ... */ },
    child,
    Some(parent),               // child of the agent turn
);
```

## Building a run tree

```rust
use std::sync::Arc;
use atomr_agents_observability::{EventBus, RunTreeBuilder};

let bus = EventBus::new();
let builder = Arc::new(RunTreeBuilder::new());
builder.clone().attach(&bus);

// … agent / workflow / harness emits events …

let roots = builder.roots();          // top-level RunNodes
let snapshot = builder.snapshot();    // every node by RunId
let one = builder.get(&run_id);       // a specific node
```

Each `RunNode` has:

```rust
pub struct RunNode {
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
    pub kind: RunKind,
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

### StdoutTracer (local dev)

```rust
use atomr_agents_observability::StdoutTracer;

let tracer = StdoutTracer::new(builder.clone());
tracer.flush().await?;
// - agent:a-1 [Agent] 230 ms
//   - tool:calc [Tool] 5 ms
//   - tool:search [Tool] 22 ms
```

### JsonlTracer

One JSON-line per node — pipe into `jq`, log aggregation, or
storage:

```rust
use std::sync::Arc;
use atomr_agents_observability::{FileSink, JsonlTracer};

let sink: Arc<dyn atomr_agents_observability::TracerSink> =
    Arc::new(FileSink::new("./traces.jsonl"));
let tracer = JsonlTracer::new(builder.clone(), sink);
tracer.flush().await?;
```

For tests, use the in-memory variant:

```rust
let (tracer, mem_sink) = JsonlTracer::in_memory(builder.clone());
tracer.flush().await?;
let lines: Vec<String> = mem_sink.lines.lock().clone();
```

### LangSmithTracer

LangSmith-shaped JSON records (`id` / `name` / `run_type` /
`start_time_ms` / `end_time_ms` / `parent_run_id` / `tags` /
`error` / `project`):

```rust
use atomr_agents_observability::LangSmithTracer;

// In tests / offline:
let (tracer, sink) = LangSmithTracer::in_memory(builder.clone(), "my-project");
tracer.flush().await?;

// To ingest into LangSmith proper, write a `TracerSink` impl that
// POSTs each line to the LangSmith API using your `LANGCHAIN_API_KEY`.
```

The shipped exporter writes one record per `RunNode`. The
`run_type` field maps `RunKind::{Agent, Workflow, Harness, Chain,
Other}` → `"chain"`; `Tool` / `Llm` / `Retriever` / `Parser` map
1:1.

## RunId discipline

- Generate with `RunId::new()` (UUID-based) for fresh runs.
- Use `RunId::from("stable-id")` when correlating against external
  trace ids.
- The same `(WorkflowId, RunId)` keys all checkpoints — see the
  `atomr-agents-state` and `atomr-agents-hitl` skills for how
  state-layer and event-layer correlate.

## Wiring an agent

```rust
let bus = EventBus::new();
let builder = Arc::new(RunTreeBuilder::new());
builder.clone().attach(&bus);

let agent = Agent { /* ... */, bus: bus.clone(), /* ... */ };
let _ = agent.run_turn(/* ... */).await?;

let tracer = StdoutTracer::new(builder.clone());
tracer.flush().await?;
```

For multi-agent or workflow scenarios, share the same `EventBus`
across all `Agent` / `Harness` / `WorkflowRunner` instances — the
run tree fuses by `run_id`.

## Custom event subscribers

```rust
use std::sync::Arc;
use parking_lot::Mutex;

let counter = Arc::new(Mutex::new(0u32));
{
    let counter = counter.clone();
    bus.subscribe(move |env| {
        if matches!(env.event, atomr_agents_core::Event::ToolInvoked { .. }) {
            *counter.lock() += 1;
        }
    });
}
```

## Canonical references

- [`docs/observability.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/observability.md)
- [`crates/observability/src/run_tree.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/observability/src/run_tree.rs)
- [`crates/observability/src/tracer.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/observability/src/tracer.rs)

## Common mistakes

- **Forgetting to call `flush()`.** Tracers are lazy — they buffer
  in the `RunTreeBuilder` and emit on `flush`.
- **Mixing `bus.emit(...)` and `bus.emit_run(...)` carelessly.**
  Bare `emit` produces events with `run_id = None`; the run tree
  builder ignores them.
- **Cloning `EventBus` and expecting separate states.** It's an
  `Arc` internally — clones share subscribers.
- **Heavy work inside a subscriber.** Subscribers run synchronously
  on the emit thread; long-running work blocks the agent loop. Push
  to a channel and run async on the receiver.
- **Subscribing late.** Late subscribers see only events emitted
  after they joined; pre-historic events are not replayed.
