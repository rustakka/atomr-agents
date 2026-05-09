# Changelog

All notable changes to atomr-agents are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — speech-to-text capability

A new capability surface for ingesting audio into agent workflows.
Lives across ten new crates under `crates/stt-*/` plus PyO3 wrappers
at `crates/py-bindings/src/{stt,voice}.rs` and Python facades at
`python/atomr_agents/{stt,voice}.py`. All backends advertise their
abilities via a rich `Capabilities` struct (a `pub const` on each
runner) so callers can introspect support for streaming, diarization,
languages, word timestamps, max audio length, and per-minute cost
before wiring a backend into an agent.

- **`atomr-agents-stt-core`** — `SpeechToText` and `StreamingSession`
  traits, `Capabilities` (serializable to JSON for Python
  introspection), `AudioInput` (file / bytes / PCM), `Transcript`
  with per-word timing and speaker tags, `BackendKind` /
  `TransportKind` enums, and `MockSpeechToText` for tests.
- **`atomr-agents-stt-remote-core`** — shared HTTP / WebSocket
  plumbing: `reqwest::Client` builder, `tokio-tungstenite` connect
  helper, `SecretRef` (env / literal / file), `RetryPolicy`,
  `Timeouts`, `RateLimits`, exponential-backoff `retry()` helper.
- **`atomr-agents-stt-audio`** — `symphonia`-based decoder,
  `rubato` resampler, `cpal`-based microphone capture (feature
  `mic`, default-off because `cpal` requires platform audio dev
  headers).
- **`atomr-agents-stt-runtime-openai`** — Whisper / `gpt-4o-transcribe`
  REST batch.
- **`atomr-agents-stt-runtime-deepgram`** — REST batch + live
  WebSocket; diarization (speaker count); partial results;
  VAD-based endpointing.
- **`atomr-agents-stt-runtime-assemblyai`** — REST upload + poll +
  Universal-Streaming WebSocket; named-speaker diarization.
- **`atomr-agents-stt-runtime-whisper`** — local whisper.cpp via
  `whisper-rs`. The actual binding is gated behind the
  `whisper-cpp` feature so plain workspace builds don't need a
  C++ toolchain. Without the feature, `transcribe` returns a
  typed `SttError::ModelLoad` naming the missing flag. Optional
  `download-models` helper fetches ggml weights into the OS
  cache dir.
- **`atomr-agents-stt-diarize-sherpa`** — `Diarizer` trait,
  always-available `MockDiarizer`, and a `SherpaDiarizer` stub
  ready for the upstream `sherpa-onnx` Rust binding to be
  wired in. `apply_to_transcript` stitches diarization spans
  into segment speaker tags.
- **`atomr-agents-stt-voice`** — `VoiceSession` with `Live` vs
  `TurnBased { silence_ms }` modes layered on top of any
  `StreamingSession`. Includes a `Vad` trait, an always-on
  `EnergyVad`, and (via the `vad-silero` feature) a `SileroVad`
  using the `voice_activity_detector` crate. The `mic` feature
  enables a `pump_mic_to_stream` helper that wires
  `MicCaptureSession` → `StreamingSession::push_audio`.
- **`atomr-agents-stt-tool`** — `TranscribeTool` (a `Tool` the
  model can elect to call) and `voice_input_skill(stt) -> (Skill,
  DynTool)` for declarative agent integration.
- **Umbrella crate** gains `stt`, `stt-{openai,deepgram,assemblyai,
  whisper,diarize,voice,mic}`, and `stt-full` features that
  re-export the relevant backend modules under
  `atomr_agents::stt`.
- **Python bindings** — `atomr_agents._native.stt` exposes
  `SpeechToText`, `Capabilities`, `Transcript`, `StreamingSession`,
  `StreamEvent`, `audio_{file,bytes,pcm}`, plus backend
  constructors `mock_speech_to_text()`, `stt_openai()`,
  `stt_deepgram()`, `stt_assemblyai()`, `stt_whisper()`. The
  `voice` submodule exposes `VoiceMode`, `VoiceEvent`, and
  `VoiceSession`. Async iterators follow the same
  `__aiter__`/`__anext__` pattern as `EventStream` in
  `observability`. Python-level facades at
  `python/atomr_agents/{stt,voice}.py`.

### Changed — track upstream atomr 0.6.0 + atomr-accel 0.4.0 + atomr-infer 0.7.0

