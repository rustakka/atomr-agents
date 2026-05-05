# atomr-agents

A native Rust **agentic framework** built as a layered actor / strategy
/ harness substrate on top of [atomr](https://github.com/rustakka/atomr)
and [atomr-infer](https://github.com/rustakka/atomr-infer). atomr-agents
gives you a single mental model — pluggable strategies that resolve
under shared budgets, channelled state with first-class checkpointing,
tool-call orchestration with parallel dispatch, and durable harness
loops — that scales from a one-off chatbot to a multi-tenant
production agent platform.

```rust
use atomr_agents::prelude::*;

// One Pipeline composes prompt → model → parser like LCEL.
let pipeline = Pipeline::from(prompt)
    .then(model)
    .then(parser)
    .build();

let answer = pipeline.call(input, ctx).await?;
```

## Why an agentic framework, in Rust, on actors

Agentic systems don't fail because the models aren't good enough —
they fail because the substrate underneath them treats context,
composition, and persistence as afterthoughts. Glue-code retry
policies, opaque memory, hand-rolled tool loops, brittle handoff
between agents — that's where 3 a.m. pages come from.

**Composition is the unit of work.** A real agent is a `Pipeline` of
prompts, models, parsers, and tools — each with its own retry,
fallback, timeout, cache, and trace policy. atomr-agents makes every
component a `Callable` with the same composition surface, so
`with_retry`, `with_fallbacks`, and `with_config` apply uniformly to
prompts, models, retrievers, and parsers alike.

**State is channelled, durable, and forkable.** Long-running agents
need more than chat history. They need **typed channels with
reducers** (`AppendMessages`, `MergeMap`, `LastWriteWins`,
`MaxByTimestamp`), **per-super-step checkpoints** keyed by `(workflow,
run, step)`, and **fork-with-edit** so an operator can branch a
divergent run from any prior state. atomr-agents ships LangGraph's
state model verbatim in atomr's actor idiom.

**Tool calls are parallel and provider-agnostic.** When a model emits
five tool calls in one turn, atomr-agents fans them into a `JoinSet`
and aggregates in original order. The streaming `tool_call_delta`
parser handles OpenAI and Anthropic deltas natively; new providers
plug in behind the same `Provider` enum. `RichTool` returns
`ToolReturn::{Content, ContentAndArtifact, Command}` so a tool can
also drive graph control flow.

**Granular efficiency.** Rust gives us deterministic resource use,
zero-cost abstractions, and ownership-as-concurrency-safety. Strategy
trait generics monomorphize the per-turn pipeline; `Box<dyn>` opt-in
exists for config-driven loading. The whole 26-crate workspace builds
under `cargo check --workspace` in seconds and ships **zero** runtime
overhead beyond what the actor crate already pays.

## What's in the box

| Crate | What it does |
|---|---|
| `atomr-agents` | Umbrella facade re-exporting the public surface, feature-flag-driven |
| `atomr-agents-core` | Ids, budgets (token / time / money / iteration), `AgentContext`, `RunId`, structured `Event` taxonomy, error types |
| `atomr-agents-callable` | `Callable` trait, `CallableHandle`, `Pipeline` builder (`then` / `fan_out` / `assign`), decorators (`with_retry` / `with_fallbacks` / `with_config` / `with_timeout` / `Branch` / `Lambda`) |
| `atomr-agents-strategy` | Strategy trait family (`ToolStrategy`, `MemoryStrategy`, `SkillStrategy`, `RoutingStrategy`, `PolicyStrategy`, `LoopStrategy`, `TerminationStrategy`) + combinators |
| `atomr-agents-context` | `ContextAssembler` — priority-merge under a `TokenBudget` |
| `atomr-agents-observability` | `EventBus`, `RunTree` builder, `Tracer` trait, `StdoutTracer` / `JsonlTracer` / `LangSmithTracer` |
| `atomr-agents-state` | `StateSchema` + 5 reducers, `RunState`, `Checkpointer` trait + `InMemoryCheckpointer`, fork-with-edit; SQLite/Postgres backend stubs behind features |
| `atomr-agents-tool` | `Tool` / `RichTool` traits, `ToolDescriptor`, `ToolSet` + `ToolSetRegistry`, `PermissionSpec`, provider-aware `ToolCallParser` (OpenAI / Anthropic), `HandoffTool` |
| `atomr-agents-skill` | `Skill`, `SkillSet`, `Static` / `Keyword` skill strategies |
| `atomr-agents-memory` | `MemoryStore` (short-term) + `LongStore` (long-term, namespace-tupled), `RecencyMemoryStrategy` / `SummarizingMemoryStrategy` / `ChainedMemoryStrategy`, `WriteMemoryTool` / `UpdateMemoryTool` / `RecallMemoryTool` |
| `atomr-agents-embed` | `Embedder` trait, `MockEmbedder`, `AnnIndex` + `InMemoryAnnIndex`, `EmbeddingToolStrategy` |
| `atomr-agents-retriever` | Retriever zoo: `Bm25` / `Vector` / `MultiQuery` / `ContextualCompression` / `ParentDocument` / `Ensemble` (RRF) / `SelfQuery` / `EmbeddingsFilter` / `TimeWeighted` |
| `atomr-agents-ingest` | `Loader` (text / md / json / csv) + splitters (`Recursive` / `MarkdownHeader` / `Code` / `Token` / `Semantic`) + `CachedEmbedder` + `IngestPipeline` |
| `atomr-agents-persona` | All five structural strategies (`Static`, `BigFive`, `Mbti`, `Jungian`, `Composite`) + emphasis strategies (`Static`, `AudienceAdaptive`, `TaskAdaptive`, `MoodState`, `GoalConditioned`) |
| `atomr-agents-instruction` | `ComposedInstructionStrategy<P, T, B>`, `ChatPromptTemplate`, `MessagesPlaceholder`, `FewShotChatTemplate`, `LengthBasedSelector` / `SemanticSimilaritySelector` |
| `atomr-agents-agent` | `Agent<I, T, Ms, Sk>` actor + per-turn pipeline, tool-call loop with parallel dispatch, `AgentMiddleware` (logging / retry / rate-limit / redaction / tool-error-recovery), `InferenceClient` adapter for any `ModelRunner` |
| `atomr-agents-workflow` | DAG primitives, `WorkflowRunner`, `StatefulRunner` (channelled state), `Interruptible` (`interrupt()` + `interrupt_before` / `_after` + `Command::{Continue, Resume, Update, Goto}`), `Subgraph`, `dispatch_fan_out` (Send-API analogue) |
| `atomr-agents-harness` | `Harness<L, T>` actor, `LoopStrategy` / `TerminationStrategy`, durable iteration log; `Harness` is itself a `Callable` |
| `atomr-agents-org` | `Org` / `Department` / `Team`, `OrgRoutingStrategy` impls (`RoundRobin` / `LoadAware` / `CapabilityMatch`), `Policy::narrow`, `NamespacedMemory` (read-cascade + write-gating), `swarm_loop` helper |
| `atomr-agents-registry` | Versioned artifact registry with `(kind, id, version)` keys + `publish_gated` for eval-regression blocking |
| `atomr-agents-eval` | `EvalSuite`, `Scorer` (Contains / Equality / Regex / `LlmJudgeScorer` / `RubricScorer` / `PairwiseScorer`), `RegressionGate`, `AnnotationQueue` |
| `atomr-agents-cache` | `LlmCache` trait + `InMemoryLlmCache` + `SemanticLlmCache` (cosine match on prompt embedding); SQLite/Redis backend stubs behind features |
| `atomr-agents-parser` | `Parser<T>` trait, `JsonParser` / `JsonSchemaParser` / `SchemaParser<T>` / `EnumParser` / `CommaListParser` / `XmlParser` / `YamlParser`, `OutputFixingParser`, `RetryWithErrorParser`, `StreamingPartialJsonParser` |
| `atomr-agents-py-bindings` | `atomr_agents._native` PyO3 module — `Event` / `EventBus` / `Registry` exposed to Python |
| `atomr-agents-cli` | `atomr-agents` binary with `eval` / `registry` / `harness` / `serve` (Studio-style read+resume inspector) subcommands |
| `atomr-agents-testkit` | Test fakes: `MockInference` (wraps atomr-infer's `MockRunner`), deterministic strategies, in-memory stores, replay harness |

Plus a Python facade — `pip install atomr-agents` — that exposes the
host-mode `Registry` / `EventBus` and the guest-mode `@tool` /
`@strategy` / `@persona` decorators.

## Quick start (Rust)

The umbrella crate is published on crates.io as **`atomr-agents`**:

```toml
[dependencies]
atomr-agents = { version = "0.1", features = ["agent", "harness", "eval"] }
atomr-infer  = { version = "0.4", features = ["openai"] }   # or any provider
```

A minimal agent against `MockRunner` (good for tests; swap for any
`ModelRunner` in production):

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
    model: "mock".into(),
    instructions: ComposedInstructionStrategy::new(
        StaticPersonaStrategy::new("You are a helpful assistant."),
        StaticTaskStrategy("Answer arithmetic questions.".into()),
        StaticBehaviorStrategy("Reply tersely.".into()),
    ),
    tools: StaticToolStrategy::new(Vec::<DynTool>::new()),
    memory: RecencyMemoryStrategy::new(Arc::new(InMemoryStore::new()), 5, 30),
    skills: StaticSkillStrategy::new(vec![]),
    inference,
    bus: EventBus::new(),
    max_tool_iterations: 3,
};

let r = agent
    .run_turn("what's 1+2".into(), AgentBudgets::default())
    .await?;
println!("{}", r.text);
```

Add tools, switch the `MockRunner` to a real `ModelRunner` (OpenAI,
Anthropic, vLLM, …), and the same code runs unchanged.

## Quick start (Python)

```bash
pip install atomr-agents
```

```python
from atomr_agents import EventBus, Registry

bus = EventBus()
bus.subscribe(lambda ev: print(ev.kind, ev.timestamp_ms))

registry = Registry()
registry.publish("tool_set", "ts", "0.1.0", {"tools": ["calc"]})
print(registry.latest("tool_set", "ts"))
```

See `docs/python.md` for the full host/guest model and the
subinterpreter-pool dispatcher pattern inherited from atomr's pycore.

## Documentation map

- [`docs/index.md`](docs/index.md) — documentation hub
- [`docs/architecture.md`](docs/architecture.md) — runtime layout, crate stack, where each layer slots in
- [`docs/state-and-checkpointing.md`](docs/state-and-checkpointing.md) — channels, reducers, `Checkpointer`, fork/replay
- [`docs/agent-pipeline.md`](docs/agent-pipeline.md) — the per-turn pipeline + tool-call loop + middleware
- [`docs/workflows-and-hitl.md`](docs/workflows-and-hitl.md) — DAG, Send-API, dynamic interrupts, breakpoints
- [`docs/retrieval-and-ingestion.md`](docs/retrieval-and-ingestion.md) — retriever zoo, `LongStore`, loaders, splitters
- [`docs/observability.md`](docs/observability.md) — `EventBus`, `RunTree`, tracers
- [`docs/eval.md`](docs/eval.md) — eval suites, judge / pairwise / rubric scorers, regression gate
- [`docs/multi-agent-patterns.md`](docs/multi-agent-patterns.md) — supervisor / swarm / network / hierarchical
- [`docs/feature-matrix.md`](docs/feature-matrix.md) — every feature flag, what it pulls in
- [`docs/python.md`](docs/python.md) — Python bindings + subinterpreter-pool guest mode
- [`docs/migrating-from-langgraph.md`](docs/migrating-from-langgraph.md) — concept-mapping table + concrete code translations
- [`ai-skills/`](ai-skills/) — Claude Code / Agent SDK skills for AI-assisted coding against atomr-agents

## License

Apache-2.0.
