# atomr-agents

A native Rust agentic framework built as a layered actor / strategy /
harness substrate on top of [atomr](https://github.com/rustakka/atomr)
and [atomr-infer](https://github.com/rustakka/atomr-infer). One
programming model — pluggable strategies that resolve under shared
budgets, channelled state with first-class checkpointing, tool-call
orchestration with parallel dispatch, and durable harness loops —
that scales from a one-off chatbot to a multi-tenant production agent
platform.

The framework is small at the edge — `Callable` + `Pipeline` + a few
strategy traits — and grows outward into channelled state, durable
checkpointing, dynamic human-in-the-loop interrupts, retriever zoo,
ingestion pipeline, agent middleware, multi-agent topologies, eval
suites with judge / pairwise / regression scorers, and a Studio-style
read+resume inspector. The same composition contract holds at every
layer.

## Why this design

**Composition is the unit of work.** Production agent code is a long
sequence of prompts, models, parsers, tools, and retrievers — each
with its own retry, fallback, timeout, cache, and trace policy.
Frameworks that hide composition behind a runtime "agent executor"
turn every customization into a fork. atomr-agents makes every
component a `Callable` and every policy a decorator, so
`with_retry(...)`, `with_fallbacks(...)`, and `with_config(...)`
apply uniformly across the entire pipeline.

**State has a shape.** Long-running agents need typed channels with
reducers (`AppendMessages`, `MergeMap`, `LastWriteWins`,
`MaxByTimestamp`, `AppendList`), per-super-step checkpoints scoped
to `(workflow_id, run_id, super_step)`, and a `fork` operation that
creates a divergent run from any prior checkpoint with optional state
edits. This is the LangGraph state model, expressed in atomr's actor
idiom — and **necessary for HITL, time-travel, multi-agent merging,
and parallel-write resolution**.

**Tools are parallel and provider-agnostic.** When a model emits
multiple tool calls in a single assistant turn, atomr-agents fans
them into a `JoinSet` and aggregates by original index. The streaming
`tool_call_delta` parser handles OpenAI and Anthropic deltas natively;
new providers add a `Provider` variant rather than a new pipeline.
`RichTool` lets a tool return `ToolReturn::{Content,
ContentAndArtifact, Command}` so a tool can also drive graph control
flow (handoff, goto, state update, done).

**Rust earns the granularity.** The strategy trait family
(`InstructionStrategy`, `ToolStrategy`, `MemoryStrategy`,
`SkillStrategy`, `LoopStrategy`, …) monomorphizes the per-turn hot
path; `Box<dyn>` opt-in exists for config-driven instantiation. The
136-test workspace builds clean under `cargo check --workspace`. The
underlying actor model from atomr (mailboxes, supervision,
dispatchers, persistence, sharding) is carried through unchanged.

## At a glance

- **Composable callables** — `Callable` trait, `Pipeline` builder
  (`then` / `fan_out` / `assign`), decorators (`with_retry`,
  `with_fallbacks`, `with_config`, `with_timeout`, `Branch`,
  `Lambda`).
- **Channelled state + durable checkpoints** — `StateSchema`,
  five built-in reducers, `RunState`, `Checkpointer` trait,
  `InMemoryCheckpointer` with fork-with-edit; SQLite + Postgres
  backend stubs gated on features.
- **HITL interrupts + breakpoints** — dynamic `interrupt()` from
  inside a step, static `interrupt_before` / `interrupt_after`,
  `Command::{Continue, Resume, Update, Goto}` resume API.
- **Send-API + Command-return + parallel tools** — `Step::Dispatch`
  for runtime fan-out, `dispatch_fan_out` helper, `ToolReturn` enum,
  `JoinSet`-backed parallel tool dispatch in the agent turn.
- **Subgraphs with shared channels** — workflow-as-callable with
  declared `input_channels` / `output_channels` projection.
- **Long-term `Store` API** — namespace-tupled, embedding-indexed,
  cross-thread; `WriteMemoryTool` / `UpdateMemoryTool` /
  `RecallMemoryTool` available as built-in tools.
- **Retriever zoo** — BM25, dense vector, MultiQuery, contextual
  compression, parent-document, RRF ensemble, self-query (NL →
  filter+query), embeddings filter, time-weighted decay.
- **Document ingestion** — text / markdown / json / csv loaders;
  recursive / markdown-header / code / token / semantic splitters;
  `CachedEmbedder`; one-call `ingest()` helper.
- **Agent middleware** — `wrap_model_call` / `wrap_tool_call` /
  `dynamic_prompt` / `before_agent` / `after_agent` hooks; ships
  `Logging`, `RateLimit` (token-bucket), `Redaction`,
  `ToolErrorRecovery`.
- **Output parsers + structured output** — JSON / JsonSchema /
  Pydantic-style `SchemaParser<T>` / Enum / CSV / XML / YAML;
  `OutputFixingParser`, `RetryWithErrorParser`,
  `StreamingPartialJsonParser`.
- **Prompt templates + few-shot** — `ChatPromptTemplate` with
  `MessagesPlaceholder`, `FewShotChatTemplate`,
  `LengthBasedSelector` / `SemanticSimilaritySelector`.
- **LLM cache** — `InMemoryLlmCache` and `SemanticLlmCache` (cosine
  match on prompt embedding); SQLite + Redis backend stubs.
- **Multi-agent patterns** — `Org` / `Department` / `Team` with
  `RoundRobin` / `LoadAware` / `CapabilityMatch` routing; reference
  patterns for supervisor / swarm / network / hierarchical;
  `HandoffTool` helper.
- **Eval suites** — `Contains` / `Equality` / `Regex` /
  `LlmJudgeScorer` / `RubricScorer` / `PairwiseScorer`,
  `RegressionGate`, `AnnotationQueue`.
- **Run-tree observability** — `EventBus` with `RunId` /
  `parent_run_id`, `RunTreeBuilder`, `Tracer` trait,
  `StdoutTracer` / `JsonlTracer` / `LangSmithTracer`.
- **Versioned registry** — `(kind, id, version)` keys,
  `publish_gated` for eval-regression blocking.
- **Python bindings** — `atomr_agents._native` exposes `EventBus` and
  `Registry`; guest-mode `@tool` / `@strategy` / `@persona`
  decorators ride on atomr's `python-subinterpreter-pool` dispatcher.

## Getting started

### Rust

```bash
cargo build --workspace
cargo test  --workspace
cargo run   -p atomr-agents-harness --example research_harness
```

### Python

```bash
maturin develop --manifest-path crates/py-bindings/Cargo.toml
python -c "from atomr_agents import Registry; print(Registry())"
```

## Documentation map

- [Architecture](architecture.md) — runtime layout, crate stack, where each layer slots in.
- [State and checkpointing](state-and-checkpointing.md) — channels, reducers, `Checkpointer`, fork/replay.
- [Agent pipeline](agent-pipeline.md) — per-turn pipeline + tool-call loop + middleware.
- [Workflows and HITL](workflows-and-hitl.md) — DAG, Send-API, dynamic interrupts, breakpoints.
- [Retrieval and ingestion](retrieval-and-ingestion.md) — retriever zoo, `LongStore`, loaders, splitters.
- [Observability](observability.md) — `EventBus`, `RunTree`, tracers.
- [Eval](eval.md) — eval suites, judge / pairwise / rubric scorers, regression gate.
- [Multi-agent patterns](multi-agent-patterns.md) — supervisor / swarm / network / hierarchical.
- [Feature matrix](feature-matrix.md) — every feature flag, what it pulls in.
- [Python bindings](python.md) — host-mode + guest-mode, GIL containment.
- [Migrating from LangGraph / LangChain](migrating-from-langgraph.md) — concept map and code translations.
- [`../README.md`](https://github.com/rustakka/atomr-agents) — repository overview.
- [`../ai-skills/`](https://github.com/rustakka/atomr-agents/tree/main/ai-skills) — skills for AI-assisted coding.