- **Catch up to current sibling workspaces.** Path-dep version pins
  bumped to **atomr 0.6.0**, **atomr-infer 0.7.0**, **atomr-accel
  0.4.0**. No source changes were required for the bump itself —
  atomr-agents' existing usage of the sibling APIs (`ExecuteBatch`,
  `Message`, `MessageContent`, `Role`, `SamplingParams`, `TokenUsage`,
  `FinishReason`, `RunHandle`, `RuntimeKind`, `TransportKind`,
  `ModelRunner`, plus `atomr_core`'s `Event` / `ActorRef` /
  `ActorSystem`) is forward-compatible. `cargo check --workspace`
  and `cargo test --workspace` are clean against the new pins.
- **`Event::AgentTurn`** carries two new u32 fields, `reasoning_tokens`
  and `cached_tokens`, sourced from `atomr_infer_core::tokens::TokenUsage`.
  Both are `#[serde(default)]` so existing event JSON deserialises
  unchanged. Surfaces o1-style reasoning-token billing and Anthropic
  prompt-cache / OpenAI cached-input pricing in cost reports.

### Added — Python parity wave

The PyO3 surface is restructured from a 260-line monolithic `lib.rs`
into hierarchical `atomr_agents._native.{...}` submodules, mirroring
the upstream `atomr-infer/inference-py-bindings/src/` layout. The
top-level Python facade re-exports the new surface, ships a `py.typed`
PEP 561 marker, and wires the previously-stub `@tool` / `@strategy` /
`@persona` decorators through real `_native.guest.register_*_factory`
factories. Async coroutines and async iterators are exposed via
`pyo3-async-runtimes::tokio::future_into_py` for the surfaces that
already have a non-generic Rust adapter.

- **`atomr_agents._native.errors`** — exception hierarchy rooted at
  `AgentError`, with leaf classes `RegistryError`, `BudgetExhausted`,
  `ToolError`, `StrategyError`, `WorkflowError`, `HarnessError`,
  `EvalError`, `MemoryError`, `ParserError`, `CacheError`. Translates
  Rust `Result<T, AgentError>` into structured Python exceptions
  instead of opaque `RuntimeError`.
- **`atomr_agents._native.core`** — IDs (`AgentId`, `TeamId`,
  `DepartmentId`, `OrgId`, `WorkflowId`, `HarnessId`, `ToolId`,
  `ToolSetId`, `SkillId`, `PersonaId`, `RunId`); budgets
  (`TokenBudget`, `TimeBudget`, `MoneyBudget`, `IterationBudget`);
  `MemoryNamespace` (Agent / Team / Org variants); string-tagged
  `MemoryKind`; `MemoryItem` and `MemoryChunk`; `TokenUsage`;
  `FinishReason`.
- **`atomr_agents._native.context`** — `ContextFragment`,
  `RenderedContext`, free `assemble(fragments, budget)` function for
  priority-based bin-packing under a `TokenBudget`.
- **`atomr_agents._native.state`** — `CheckpointKey`,
  `CheckpointMeta`, `Snapshot`, `InMemoryCheckpointer`, plus reducer
  marker classes (`LastWriteWins`, `AppendList`, `AppendMessages`,
  `MergeMap`, `MaxByTimestamp`).
- **`atomr_agents._native.observability`** — `Event`, `EventBus`,
  `EventStream` (async iterator via `__aiter__` / `__anext__`),
  `RunTreeBuilder` with async `flush_stdout` / `flush_jsonl` /
  `flush_langsmith`.
- **`atomr_agents._native.registry`** — `Registry` with sync
  `publish` / `get` / `latest` / `list` plus async `publish_async`;
  string-tagged `ArtifactKind`; `ArtifactRecord`; `EvalSummary`.
- **`atomr_agents._native.tool`** — `ToolSchema`, `ToolDescriptor`,
  string-tagged `Provider`, `ParsedToolCall`, stateful streaming
  `ToolCallParser` (`feed` / `finish`), `ToolSet`.
- **`atomr_agents._native.skill`** — `Skill`, `SkillSet`.
- **`atomr_agents._native.persona`** — `RenderedPersona`.
- **`atomr_agents._native.parser`** — `JsonParser`,
  `JsonSchemaParser`, `CommaListParser`, `XmlParser`, `YamlParser`,
  `StreamingPartialJsonParser`. All async `parse(raw)` methods bridge
  through `future_into_py`.
