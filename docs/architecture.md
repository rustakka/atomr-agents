# Architecture

How atomr-agents is laid out, what each crate does, where the
composition and persistence boundaries fall, and where the
heterogeneous components (retriever, parser, cache, multi-agent
topology) slot in. This is the map for somebody who wants to
understand or extend the framework.

## Crate stack (bottom → top)

```
                              atomr-agents (umbrella)
                                       ▲
                                       │
   ┌──────────┬──────────┬──────────┬──────┴───────┬──────────┬──────────┐
   │          │          │          │              │          │          │
   ▼          ▼          ▼          ▼              ▼          ▼          ▼
testkit   cli        py-bindings  registry      eval        cache     ingest
   │          │          │          │              │          │          │
   │          │          │          ▼              ▼          │          │
   │          │          │     harness ◄──────────┘           │          │
   │          │          │          ▲                         │          │
   │          │          │          │                         │          │
   │          │          │     workflow                       │          │
   │          │          │          ▲                         │          │
   │          │          │          │                         │          │
   │          │          │      agent ◄─── middleware         │          │
   │          │          │          ▲                         │          │
   │          │          │     ┌────┼────┬──────────┐         │          │
   │          │          │     │    │    │          │         │          │
   │          │          │     ▼    ▼    ▼          ▼         │          │
   │          │          │  instr persona memory  retriever ◄─┘          │
   │          │          │     │    │    │          │                    │
   │          │          │     │    │    │          │     embed ─────────┘
   │          │          │     │    │    │          │       ▲
   │          │          │     │    │    │          │       │
   │          │          │     ▼    ▼    ▼          ▼       │
   │          │          │       skill, tool, context, state, observability
   │          │          │                          │
   │          │          │                          ▼
   │          │          │                     strategy, callable, core
   ▼          ▼          ▼                          ▲
                                                     │
                                              atomr / atomr-infer
```

Each crate owns one concern — picking it up in isolation gives you the
contract; pulling more in adds capability without changing the
contract.

## Concept by concept

### Callable + Pipeline

`Callable` is the universal composition trait:

```rust
#[async_trait]
pub trait Callable: Send + Sync + 'static {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value>;
    fn label(&self) -> &str { /* default */ }
}
pub type CallableHandle = Arc<dyn Callable>;
```

Every executable unit — prompt template, model client, parser,
retriever, tool, sub-agent, workflow, harness — implements
`Callable`. `Pipeline` chains callables sequentially (`.then`),
fans out into a JSON object (`.fan_out`), or augments with a derived
key (`.assign`). Decorators (`with_retry`, `with_fallbacks`,
`with_config`, `with_timeout`, `Branch`) wrap any `CallableHandle`
into a new `CallableHandle`, so policies layer freely.

This is atomr-agents' answer to LangChain's LCEL: same composition
shape (`prompt | model | parser`), expressed as an additive Rust
builder.

### Strategy traits

The strategy trait family is the framework's extension point. A
strategy answers a single question — *given the current context and
budget, what do you contribute?* — and composes orthogonally:

| Trait | Question | Implementations |
|---|---|---|
| `InstructionStrategy::render` | what's the system prompt? | `ComposedInstructionStrategy<P, T, B>` (Persona × Task × Behavior) with cooperative budget split |
| `PersonaStrategy::resolve` | who is the agent? | `Static`, `BigFive`, `Mbti`, `Jungian`, `Composite` (weighted) + emphasis strategies |
| `ToolStrategy::select` | which tools matter this turn? | `Static`, `Keyword`, `Embedding` (cosine top-k over descriptors) |
| `MemoryStrategy::retrieve` / `store` | what do I remember / commit? | `Recency`, `Summarizing`, `Chained` |
| `SkillStrategy::applicable` | which skill bundles to inject? | `Static`, `Keyword` |
| `RoutingStrategy::route` | (org-level) where does this go? | `RoundRobin`, `LoadAware`, `CapabilityMatch` |
| `PolicyStrategy::evaluate` | is this allowed? | `Policy::narrow` for inheritance |
| `LoopStrategy::step` / `TerminationStrategy::should_terminate` | (harness) one iteration / when stop? | user-supplied |

Strategies are generic so the per-turn hot path monomorphizes; each
trait also has a `Box<dyn>` form for runtime config-driven loading.

### State, channels, reducers, checkpointing

`StateSchema` declares one channel per state key. Each channel
carries a typed reducer that merges existing values with incoming
writes. Five reducers ship:

