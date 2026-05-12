# Python API reference

This page is the structural map of the `atomr_agents` Python package
on `main`. Layout mirrors the upstream
[`atomr-infer/inference-py-bindings`](https://github.com/rustakka/atomr-infer/tree/main/crates/inference-py-bindings/src)
and [`atomr/pycore`](https://github.com/rustakka/atomr/tree/main/crates/py-bindings/pycore)
binding crates so consumers who already use the sibling Python
surfaces find the same idioms here: a hierarchical `_native.{...}`
PyO3 module, one facade `.py` per submodule, async coroutines via
`pyo3-async-runtimes`, an exception hierarchy rooted at
`AgentError`, and a guest-mode decorator family that registers
Python implementations as Rust trait objects.

For the prose-level introduction to host vs guest mode and the GIL
strategy, read [`python.md`](python.md) first. This document is the
type-by-type module map.

## Module map

The native extension is `atomr_agents._native`. Every submodule has
a sibling facade `.py` under `python/atomr_agents/` that re-exports
its public surface, so user code never has to import from
`_native.*` directly.

### Foundational

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `errors` | Exception hierarchy translating Rust `AgentError` | `AgentError`, `RegistryError`, `BudgetExhausted`, `ToolError`, `StrategyError`, `WorkflowError`, `HarnessError`, `EvalError`, `MemoryError`, `ParserError`, `CacheError` |
| `core` | IDs, budgets, memory primitives, token usage | `AgentId`/`TeamId`/`DepartmentId`/`OrgId`/`WorkflowId`/`HarnessId`/`ToolId`/`ToolSetId`/`SkillId`/`PersonaId`/`RunId`; `TokenBudget`/`TimeBudget`/`MoneyBudget`/`IterationBudget`; `MemoryNamespace`/`MemoryKind`/`MemoryItem`/`MemoryChunk`; `TokenUsage`; `FinishReason` |
| `callable_` | Universal composition handle | `Callable` (dyn handle), `Pipeline` (builder), free fns `with_retry`, `with_timeout`, `with_fallbacks`, `with_config`, `fan_out`, `branch`, `lambda_`, `passthrough` |
| `strategy` | Strategy enums + dyn-handle adapters | `Termination`, `RoutingTarget`, `SkillRef`, `ToolRef`, `Policy`, `PolicyDecision`; dyn handles `MemoryStrategyHandle`/`SkillStrategyHandle`/`ToolStrategyHandle`/`RoutingStrategyHandle`/`PolicyStrategyHandle`; `*_strategy_from_factory(key)` builders |
| `context` | Token-budgeted fragment assembly | `ContextFragment`, `RenderedContext`, `assemble(...)` |
| `state` | Checkpointing | `Checkpointer` dyn handle, `InMemoryCheckpointer`, reducers (`LastWriteWins`, `AppendList`, `AppendMessages`, `MergeMap`, `MaxByTimestamp`); feature-gated `sqlite_checkpointer` / `postgres_checkpointer` |
| `observability` | Event bus + tracers | `Event`, `EventBus`, `EventStream` (async iterator), `RunTreeBuilder`, `Tracer` dyn handle; `jsonl_tracer`, `lang_smith_tracer`, `stdout_tracer` |
| `cache` | LLM response caches | `InMemoryLlmCache`, `LlmCache` dyn handle; `semantic_llm_cache`; feature-gated `sqlite_llm_cache` / `redis_llm_cache` |
| `parser` | Output parsers | `JsonParser`, `JsonSchemaParser`, `CommaListParser`, `XmlParser`, `YamlParser`, `StreamingPartialJsonParser`, `Parser` (unified dyn handle), `RepairModel` dyn handle; `output_fixing_parser`, `retry_with_error_parser`, `enum_parser`, `schema_parser`, `repair_model_from_factory` |
| `registry` | Versioned artifact registry | `Registry`, `ArtifactKind`, `ArtifactRecord`, `EvalSummary` |
| `instruction` | Prompt templates + instruction strategies | `RenderedInstructions`, `RenderedMessage`, `ChatPromptTemplate`(+Builder), `FewShotChatTemplate`, `MessageTemplate`, `MessagesPlaceholder` (alias), `StringTemplate`, `Example`, `ExampleSelector`, `length_based_selector`, `semantic_similarity_selector`; `InstructionStrategy`/`TaskStrategy`/`BehaviorStrategy` dyn handles; `task_static`, `behavior_static`, `*_from_factory` |

### Tool / Skill / Memory / Retrieval / Ingest

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `tool` | Tool descriptors, schemas, parser, strategies | `ToolSchema`, `ToolDescriptor`, `Provider`, `ParsedToolCall`, `ToolCallParser`, `ToolSet`; `HandoffTool`, `RichTool`, `ToolControl`, `ToolReturn`, `PermissionSpec`; `static_tool_strategy`, `keyword_tool_strategy`, `handoff_tool`, `transcribe_tool`, `speak_tool` |
| `skill` | Skill primitives + strategies | `Skill`, `SkillSet`; `static_skill_strategy`, `keyword_skill_strategy`; `voice_input_skill`, `voice_speak_skill`, `voice_response_skill` |
| `memory` | Short + long memory stores | `MemoryStore`/`LongStore` dyn handles, `Namespace`, `StoreItem`; `in_memory_store`, `in_memory_long_store`, `recency_memory_strategy`, `summarizing_memory_strategy`, `chained_memory_strategy`, `*_from_factory` |
| `embed` | Embedders + ANN index | `Embedder`/`AnnIndex` dyn handles; `mock_embedder`, `in_memory_ann_index`, `embedding_tool_strategy`, `*_from_factory` |
| `retriever` | Document retriever zoo | `Document`, `Retriever` dyn handle; `bm25_retriever`, `vector_retriever`, `multi_query_retriever`, `contextual_compression_retriever`, `parent_document_retriever`, `ensemble_retriever`, `self_query_retriever`, `time_weighted_retriever`, `retriever_from_factory` |
| `ingest` | Document loaders, splitters, pipeline | `Loader`/`Splitter`/`KvCache` dyn handles, `CodeLang` enum, `IngestPipeline` builder; `text_loader`, `markdown_loader`, `csv_loader`, `json_loader`, `recursive_character_splitter`, `token_splitter`, `markdown_header_splitter`, `code_splitter`, `semantic_splitter`, `in_memory_kv_cache`, `cached_embedder`, `ingest(...)` |
| `persona` | Persona strategies | `RenderedPersona`, `PersonaSet`, `PersonaMetadata`, `StyleSpec`, `TraitFragment`; `MbtiType`, `Archetype`, `CognitiveFunction`, `CognitiveStack`; `Persona` (strategy dyn handle), `PersonaEmphasis`, `PersonaReconciler` dyn handle; `static_persona_strategy`, `mbti_persona_strategy`, `jungian_archetype_strategy`, `big_five_persona_strategy`, `composite_persona_strategy`, `static_emphasis`, `task_adaptive`, `audience_adaptive`, `goal_conditioned`, `mood_state`, `*_from_factory` |

### Agent / Workflow / Harness / Org / Eval

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `agent` | Agent runtime via `BoxedAgent` | `AgentSpec`, `AgentBudgets`, `TurnResult`, `AgentBuilder`, `AgentRef`, `InferenceClient`, `AgentMiddleware`; `inference_client_from_factory`, `logging_middleware`, `tool_error_recovery_middleware`, `redaction_middleware`, `rate_limit_middleware` |
| `workflow` | Dag + step runtime | `StepKind`, `StepId`, `Step` (with classmethods `invoke` / `branch` / `parallel` / `loop_` / `map` / `human`), `Dag`, `DagHandle`, `WorkflowRunner`, `WorkflowState`, `Journal` dyn handle; `in_memory_journal`, `journal_from_factory`, `fan_out_dispatch` |
| `harness` | Loop harness runtime | `HarnessSpec`, `Harness`, `HarnessState`, `StepEvent`, `LoopStrategy` and `TerminationStrategy` dyn handles, `IterationCapTermination`; `iteration_cap`, `loop_strategy_from_callable`, `loop_strategy_from_factory`, `termination_from_factory` |
| `org` | Multi-agent topologies | `Org`, `Department`, `Team`, `OrgRoutingStrategyHandle`, `ActiveAgent`; `round_robin_router`, `load_aware_router`, `capability_match_router`, `namespaced_memory`, `swarm_loop` |
| `eval` | Eval suites + scorers + regression | `PairwiseChoice`, `Verdict`; `RubricCriterion`, `JudgeModel`, `EvalCase`, `EvalResult`, `EvalRun`, `EvalSuite`, `Scorer`/`PairwiseScorer`/`AnnotationQueue` dyn handles; `rubric_scorer`, `llm_judge_scorer`, `pairwise_scorer`, `in_memory_annotation_queue`, `regression_gate`, `scorer_from_factory` |

### STT / TTS / Voice

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `stt` | Speech-to-text + tools | `SpeechToText`, `Capabilities`, `AudioInput`, `Transcript`, `StreamingSession`, `StreamEvent`; `mock_speech_to_text`, `stt_openai`, `stt_deepgram`, `stt_assemblyai`, `stt_whisper`, `audio_file`, `audio_bytes`, `audio_pcm`, `transcribe_tool`, `voice_input_skill` |
| `tts` | Text-to-speech + tools | `TextToSpeech`, `SynthesisRequest`, `VoiceRef`, `Capabilities`, `AudioOutput`, `AudioChunk`, `SynthesisStream`, `RealtimeEvent`, `RealtimeSession`; `mock_tts`, `tts_openai`, `tts_elevenlabs`, `tts_openai_realtime`, `tts_gemini_live`, `tts_piper`, `tts_kokoro`, `tts_moss`, `tts_xtts`, `voice_library`, `voice_described`, `voice_cloned`, `tts_request`, `sfx_request`, `dialogue_request`, `speak_tool`, `voice_speak_skill`, `voice_response_skill` |
| `voice` | Bidirectional conversation | `VoiceMode`, `VoiceSession`, `VoiceEvent`; `ConversationMode`, `ConversationOptions`, `ConversationAgent` dyn handle, `Conversation`, `InboundTranscript`, `ConversationEvent`; `noop_agent`, `conversation_agent_from_factory` |
| `voice_extras` | Diarization, VAD, phonemization | `Diarizer`, `Vad`, `Phonemizer` dyn handles; `PhonemizedText`, `DiarizationSpan`; `mock_diarizer`, `sherpa_diarizer` (feature-gated), `energy_vad`, `silero_vad` (feature-gated), `mock_phonemizer`, `*_from_factory` |

### Guest registration

| `atomr_agents.<submodule>` | Purpose | Key types |
|---|---|---|
| `guest` | Python-implementable Rust trait factories | `GuestHandle`; `register_*_factory(...)` for tool, strategy, persona, skill, parser, scorer, memory, embedder, callable_, retriever, loader, splitter, kv_cache, long_store, tracer, conversation_agent, diarizer, vad, phonemizer, journal, repair_model, persona_reconciler, inference_client, ann_index. User-facing decorators: `@tool`, `@strategy(kind=...)`, `@persona`, `@skill`, `@parser`, `@scorer`, `@memory_store`, `@embedder`, `@callable_`, `@retriever`, `@loader`, `@splitter`, `@kv_cache`, `@long_store`, `@tracer`, `@conversation_agent`, `@diarizer`, `@vad`, `@phonemizer`, `@journal`, `@repair_model`, `@persona_reconciler`, `@inference_client`, `@ann_index` |

The top-level `atomr_agents.__init__` re-exports the most commonly
used classes from each submodule (`Callable`, `Pipeline`,
`AgentBuilder`, `Harness`, `WorkflowRunner`, `Registry`, `EventBus`,
`TokenBudget`, …), so `from atomr_agents import Harness` resolves
without users having to remember submodule paths. `__version__` is
sourced from `importlib.metadata`.

## Universal composition: `Callable`

Every dispatchable Rust type (agent, workflow, harness, retriever,
tool, ingest pipeline) is bound to Python as a `Callable`. The
underlying Rust type may be `Arc<dyn Callable>`, but Python sees a
uniform `.call(input, ctx) -> awaitable` signature and composes the
result with free functions that themselves return `Callable`s:

```python
from atomr_agents import callable_, Pipeline

double = callable_.Callable.from_callable(lambda v, _: v * 2)
add_one = callable_.Callable.from_callable(lambda v, _: v + 1)

pipe = Pipeline.from_(double)
pipe.then(add_one)
built = pipe.build()  # -> Callable

await built.call(5)   # -> 11
```

Decorators are free functions: `with_retry(c, max_attempts=3,
initial_backoff_ms=50)`, `with_timeout(c, ms)`,
`with_fallbacks(c, [alt1, alt2])`, `with_config(c, run_name=…,
tags=[…], metadata={…})`, `fan_out({"a": c1, "b": c2})`,
`branch(predicate, if_true, if_false)`. All return a fresh
`Callable`.

## Host mode

Host mode is the *Python-drives-Rust* pattern. The recommended host
loop:

1. Build foundational components: `EventBus`, `Registry`,
   in-memory stores, embedders.
2. Build strategy slots from Python guests or in-process factories
   (`mbti_persona_strategy(...)`, `recency_memory_strategy(...)`,
   `static_tool_strategy(...)`, etc.).
3. Build an `InferenceClient` — either from a sibling
   `atomr_infer.providers.*` runner wrapped via
   `inference_client_from_factory(key)`, or via a guest factory
   that implements `provider()` + `async run(batch)`.
4. Assemble with `AgentBuilder`:
   ```python
   from atomr_agents.agent import AgentBuilder
   builder = AgentBuilder("agent-1", "gpt-4o-mini")
   builder.with_instructions(instruction_strategy)
   builder.with_tools(tool_strategy)
   builder.with_memory(memory_strategy)
   builder.with_skills(skill_strategy)
   builder.with_inference(inference_client)
   agent_ref = builder.build()
   ```
5. Run a turn:
   ```python
   result = await agent_ref.run_turn("hello")
   print(result.text, result.usage.total_tokens)
   ```
   Or compose the agent as a `Callable`:
   ```python
   await agent_ref.as_callable().call({"user": "hello"})
   ```
6. Subscribe to events asynchronously:
   ```python
   async for ev in bus.stream():
       print(ev.kind, ev.timestamp_ms)
   ```

`WorkflowRunner` and `Harness` follow the same shape — construct from
the typed factories, then `.run(...)` or `.as_callable()`.

## Guest mode

Guest mode is the *Python-defined-strategy-runs-inside-Rust* pattern.
Decorate a class or function with the matching `atomr_agents.guest`
decorator; the decorator registers it with the process-wide native
guest registry and the Rust adapter materialises it on demand.

```python
from atomr_agents.guest import tool, strategy, retriever, embedder


@tool(toolset="finance")
class DiscountedCashFlow:
    name = "dcf"

    async def invoke(self, args, ctx):
        rate = args["rate"]
        cashflows = args["cashflows"]
        return {
            "npv": sum(cf / (1 + rate) ** i for i, cf in enumerate(cashflows)),
        }


@retriever()
class MyRetriever:
    async def retrieve(self, query, ctx):
        # Must return list of {"id", "page_content", "metadata"?, "score"?}.
        return [{"id": "doc1", "page_content": "hit"}]


@embedder()
class HashEmbedder:
    def dim(self):
        return 8

    async def embed(self, text):
        # Return a list[float] of length self.dim().
        ...
```

Materialise into a Rust trait object via the matching `*_from_factory(key)`:

```python
from atomr_agents import retriever as ret
r = ret.retriever_from_factory("MyRetriever")
docs = await r.retrieve("query")
```

The full guest-trait set: `tool`, `strategy(kind=…)`, `persona`,
`skill`, `parser`, `scorer`, `memory_store`, `embedder`, `callable_`,
`retriever`, `loader`, `splitter`, `kv_cache`, `long_store`, `tracer`,
`conversation_agent`, `diarizer`, `vad`, `phonemizer`, `journal`,
`repair_model`, `persona_reconciler`, `inference_client`, `ann_index`.

## Async surfaces

Every method that returns an awaitable is exposed via
`pyo3-async-runtimes::tokio::future_into_py`, so it integrates with
any `asyncio` event loop without blocking the Python thread. The
table below is non-exhaustive — every method that does I/O or
crosses the FFI boundary into a tokio-driven Rust operation is
async.

- `Callable.call(input, ctx) -> awaitable[Any]`
- `Pipeline.build()` produces a `Callable`, whose `.call(...)` is
  async.
- `Embedder.embed(text)`, `Embedder.embed_batch(texts)`
- `AnnIndex.upsert(id, vec)`, `AnnIndex.search(query, top_k)`,
  `AnnIndex.len()`
- `Retriever.retrieve(query, ctx)`
- `MemoryStore.put(item)`, `MemoryStore.list(ns, limit)`
- `LongStore.put/get/delete/search/list_namespaces`
- `AgentRef.run_turn(user, budgets)`
- `WorkflowRunner.run(input)`
- `Harness.run()`
- `Conversation.feed(pcm_bytes)`, `Conversation.events()` (async
  iterator)
- `EvalSuite.run(target_callable)`
- `Registry.publish_async(kind, id, version, payload)`
- `RunTreeBuilder.flush_stdout/jsonl/langsmith`
- `EventBus.stream()` (async iterator)
- `LlmCache.get(key)`, `LlmCache.put(key, value)`

Synchronous helpers remain available where appropriate
(`Callable.call_sync(...)`, `Registry.publish(...)` / `.get(...)` /
`.latest(...)` / `.list(...)`, `EventBus.subscribe(callback)`,
`EventBus.emit_*`).

## Async iterators

The standard `__aiter__` / `__anext__` pattern is used for streams:

```python
async for event in bus.stream():
    ...

async for chunk in synthesis_stream:
    ...

async for event in conversation.events():
    ...
```

## Feature flags

The native crate exposes optional features that pull in extra
backends:

| Feature | Pulls in | Affects |
|---|---|---|
| `provider-anthropic` | `atomr-agents-agent/provider-anthropic` | Anthropic pricing/runner re-exports |
| `provider-openai` | `atomr-agents-agent/provider-openai` | OpenAI pricing/runner re-exports |
| `provider-gemini` | `atomr-agents-agent/provider-gemini` | Gemini pricing/runner re-exports |
| `cache-sqlite` | `atomr-agents-cache/sqlite` | `cache.sqlite_llm_cache(path)` |
| `cache-redis` | `atomr-agents-cache/redis` | `cache.redis_llm_cache(url)` |
| `state-sqlite` | `atomr-agents-state/sqlite` | `state.sqlite_checkpointer(path)` |
| `state-postgres` | `atomr-agents-state/postgres` | `state.postgres_checkpointer(dsn)` |
| `stt-whisper-cpp` | `atomr-agents-stt-runtime-whisper/whisper-cpp` | local whisper.cpp backend |
| `stt-mic` | `atomr-agents-stt-audio/mic` | mic capture session |
| `stt-vad-silero` | `atomr-agents-stt-voice/vad-silero` | Silero VAD |
| `stt-diarize-sherpa-onnx` | `atomr-agents-stt-diarize-sherpa/sherpa-onnx` | Sherpa diarization |

Build with `maturin develop --release --features
state-sqlite,cache-sqlite` (etc.) to enable.

## Migration notes

The 0.7 cycle dramatically expanded the binding surface. Anything
that worked in 0.6 still works — every previously exposed symbol is
preserved. New runtime entry points (`AgentBuilder`, `Harness`,
`WorkflowRunner`, `Conversation`, `Retriever`, `IngestPipeline`, …)
are additive.

The 0.2 → 0.3 hierarchical `_native.{...}` move is documented at the
end of this file's earlier revision; that move is still in effect.

## Where to go from here

- [`python.md`](python.md) — host vs guest prose, GIL strategy,
  subinterpreter-pool dispatcher.
- [`agent-pipeline.md`](agent-pipeline.md) — the per-turn pipeline
  `AgentRef.run_turn` drives.
- [`observability.md`](observability.md) — Rust side of `EventBus` /
  `RunTreeBuilder` / tracers.
- [`workflows-and-hitl.md`](workflows-and-hitl.md) — `WorkflowRunner`
  semantics and interrupt API.
- [`retrieval-and-ingestion.md`](retrieval-and-ingestion.md) —
  retriever zoo and `IngestPipeline` shape.