- **`atomr_agents._native.cache`** — `CacheKey`, `CachedTurn`,
  `InMemoryLlmCache` (async `get` / `put`).
- **`atomr_agents._native.agent`** — `AgentSpec`, `AgentBudgets`,
  `TurnResult`.
- **`atomr_agents._native.workflow`** — string-tagged `StepKind`.
- **`atomr_agents._native.harness`** — `HarnessSpec`,
  `IterationCapTermination`.
- **`atomr_agents._native.eval`** — `PairwiseChoice`, `Verdict`.
- **`atomr_agents._native.guest`** — `GuestHandle` plus
  `register_*_factory` entry points for Python-implementable Rust
  traits, a working `PyToolAdapter` that dispatches Python `invoke`
  methods (sync or async) under the GIL, and `build_guest_toolset` to
  turn registered guest tools into a Rust `ToolSet`. The `@tool` /
  `@strategy` / `@persona` / `@skill` / `@parser` / `@scorer` /
  `@memory_store` / `@embedder` decorators in `atomr_agents.guest`
  now bind to these factories rather than acting as no-op markers.
- **Async surfaces today.** `pyo3-async-runtimes::tokio::future_into_py`
  is plumbed through `Registry.publish_async`,
  `RunTreeBuilder.flush_stdout` / `flush_jsonl` / `flush_langsmith`,
  `EventBus.stream() -> EventStream` (async iterator), every
  `parser.*.parse`, and `InMemoryLlmCache.get` / `put`. Awaitable
  from any `asyncio` event loop without blocking the Python thread.
- **Top-level Python facade.** `python/atomr_agents/__init__.py`
  re-exports the full submodule surface and resolves `__version__`
  via `importlib.metadata`. One facade `.py` per submodule (`core.py`,
  `errors.py`, `observability.py`, `registry.py`, `tool.py`,
  `skill.py`, `persona.py`, `agent.py`, `workflow.py`, `harness.py`,
  `eval.py`) means `from atomr_agents.errors import RegistryError`
  and `from atomr_agents.agent import AgentSpec` resolve without
  dipping into `_native.*`. `host.py` exposes the full host-mode
  entry points; `guest.py` carries the real decorator set wired to
  `_native.guest.register_*_factory`. `py.typed` ships a PEP 561
  marker so type checkers pick up the public surface.
- **`pyproject.toml`** updated with classifiers, `[project.urls]`,
  `[project.optional-dependencies] dev = [...]`,
  `[tool.pytest.ini_options]`, and `py.typed` packaging.
- **`python/atomr_agents/tests/test_smoke.py`** exercises the
  hierarchical imports, budget construction, tool descriptors, the
  async registry path, and a guest-factory round-trip end-to-end.

### Added

- **Native aarch64-Linux CI coverage.** `release.yml` builds the
  `atomr-agents` CLI binary AND the Python wheel on
  `ubuntu-22.04-arm` runners (in addition to the existing x86_64 +
  macOS + Windows targets). Drops the brittle `gcc-aarch64-linux-gnu`
  cross-compile step in favor of native ARM builds. PyPI wheel
  coverage after this lands: `linux-gnu x86_64`, `linux-gnu aarch64`
  (new), `macOS x86_64`, `macOS aarch64`, `windows x86_64`.
- **`Event::ToolCallStreamed`** — new variant emitted per detected tool
  call before dispatch (distinct from `ToolInvoked`, which fires after).
  Lets tracers and UIs surface tool intent in real time. The agent
  pipeline also now returns the full `Vec<ParsedToolCall>` aggregated
  across iterations in `TurnResult.tool_calls` (was always empty).
- **`atomr-infer-testkit::MockRunner`** adopted as a dev-dependency on
  `atomr-agents-agent`. Replaces the vendored `InlineTextMock` shim
  removed in `de34c38` (testkit is now published as part of atomr-infer
  v0.6.0).
- **`atomr_agents_memory::query`** — re-export module surfacing
  `atomr-persistence-query`'s `ReadJournal`, `EventEnvelope`, `Offset`,
  and `SimpleReadJournal`. Wrap any `Journal` with `SimpleReadJournal`
  to get `events_by_tag` / `events_by_persistence_id` /
  `all_persistence_ids` for lineage and replay.