| Reducer | Behavior |
|---|---|
| `LastWriteWins` | replace |
| `AppendList` | concatenate |
| `AppendMessages` | append-with-id-dedup (LangGraph's `add_messages`) |
| `MergeMap` | shallow object merge |
| `MaxByTimestamp` | keep value with the higher `ts_ms` field |

`RunState` is the runtime container: `read(key)` / `write(key, v)` /
`merge_writes(vec)`. After every super-step the `Checkpointer`
persists the snapshot keyed by `(workflow_id, run_id, super_step)`.
On resume, the runner hydrates from the latest snapshot and skips
already-completed super-steps. `Checkpointer::fork(from, edits)`
creates a divergent run from any prior checkpoint with optional state
edits — the substrate for time-travel debugging and HITL approval
flows.

### Workflows: DAG, Send-API, interrupts, subgraphs

`WorkflowRunner` (legacy) and `StatefulRunner` (channelled) execute a
typed `Dag<Step>`. `Step` covers `Invoke` / `Branch` / `Parallel` /
`Loop` / `Map` / `Human`. `dispatch_fan_out(producer, target, n)` is
the Send-API analogue: a producer returns a list at runtime; targets
run with bounded concurrency, order-preserved.

`Interruptible` adds dynamic `interrupt()` from inside a step plus
static `interrupt_before` / `interrupt_after`. On pause, the runner
persists a special checkpoint and returns `RunOutcome::Paused`. The
caller drives `Command::{Continue, Resume(value), Update(edits),
Goto(step)}` to resume.

`Subgraph` packages a `StatefulRunner`-style execution as a
`Callable`. Parents declare `input_channels` (read from parent state,
projected into the child) and `output_channels` (read from the
child's final state, merged back through the parent's reducers).

### Agent pipeline

`Agent<I, T, Ms, Sk>` runs the per-turn pipeline:

```
incoming message
  → MemoryStrategy::retrieve     ┐
  → SkillStrategy::applicable    ├─ tokio::join!, share TokenBudget
  → ToolStrategy::select         ┘
  → InstructionStrategy::render
  → ContextAssembler::assemble   (priority merge under budget)
  → InferenceClient::run         (atomr-infer ModelRunner)
  → tool-call loop               (parallel via JoinSet, order-preserved)
  → MemoryStrategy::store
  → emit Event::AgentTurn
```

`AgentMiddleware` wraps every layer with `before_agent` /
`before_model_call` / `after_model_call` / `before_tool_call` /
`after_tool_call` / `after_agent` / `dynamic_prompt` hooks. Stack
ordering is registration order for `before_*` and reverse for
`after_*` (Tower convention). Stock middlewares: `Logging`,
`RateLimit` (token-bucket), `Redaction`, `ToolErrorRecovery`.

### Tool-call layer

`Tool` is the trait every tool implements. `RichTool` is the richer
variant returning `ToolReturn::{Content, ContentAndArtifact,
Command(ToolControl::{Handoff, Done, Update})}`. Provider deltas
flow through the streaming `ToolCallParser` (OpenAI / Anthropic);
`ParsedToolCall::arguments()` produces the parsed JSON Value once
the args string is complete.

When a model emits multiple tool calls in one turn, the agent fans
them into `JoinSet` and aggregates by original index. `HandoffTool`
ships in `agents-tool` for multi-agent handoff flows.

### Long-term memory + retriever zoo + ingestion

`LongStore` is namespace-tupled (`("user", "alice", "facts")`),
embedding-indexed semantic search, and cross-thread (not scoped to
a single `RunId`). `WriteMemoryTool` / `UpdateMemoryTool` /
`RecallMemoryTool` ship as built-in tools.

`Retriever` is the unifying retrieval trait. Stock impls cover
sparse (`Bm25Retriever`), dense (`VectorRetriever` over `LongStore`),
LLM expansion (`MultiQueryRetriever`), extractive compression
(`ContextualCompressionRetriever`), parent-doc lookup
(`ParentDocumentRetriever`), Reciprocal Rank Fusion
(`EnsembleRetriever`), NL → filter (`SelfQueryRetriever`),
embedding cutoff (`EmbeddingsFilter`), and recency decay
(`TimeWeightedRetriever`).

`agents-ingest` rounds out RAG: text / markdown / json / csv loaders;
`Recursive` / `MarkdownHeader` / `Code` / `Token` / `Semantic`
splitters; `CachedEmbedder` (content-hash → vector); `IngestPipeline`
chains splitters; `ingest(store, namespace, embedder, chunks)` writes
embedded chunks in one call.

### Observability

Every observable boundary emits a typed `Event` (`StrategyResolved`,
`ToolInvoked`, `AgentTurn`, `WorkflowStep`, `HarnessIteration`,
`Backpressure`). `EventEnvelope` carries `run_id` / `parent_run_id`
/ `tags`, so a `RunTreeBuilder` subscriber can flatten the stream
into a parent-child run tree. `Tracer` exporters (`StdoutTracer`,
`JsonlTracer`, `LangSmithTracer`) consume the run tree.

### Multi-agent topology

`Org` → `Department` → `Team` → unit hierarchy. Each level is a
`Callable` holding children, an `OrgRoutingStrategy`, a `Policy`, and
granted toolsets. `Policy::narrow(parent, child)` intersects allowed
toolsets/models and takes the min of numeric caps — so policy
inherits and narrows downward, never expands.

`NamespacedMemory` reads cascade outward (`agent → team → org`),
writes are gated (agent owns its namespace; team writes require
`allow_team_write`; org-level writes are always denied for agents).

`ActiveAgent` / `swarm_loop` ship as the canonical helpers for
swarm / network patterns; `HandoffTool` integrates with `RichTool`'s
`Command(ToolControl::Handoff { target, payload })`.

### Eval

`EvalSuite` runs a `Vec<EvalCase>` against any `Callable`,
applying a `Scorer`. Stock scorers: `ContainsScorer`,
`LlmJudgeScorer` (single criterion: pass/fail), `RubricScorer`
(weighted multi-criterion), `PairwiseScorer` (A/B preference).
`RegressionGate` blocks publish when pass-rate drops more than
`tolerance` below baseline. `AnnotationQueue` (in-memory) captures
items for human review.

### Registry + harness

`Registry` is keyed `(kind, id, version)` over seven artifact kinds
(ToolSet / Skill / Persona / Agent / Workflow / Harness /
HarnessSet). `publish_gated(record, baseline?, current, tolerance)`
errors with `PolicyDenied` if the eval pass-rate regressed.

`Harness<L, T>` is itself a `Callable`. The loop runs `LoopStrategy::
step` until `TerminationStrategy::should_terminate` reports
`Termination::Done`. Every iteration emits
`Event::HarnessIteration`; `HarnessState` accumulates a
`StepEvent` history.

### Python

`atomr_agents._native` (PyO3) exposes the full framework surface
through 28 hierarchical submodules. The universal-currency type is
`Callable` (a PyO3 wrapper around `Arc<dyn Callable>`): agents,
workflows, harnesses, retrievers, ingest pipelines, and tools all
project as a `Callable` so generic Rust types never leak their type
parameters across the FFI boundary. The agent / harness runtimes
plug type-erased into the framework via `BoxedAgent` and `Box<dyn
LoopStrategy/TerminationStrategy>`; the necessary blanket
`impl Trait for Box<dyn Trait>` impls live alongside their trait
definitions (in `atomr-agents-instruction`, `atomr-agents-strategy`,
`atomr-agents-harness`).

Guest-mode decorators (`@tool`, `@strategy`, `@retriever`,
`@embedder`, `@callable_`, `@inference_client`, `@loader`,
`@splitter`, `@tracer`, `@conversation_agent`, `@diarizer`, `@vad`,
`@phonemizer`, `@journal`, `@repair_model`, `@persona_reconciler`,
`@ann_index`, plus `@persona`, `@skill`, `@parser`, `@scorer`,
`@memory_store`, `@long_store`, `@kv_cache`) register Python
factories with a process-wide DashMap; the matching Rust adapter
(`PyToolAdapter`, `PyRetrieverAdapter`, …) wraps the registered
`PyObject` and dispatches the trait methods via GIL acquisition +
`pyo3-async-runtimes::tokio::into_future` for coroutine returns.
The current dispatcher is in-process; an
`atomr-pycore`-subinterpreter-pool variant is a follow-up.

See [`python.md`](python.md) and [`python-api.md`](python-api.md).

### Backend feature flags

Six backend stubs are wired through feature flags: `sqlite` /
`postgres` for `Checkpointer`; `pgvector` / `qdrant` / `chroma` for
`LongStore`; `sqlite` / `redis` for `LlmCache`. The trait surface and
`connect(url)` constructors exist behind the flag; real wire-up
(sqlx / reqwest / redis) lives in deployment patches. Without a flag
enabled, none of those types compile in.

## Where this differs from atomr / atomr-infer

- atomr provides the **substrate**: actors, supervision, mailboxes,
  dispatchers, persistence, sharding, CRDTs.
- atomr-infer provides the **inference layer**: `ModelRunner` trait,
  per-provider runners, gateway / request actor / dp-coordinator,
  rate-limit + circuit-breaker + retry, `MockRunner`.
- atomr-agents adds the **agentic layer**: composable callables,
  channelled state, agent pipeline, tool-call orchestration,
  retriever zoo, parsers, cache, prompt templates, eval, and the
  Studio inspector — strictly on top, never replacing.

A `cargo check --workspace` against atomr-agents pulls atomr +
atomr-infer + atomr-accel as path deps. None of those crates know
or care about atomr-agents.
