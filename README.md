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

## Python parity wave

The Python facade in 0.3 catches up to the Rust surface. The native
extension `atomr_agents._native` is now split into hierarchical
submodules — `errors`, `core`, `observability`, `registry`, `tool`,
`skill`, `persona`, `agent`, `workflow`, `harness`, `eval`, `guest` —
mirroring `atomr-infer/inference-py-bindings`. The top-level package
re-exports the full surface, ships a PEP 561 `py.typed` marker, and
exposes async coroutines / async iterators over `pyo3-async-runtimes`.

### Install

```bash
pip install atomr-agents
```

For an editable workflow against the local checkout:

```bash
pip install maturin
maturin develop --features python -m crates/py-bindings/Cargo.toml
pip install -e ".[dev]"
```

### Host-mode async event stream

`EventBus.stream()` returns an `EventStream` that implements the
Python async iterator protocol. Drive a producer on the same loop
and consume events as they fire:

```python
import asyncio
from atomr_agents.observability import EventBus


async def main() -> None:
    bus = EventBus()
    stream = bus.stream()

    bus.emit_tool_invoked("calc", args_hash=0, elapsed_ms=5, ok=True)
    bus.emit_tool_invoked("search", args_hash=1, elapsed_ms=12, ok=True)

    async for ev in stream:
        print(ev.kind, ev.timestamp_ms)
        if ev.kind == "tool_invoked" and ev.tool == "search":
            break


asyncio.run(main())
```

### Async registry publish

`Registry.publish_async` returns a Python awaitable backed by a
`tokio` future, so version pins land without blocking the event loop:

```python
import asyncio
from atomr_agents.registry import Registry


async def main() -> None:
    registry = Registry()
    record = await registry.publish_async(
        "tool_set", "calc", "0.1.0", {"name": "calc"}
    )
    print(record.kind, record.id, record.version)


asyncio.run(main())
```

### Guest-mode `@tool` decorator

`atomr_agents.guest` exposes real decorators wired through
`_native.guest.register_*_factory`. A guest tool is a class with an
`async def invoke(self, args, ctx)` method:

```python
from atomr_agents.guest import tool


@tool(toolset="calc")
class Add:
    name = "add"

    async def invoke(self, args: dict, ctx) -> dict:
        return {"sum": args["a"] + args["b"]}
```

Mirror decorators are available for `@strategy`, `@persona`,
`@skill`, `@parser`, `@scorer`, `@memory_store`, and `@embedder`.

### Where things live

The hierarchical layout is reflected in the Python facade — every
submodule has a one-to-one `.py` mirror under `atomr_agents/`:

```python
from atomr_agents.errors import RegistryError
from atomr_agents.core import TokenBudget, AgentId
from atomr_agents.agent import AgentSpec, AgentBudgets
from atomr_agents.tool import ToolDescriptor, ToolCallParser
from atomr_agents.observability import EventBus, RunTreeBuilder
from atomr_agents.registry import Registry
```

The top-level package keeps the 0.2.x convenience names — so
`from atomr_agents import EventBus, Registry` still works.

### Roadmap

`Agent.run_turn`, `Harness.run`, and `WorkflowRunner.run` are not
yet exposed as Python coroutines. The Rust types are generic over
four-plus strategy traits, so PyO3 cannot construct them from a
stable `#[pyclass]` shape; they need a `Boxed*` adapter
(`BoxedAgent` / `BoxedHarness` / `BoxedWorkflow`) under
`crates/agent` / `crates/harness` / `crates/workflow`. Until that
adapter lands, host code drives the loop in Rust and observes
progress over the (already async-iterable) `EventBus`. See
[`docs/python-api.md`](docs/python-api.md) for the full module map.

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
plug in behind the same `Provider` enum. Per-call deltas are also
surfaced as `Event::ToolCallStreamed` so tracers and UIs see tool
intent in real time, distinct from the post-call `Event::ToolInvoked`.
`RichTool` returns `ToolReturn::{Content, ContentAndArtifact, Command}`
so a tool can also drive graph control flow.

**Provider runtimes are opt-in feature flags.** Enable
`provider-anthropic`, `provider-openai`, or `provider-gemini` on the
umbrella to pull the corresponding `atomr-infer-runtime-*` crate and
re-export its `*Config` / `*Pricing` / `*Runner` via
`atomr_agents::agent::providers::{anthropic, openai, gemini}`. Cost
reports include `cached_tokens` (Anthropic prompt-cache, OpenAI cached
input) and `reasoning_tokens` (o1-style) automatically.

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
| `atomr-agents-testkit` | Stub crate today. For tests, depend on `atomr-infer-testkit` (re-exports `MockRunner` / `MockScript`) directly — that's what `crates/agent` tests use. |

Plus a Python facade — `pip install atomr-agents` — that exposes the
host-mode `Registry` / `EventBus` and the guest-mode `@tool` /
`@strategy` / `@persona` decorators.

## Quick start (Rust)

The umbrella crate is published on crates.io as **`atomr-agents`**:

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["agent", "harness", "eval"] }
atomr-infer  = { version = "0.6", features = ["openai"] }   # or any provider
```

Or, to pull a provider runtime through the umbrella so `Agent` /
`LocalRunnerClient` / `OpenAiRunner` come from one crate:

```toml
atomr-agents = { version = "0.2", features = ["agent", "provider-openai"] }
# or features = ["agent", "provider-anthropic"], ["agent", "provider-gemini"]
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
- [`docs/python-api.md`](docs/python-api.md) — Python API reference: submodule map, async surfaces, 0.2 → 0.3 migration
- [`docs/migrating-from-langgraph.md`](docs/migrating-from-langgraph.md) — concept-mapping table + concrete code translations
- [`ai-skills/`](ai-skills/) — Claude Code / Agent SDK skills for AI-assisted coding against atomr-agents

## License

Apache-2.0.