- **Provider runtime back-ends as opt-in features.** New `agent` crate
  features `provider-anthropic`, `provider-openai`, `provider-gemini`
  pull in `atomr-infer-runtime-{anthropic,openai,gemini}` and re-export
  the `*Config` / `*Pricing` / `*Runner` types via
  `atomr_agents_agent::providers::{anthropic,openai,gemini}`. Forwarded
  through the umbrella crate as `atomr-agents/{provider-anthropic,
  provider-openai, provider-gemini}`.

### Deferred / known gaps

- **`Agent.run_turn` / `Harness.run` / `WorkflowRunner.run` async
  surfaces.** Rust-side `Agent<I, T, Ms, Sk>`, `Harness<L, T>`, and
  `WorkflowRunner<...>` are all generic over four-plus strategy
  traits, so PyO3 cannot construct them directly from a stable
  `#[pyclass]` shape. Landing the async Python entry points needs a
  `Boxed*` Rust adapter (`BoxedAgent`, `BoxedHarness`,
  `BoxedWorkflow`) under `crates/agent`, `crates/harness`, and
  `crates/workflow` — that adapter does not exist yet. Until it
  does, host code drives the loop in Rust and observes via
  `EventBus` (which is already async-iterable from Python).
- **Publish gate.** Releasing this version to crates.io requires
  **atomr 0.6.0**, **atomr-infer 0.7.0**, and **atomr-accel 0.4.0**
  to be published upstream first. Local builds and tests work today
  against the path deps.
- **MSRV regression (pre-existing).** Transitive `clap_lex 1.1.0`
  requires `edition2024`, breaking `cargo +1.78 check`. Independent
  of the sibling bumps; either pin clap below 4.6 or raise the
  workspace `rust-version`.
- **Phase 3 candidates not adopted yet:** `atomr-cluster-metrics`
  adaptive routing in `org`, `atomr-infer-pipeline` strategy alignment,
  `atomr-accel-agents::CpuVectorIndex` adoption in `embed`, atomr-streams
  new operators (`keep_alive`, `merge_prioritized`, `recover_with_retries`,
  `conflate`, `expand`), `FsmBuilder` rewrite of workflow,
  `TelemetryBus::subscribe_topic`, `expect_msg_*` matchers in testkit.

## [0.1.0] — initial release

### Added

