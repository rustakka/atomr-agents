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
  `StdoutTracer` / `JsonlTracer` / `LangSmithTracer`. `Event::AgentTurn`
  carries `reasoning_tokens` + `cached_tokens` for accurate cost
  reporting under prompt-cache and o1-style usage; `Event::ToolCallStreamed`
  fires per detected tool call before dispatch.
- **Pluggable provider back-ends** — `provider-anthropic`,
  `provider-openai`, `provider-gemini` features pull the corresponding
  `atomr-infer-runtime-*` crate and re-export it under
  `agent::providers::*`. Wire a runner without a direct `atomr-infer`
  dep.
- **Versioned registry** — `(kind, id, version)` keys,
  `publish_gated` for eval-regression blocking.
- **Python bindings — full surface** — `atomr_agents._native`
  ships 28 hierarchical submodules covering the entire framework:
  `Callable` + `Pipeline` composition, strategy dyn handles,
  prompt templates, `Embedder` / `AnnIndex`, short + long memory
  stores, the retriever zoo, ingest pipeline, `AgentBuilder` /
  `Harness` / `WorkflowRunner` runtimes (built on type-erased
  `BoxedAgent` / `Box<dyn LoopStrategy>` / `Box<dyn
  TerminationStrategy>`), `Org` / `Department` / `Team`, eval
  suites + scorers + `RegressionGate`, `Tracer` family, parser
  fixers, bidirectional `Conversation`, diarizer / VAD /
  phonemizer, and feature-gated SQLite / Redis / Postgres
  backends. Twenty-four guest decorators (`@tool`, `@strategy`,
  `@retriever`, `@loader`, `@callable_`, `@inference_client`, …)
  register Python implementations as Rust trait objects.

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
- [STT harness](stt-harness.md) — agentic streaming speech-to-text pipeline, diarization, the editable diarized-transcript review UI.
- [Meetings harness](meetings-harness.md) — downstream of STT: attendees, notes, actions, tiered summaries from a diarized transcript.
- [Avatar harness](avatar-harness.md) — real-time embodied agent: perception → cognition → TTS → 60 Hz sync manager → CBOR-over-UDP `LiveLinkSink` to a UE5 MetaHuman. Includes Ubuntu setup, x86/ARM architecture rules, the current (post-web-Creator) MetaHuman authoring workflow, and a full `ILiveLinkSource` receiver-plugin skeleton.
- [Deep research harness](deep-research-harness.md) — multi-step, multi-source, citation-bearing research over a user query, with three pluggable v1 topologies (AI-Q, Anthropic multi-agent, LangGraph open_deep_research).
- [Coding CLI harness](coding-cli-harness.md) — wraps local AI coding CLIs (Claude Code, Codex CLI, Antigravity CLI) as atomr-agents callables: headless mode parses structured events; interactive mode bridges a tmux session to xterm.js. Local or Docker isolation.
- [Agent host](agent-host/index.md) — long-lived on-disk runtime (SOUL / RULES / MEMORY / USER / SKILL.md per agent) that gives an atomr-agents agent persistent identity, skills, hooks, schedules, and inbound channels. The `atomr-host` CLI does for atomr-agents what Claude Code does for the Claude model.
- [Eval](eval.md) — eval suites, judge / pairwise / rubric scorers, regression gate.
- [Multi-agent patterns](multi-agent-patterns.md) — supervisor / swarm / network / hierarchical.
- [Feature matrix](feature-matrix.md) — every feature flag, what it pulls in.
- [Python bindings](python.md) — host-mode + guest-mode, GIL containment.
- [Python API reference](python-api.md) — module-by-module map of `atomr_agents.*`, async surfaces, 0.2 → 0.3 migration.
- [Migrating from LangGraph / LangChain](migrating-from-langgraph.md) — concept map and code translations.
- [`../README.md`](https://github.com/rustakka/atomr-agents) — repository overview.
- [`../ai-skills/`](https://github.com/rustakka/atomr-agents/tree/main/ai-skills) — skills for AI-assisted coding.
