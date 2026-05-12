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

## Python parity

The Python facade ships every Rust capability. The native extension
`atomr_agents._native` is split into 28 hierarchical submodules:
**foundational** (`errors`, `core`, `callable_`, `strategy`,
`instruction`, `context`, `state`, `observability`, `cache`,
`parser`, `registry`); **tool / skill / memory / retrieval / ingest**
(`tool`, `skill`, `memory`, `embed`, `retriever`, `ingest`,
`persona`); **agent / workflow / harness / org / eval** (`agent`,
`workflow`, `harness`, `org`, `eval`); **voice** (`stt`, `tts`,
`voice`, `voice_extras`); plus the **`guest`** registry. The
top-level package re-exports the most-used classes, ships a PEP 561
`py.typed` marker, and exposes async coroutines / async iterators
over `pyo3-async-runtimes`.

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

Mirror decorators are available for the full set of 24 Rust traits:
`@strategy(kind=...)`, `@persona`, `@skill`, `@parser`, `@scorer`,
`@memory_store`, `@embedder`, `@callable_`, `@retriever`, `@loader`,
`@splitter`, `@kv_cache`, `@long_store`, `@tracer`,
`@conversation_agent`, `@diarizer`, `@vad`, `@phonemizer`,
`@journal`, `@repair_model`, `@persona_reconciler`,
`@inference_client`, `@ann_index`. Each pairs with an
`atomr_agents.<module>.*_from_factory(key)` builder that
materialises the registered Python target as a Rust dyn handle.

### Host-mode agent runtime

`AgentBuilder` assembles strategy slots into a runnable `AgentRef`
that implements `Callable`, so an agent composes with the same
decorators and pipeline operators as any other unit:

```python
from atomr_agents.agent import AgentBuilder
from atomr_agents.harness import Harness, iteration_cap, loop_strategy_from_callable
from atomr_agents.workflow import Dag, Step, WorkflowRunner

# Strategy slots come from in-process factories or Python guests.
builder = AgentBuilder("research-agent", "gpt-4o-mini")
builder.with_instructions(instructions)
builder.with_tools(tool_strategy)
builder.with_memory(memory_strategy)
builder.with_skills(skill_strategy)
builder.with_inference(inference_client)
agent_ref = builder.build()
result = await agent_ref.run_turn("What's the GDP of France?")

# The agent is itself a Callable — drop it into a workflow.
dag = Dag("plan")
dag.add_step("plan", Step.invoke(agent_ref.as_callable()))
runner = WorkflowRunner("research-wf", dag.build())
await runner.run({"user": "..."})
```

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

### Runtime coverage

`AgentRef.run_turn`, `Harness.run`, `WorkflowRunner.run`, and
`Conversation` are all callable from Python. The Rust runtimes are
type-erased through `BoxedAgent` (in `crates/agent`) and `Box<dyn
LoopStrategy>` / `Box<dyn TerminationStrategy>` (in
`crates/harness`); the blanket `impl Trait for Box<dyn Trait>` impls
live in their respective trait crates so the composition contract
holds regardless of whether a strategy is monomorphic or boxed. See
[`docs/python-api.md`](docs/python-api.md) for the full module map
and async-surface table.

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
| `atomr-agents-stt-core` | `SpeechToText` / `StreamingSession` traits, `Capabilities` (advertised per backend via a `pub const`), `AudioInput` / `Transcript` / `StreamEvent`, `MockSpeechToText` |
| `atomr-agents-stt-remote-core` | Shared HTTP / WebSocket plumbing for cloud STT backends: `reqwest` client builder, `tokio-tungstenite` connect helper, `SecretRef` (env / literal / file), retry / rate-limit / timeout config |
| `atomr-agents-stt-audio` | `symphonia`-based decoder, `rubato` resampler, and (feature `mic`) `cpal`-based `MicCaptureSession` with backpressure-aware mpsc producer |
| `atomr-agents-stt-runtime-openai` | OpenAI Whisper / `gpt-4o-transcribe` REST batch backend |
| `atomr-agents-stt-runtime-deepgram` | Deepgram REST + WebSocket backend; speaker-count diarization, partial results, VAD endpointing |
| `atomr-agents-stt-runtime-assemblyai` | AssemblyAI REST upload + Universal-Streaming WebSocket; named-speaker diarization |
| `atomr-agents-stt-runtime-whisper` | Local whisper.cpp via `whisper-rs` (gated behind the `whisper-cpp` feature). Optional `download-models` helper fetches ggml weights |
| `atomr-agents-stt-diarize-sherpa` | `Diarizer` trait, `MockDiarizer`, sherpa-onnx-backed `SherpaDiarizer` (gated behind `sherpa-onnx`), `apply_to_transcript` stitching |
| `atomr-agents-stt-voice` | `VoiceSession` (`Live` vs `TurnBased { silence_ms }`), `Vad` trait + `EnergyVad`/`SileroVad`, `pump_mic_to_stream` glue |
| `atomr-agents-stt-tool` | `TranscribeTool` (a `Tool` the model can call) and `voice_input_skill(stt) -> (Skill, DynTool)` for declarative agent integration |
| `atomr-agents-py-bindings` | `atomr_agents._native` PyO3 module — 28 hierarchical submodules exposing every framework capability to Python (callable composition, strategies, instruction templates, memory + retriever zoo + ingest, agent / workflow / harness runtimes via `BoxedAgent`, eval, tracers, voice + conversation, 24 guest-trait decorators) |
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