- **Initial public release.** A composable agentic framework on
  [atomr](https://github.com/rustakka/atomr) and
  [atomr-infer](https://github.com/rustakka/atomr-infer).
- **`atomr-agents-callable`** — `Callable` trait, `Pipeline`
  builder (`then` / `fan_out_with` / `assign`), decorators
  (`with_retry` / `with_fallbacks` / `with_config` / `with_timeout`
  / `Branch` / `Lambda`).
- **`atomr-agents-strategy`** — strategy trait family
  (`InstructionStrategy`, `ToolStrategy`, `MemoryStrategy`,
  `SkillStrategy`, `RoutingStrategy`, `PolicyStrategy`,
  `LoopStrategy`, `TerminationStrategy`).
- **`atomr-agents-state`** — `StateSchema`, five reducers
  (`AppendList`, `AppendMessages`, `MergeMap`, `LastWriteWins`,
  `MaxByTimestamp`), `RunState`, `Checkpointer` trait,
  `InMemoryCheckpointer` with fork-with-edit.
- **`atomr-agents-tool`** — `Tool`/`RichTool`, `ToolDescriptor`,
  `ToolSet`, `ToolSetRegistry`, provider-aware `ToolCallParser`
  (OpenAI / Anthropic), `HandoffTool`, `Provider` enum.
- **`atomr-agents-skill`** — `Skill`, `SkillSet`, `Static` /
  `Keyword` skill strategies.
- **`atomr-agents-memory`** — `MemoryStore` (short-term),
  `LongStore` (long-term, namespace-tupled, embedding-indexed),
  `WriteMemoryTool` / `UpdateMemoryTool` / `RecallMemoryTool`,
  `RecencyMemoryStrategy` / `SummarizingMemoryStrategy`.
- **`atomr-agents-embed`** — `Embedder` trait, `MockEmbedder`,
  `AnnIndex` + `InMemoryAnnIndex`, `EmbeddingToolStrategy`.
- **`atomr-agents-retriever`** — Retriever zoo: BM25, Vector,
  MultiQuery, ContextualCompression, ParentDocument, Ensemble
  (RRF), SelfQuery, EmbeddingsFilter, TimeWeighted.
- **`atomr-agents-ingest`** — Loaders (text / md / json / csv),
  splitters (Recursive / MarkdownHeader / Code / Token /
  Semantic), `CachedEmbedder`, `IngestPipeline`, `ingest()`.
- **`atomr-agents-persona`** — All five structural persona
  strategies (Static, Big Five, MBTI, Jungian, Composite) and
  five emphasis strategies (Static, AudienceAdaptive, TaskAdaptive,
  MoodState, GoalConditioned).
- **`atomr-agents-instruction`** — `ComposedInstructionStrategy<P,
  T, B>`, `ChatPromptTemplate`, `MessagesPlaceholder`,
  `FewShotChatTemplate`, `LengthBasedSelector` /
  `SemanticSimilaritySelector`.
- **`atomr-agents-agent`** — `Agent<I, T, Ms, Sk>` with per-turn
  pipeline + parallel tool-call dispatch via `tokio::JoinSet`,
  `InferenceClient` / `LocalRunnerClient` adapter for any atomr-infer
  `ModelRunner`, `AgentMiddleware` (logging / retry / rate-limit /
  redaction / tool-error-recovery).
- **`atomr-agents-workflow`** — DAG primitives, `WorkflowRunner`
  (legacy), `StatefulRunner` (channelled state), `Interruptible`
  with dynamic `interrupt()` + static breakpoints + `Command`
  resume API, `Subgraph`, `dispatch_fan_out` (Send-API analogue).
- **`atomr-agents-harness`** — `Harness<L, T>`, `LoopStrategy`,
  `TerminationStrategy`, durable iteration log; `Harness` is itself
  a `Callable`.
- **`atomr-agents-org`** — `Org` / `Department` / `Team`,
  `OrgRoutingStrategy` impls (`RoundRobin`, `LoadAware`,
  `CapabilityMatch`), `Policy::narrow`, `NamespacedMemory`,
  `swarm_loop`.
- **`atomr-agents-registry`** — versioned artifact registry,
  `publish_gated` for eval-regression blocking.
- **`atomr-agents-eval`** — `EvalSuite`, `Scorer` impls (`Contains`,
  `LlmJudgeScorer`, `RubricScorer`, `PairwiseScorer`),
  `RegressionGate`, `AnnotationQueue`.
- **`atomr-agents-cache`** — `LlmCache` trait, `InMemoryLlmCache`,
  `SemanticLlmCache` with cosine match.
- **`atomr-agents-parser`** — `Parser<T>` trait, `JsonParser` /
  `JsonSchemaParser` / `SchemaParser<T>` / `EnumParser` /
  `CommaListParser` / `XmlParser` / `YamlParser`,
  `OutputFixingParser`, `RetryWithErrorParser`,
  `StreamingPartialJsonParser`.
- **`atomr-agents-observability`** — `EventBus` with `RunId` /
  `parent_run_id`, `RunTreeBuilder`, `Tracer` trait,
  `StdoutTracer` / `JsonlTracer` / `LangSmithTracer`.
- **`atomr-agents-py-bindings`** — `atomr_agents._native` PyO3 module
  exposing `Event` / `EventBus` / `Registry` to Python.
- **`atomr-agents-cli`** — `atomr-agents` binary with `eval` /
  `registry` / `harness` / `serve` (Studio-style read+resume
  inspector) subcommands.
- **`atomr-agents-testkit`** — `MockInference`, deterministic
  strategies, in-memory stores.
- **Backend feature stubs** — `sqlite` / `postgres` for
  `Checkpointer`; `pgvector` / `qdrant` / `chroma` for `LongStore`;
  `sqlite` / `redis` for `LlmCache`. Trait surface is in place;
  real wiring lives in deployment patches.
- **AI skills** — 12 `SKILL.md` files under `ai-skills/skills/` for
  Claude Code / Agent SDK consumers.
- **Documentation** — full docs hub at `docs/index.md` plus
  architecture, state, agent pipeline, workflows + HITL, retrieval
  + ingestion, observability, eval, multi-agent patterns, feature
  matrix, Python bindings, and a LangGraph migration guide.

[Unreleased]: https://github.com/rustakka/atomr-agents/compare/v0.1.4...HEAD
[0.1.0]: https://github.com/rustakka/atomr-agents/releases/tag/v0.1.0
