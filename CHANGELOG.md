# Changelog

All notable changes to atomr-agents are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — full Python binding parity (Phase 1–5)

The Python surface (`atomr_agents._native` + `python/atomr_agents/`)
now mirrors the entire Rust workspace. Twenty-eight hierarchical
submodules; 24 guest-trait decorators; `AgentBuilder` /
`WorkflowRunner` / `Harness` / `Conversation` runtimes callable
directly from Python.

- **`callable`** (new) — universal `Callable` dyn handle wrapping
  `Arc<dyn Callable>`; `Pipeline` builder; free decorators
  `with_retry`, `with_timeout`, `with_fallbacks`, `with_config`,
  `fan_out`, `branch`, `lambda_`, `passthrough`. Generic Rust
  composition types (`WithRetry<T>`, `Pipeline<…>`) never leak
  their type parameters across the FFI boundary.
- **`strategy`** (new) — value types (`Termination`, `RoutingTarget`,
  `SkillRef`, `ToolRef`, `Policy`, `PolicyDecision`) and dyn
  handles for `MemoryStrategy`, `SkillStrategy`, `ToolStrategy`,
  `RoutingStrategy`, `PolicyStrategy`.
- **`instruction`** (new) — `ChatPromptTemplate`(+Builder),
  `FewShotChatTemplate`, `MessageTemplate`, `MessagesPlaceholder`,
  `StringTemplate`, `Example`, `LengthBasedSelector`,
  `SemanticSimilaritySelector`; `InstructionStrategy` /
  `TaskStrategy` / `BehaviorStrategy` dyn handles; `task_static`,
  `behavior_static`, `*_from_factory`.
- **`memory`** (new — was partial in `core`) — `MemoryStore` /
  `LongStore` dyn handles, `Namespace`, `StoreItem`,
  `in_memory_store`, `in_memory_long_store`,
  `recency_memory_strategy`, `summarizing_memory_strategy`,
  `chained_memory_strategy`.
- **`embed`** (new) — `Embedder` / `AnnIndex` dyn handles;
  `mock_embedder`, `in_memory_ann_index`,
  `embedding_tool_strategy`.
- **`retriever`** (new) — `Document`, `Retriever` dyn handle; 8
  concrete retrievers (`bm25_retriever`, `vector_retriever`,
  `multi_query_retriever`, `contextual_compression_retriever`,
  `parent_document_retriever`, `ensemble_retriever`,
  `self_query_retriever`, `time_weighted_retriever`).
- **`ingest`** (new) — `Loader` / `Splitter` / `KvCache` dyn
  handles, `CodeLang` enum, fluent `IngestPipeline` builder
  (`.loader().splitter().cache().embedder().long_store().build()
  -> Callable`); `text_loader`, `markdown_loader`, `csv_loader`,
  `json_loader`, `recursive_character_splitter`, `token_splitter`,
  `markdown_header_splitter`, `code_splitter`, `semantic_splitter`,
  `in_memory_kv_cache`, `cached_embedder`, `ingest(...)`.
- **`org`** (new) — `Org`, `Department`, `Team` builders;
  `round_robin_router`, `load_aware_router`,
  `capability_match_router`, `namespaced_memory`, `swarm_loop`,
  `ActiveAgent`.
- **`agent`** (expanded) — `AgentBuilder`, `AgentRef` (callable +
  runnable), `InferenceClient` dyn handle with
  `inference_client_from_factory(key, provider)`,
  `AgentMiddleware` handle plus `logging_middleware`,
  `tool_error_recovery_middleware`, `redaction_middleware`,
  `rate_limit_middleware`. Built on the new upstream `BoxedAgent`.
- **`workflow`** (expanded) — `StepId`, `Step` with six
  classmethods (`invoke`/`branch`/`parallel`/`loop_`/`map`/`human`),
  `Dag`/`DagHandle`, `WorkflowRunner`, `WorkflowState`, `Journal`
  dyn handle + `in_memory_journal`, `fan_out_dispatch`.
- **`harness`** (expanded) — `Harness`, `HarnessState`, `StepEvent`,
  `LoopStrategy` / `TerminationStrategy` dyn handles, `iteration_cap`,
  `loop_strategy_from_callable`, `*_from_factory`.
- **`eval`** (expanded) — `RubricCriterion`, `JudgeModel`,
  `EvalCase`/`EvalResult`/`EvalRun`/`EvalSuite`, `Scorer` /
  `PairwiseScorer` / `AnnotationQueue` dyn handles, `rubric_scorer`,
  `llm_judge_scorer`, `pairwise_scorer`,
  `in_memory_annotation_queue`, `regression_gate`.
- **`persona`** (expanded) — `MbtiType`, `Archetype`,
  `CognitiveFunction`, `CognitiveStack` enums; `static_persona_strategy`,
  `mbti_persona_strategy`, `jungian_archetype_strategy`,
  `big_five_persona_strategy`, `composite_persona_strategy`,
  `static_emphasis`, `task_adaptive`, `audience_adaptive`,
  `goal_conditioned`, `mood_state`; `PersonaReconciler` dyn handle.
- **`observability`** (expanded) — `Tracer` dyn handle;
  `jsonl_tracer`, `lang_smith_tracer`, `stdout_tracer`;
  `EventBus.attach_tracer(...)`.
- **`parser`** (expanded) — `Parser` unified dyn handle;
  `output_fixing_parser`, `retry_with_error_parser`, `enum_parser`,
  `schema_parser`; `RepairModel` dyn handle + adapter.
- **`tool`** / **`skill`** (expanded) — `HandoffTool`, `RichTool`,
  `ToolControl`, `ToolReturn`, `PermissionSpec`;
  `static_tool_strategy`, `keyword_tool_strategy`,
  `static_skill_strategy`, `keyword_skill_strategy`;
  `transcribe_tool`, `speak_tool`, `voice_input_skill`,
  `voice_speak_skill`, `voice_response_skill`.
- **`voice`** (expanded) — `ConversationMode`, `ConversationOptions`,
  `ConversationAgent` dyn handle, `Conversation` with push-model
  `feed(pcm_bytes)` + async `events()` iterator,
  `InboundTranscript`, `ConversationEvent`, `noop_agent`.
- **`voice_extras`** (new) — `Diarizer`, `Vad`, `Phonemizer` dyn
  handles + adapters; `mock_diarizer`, `sherpa_diarizer` (feature
  `stt-diarize-sherpa-onnx`), `energy_vad`, `silero_vad` (feature
  `stt-vad-silero`), `mock_phonemizer`.
- **`cache`** (expanded) — `LlmCache` dyn handle;
  `semantic_llm_cache`; feature-gated `sqlite_llm_cache`,
  `redis_llm_cache`.
- **`state`** (expanded) — `Checkpointer` dyn handle;
  `in_memory_checkpointer`; feature-gated `sqlite_checkpointer`,
  `postgres_checkpointer`.
- **`guest`** (expanded) — 16 new `register_X_factory` entry points:
  `register_callable_factory`, `register_retriever_factory`,
  `register_loader_factory`, `register_splitter_factory`,
  `register_kv_cache_factory`, `register_long_store_factory`,
  `register_tracer_factory`,
  `register_conversation_agent_factory`,
  `register_diarizer_factory`, `register_vad_factory`,
  `register_phonemizer_factory`, `register_journal_factory`,
  `register_repair_model_factory`,
  `register_persona_reconciler_factory`,
  `register_inference_client_factory`, `register_ann_index_factory`.
  Each pairs with a `@callable_` / `@retriever` / `@loader` / …
  Python decorator.

### Added — `BoxedAgent` and `Box<dyn>` blanket impls

The agent runtime is now constructible from `Box<dyn …>` strategy
slots:

- `atomr-agents-agent::BoxedAgent` wraps `Agent<Box<dyn
  InstructionStrategy>, Box<dyn ToolStrategy>, Box<dyn
  MemoryStrategy>, Box<dyn SkillStrategy>>` with an `into_ref()`
  helper that produces an `AgentRef` (which already implements
  `Callable`).
- `atomr-agents-instruction::InstructionStrategy` gains a blanket
  `impl InstructionStrategy for Box<dyn InstructionStrategy>`.
- `atomr-agents-strategy` gains the same blanket impls for
  `MemoryStrategy`, `SkillStrategy`, `ToolStrategy`.
- `atomr-agents-harness` gains the same blanket impls for
  `LoopStrategy` and `TerminationStrategy`.

These are additive — every existing monomorphic call site continues
to work.

### Added — Cargo features on `atomr-agents-py-bindings`

`provider-anthropic` / `provider-openai` / `provider-gemini`
(forwarders to the agent crate); `cache-sqlite`, `cache-redis`,
`state-sqlite`, `state-postgres`; `stt-diarize-sherpa-onnx` (in
addition to existing `stt-whisper-cpp`, `stt-mic`,
`stt-vad-silero`).

## [0.6.0] — 2026-05-08

### Added — text-to-speech capability

The mirror of the v0.5.0 STT capability: a new TTS surface with
cloud + local backends, a unified `Capabilities` struct that
advertises the five MOSS-TTS surfaces (plain TTS, voicegen-from-text,
voice cloning, multispeaker dialogue, sound effects, plus
realtime-bidirectional and streaming output), and an agent-framework
adapter pair (`SpeakTool`, `voice_response_skill`).

Eleven new crates under `crates/tts-*/` plus PyO3 bindings at
`crates/py-bindings/src/tts.rs` and a Python facade at
`python/atomr_agents/tts.py`. Each runner exports a `pub const CAPS`
so callers can compare backends before wiring one in.

- **`atomr-agents-tts-core`** — `TextToSpeech`, `SynthesisStream`,
  `RealtimeSession` traits; `SynthesisRequest::{Tts, SoundEffect,
  Dialogue}`; `VoiceRef::{Library, DescribedAs, ClonedFrom, Custom}`;
  `Capabilities` with the five MOSS-TTS surface flags; `AudioOutput`
  (PCM or container bytes); `RealtimeEvent` covering audio frames,
  inbound transcripts, outbound text, speech-start/end, barge-in,
  and done; `MockTextToSpeech` for tests.
- **`atomr-agents-tts-audio`** — output-side audio: WAV writer
  (`encode-wav`, default-on), `cpal`-backed `SpeakerStream`
  (`speaker`, default-off), and pump helpers.
- **`atomr-agents-tts-runtime-openai`** — `POST /v1/audio/speech`
  batch + chunked streaming for `tts-1`, `tts-1-hd`,
  `gpt-4o-mini-tts`. Eleven preset voices.
- **`atomr-agents-tts-runtime-elevenlabs`** — REST batch +
  `POST .../stream` chunked stream + `POST /v1/sound-generation`
  SFX + Conversational AI WS realtime. Voice cloning (60s reference
  clip), 30 supported languages, dynamic voice catalog.
- **`atomr-agents-tts-runtime-openai-realtime`** — `wss://
  api.openai.com/v1/realtime` bidirectional voice agent. Maps
  audio.delta / audio_transcript.delta / speech_started/_stopped /
  input_audio_transcription.completed events onto the unified
  `RealtimeEvent` shape.
- **`atomr-agents-tts-runtime-gemini-live`** — `wss://
  generativelanguage.googleapis.com/.../BidiGenerateContent` voice
  agent with the same trait shape. Setup picks voice (Puck / Charon /
  Kore / Fenrir / Aoede), modalities, and a system instruction.
- **`atomr-agents-tts-runtime-piper`** — Piper local TTS
  (~50 MB ONNX models, ~30 languages, RTF ~0.05). Ships the trait
  surface + CAPS today; the ORT pipeline lands behind the
  `piper-ort` feature once `atomr-infer-runtime-ort` matures
  (mirrors the `stt-diarize-sherpa` skeleton-with-feature-gate
  pattern).
- **`atomr-agents-tts-runtime-kokoro`** — Kokoro-82M local TTS
  (English, 21 preset voices). Same skeleton-with-feature-gate
  pattern (`kokoro-ort`).
- **`atomr-agents-tts-runtime-moss`** — MOSS-TTS local backend
  covering all five surfaces (Delay-8B / Local-1.7B / TTSD /
  VoiceGenerator / SoundEffect / Realtime variants). Talks to a
  colocated Python server (SGLang or FastAPI wrapper) over HTTP;
  feature `moss-http` enables the client.
- **`atomr-agents-tts-runtime-xtts`** — Coqui XTTS v2 zero-shot
  voice cloning (6s reference, 17 languages). Same Python-server
  pattern as MOSS (`xtts-http` feature).
- **`atomr-agents-tts-voice`** — `Conversation` session with
  `TurnBased` (caller drives transcript in, gets synthesised reply
  out) and `UnifiedRealtime` (one realtime backend serves both
  directions) modes. `ConversationAgent` trait + `NoopAgent` for
  testing. `ConversationEvent::{UserSpoke, AssistantText,
  AssistantAudio, Interrupted, Done}`.
- **`atomr-agents-tts-tool`** — `SpeakTool` (a framework `Tool` the
  model can call to render a WAV file), `voice_response_skill(stt,
  tts)` (bundles `SpeakTool` + `TranscribeTool` into a Skill), and
  `voice_speak_skill(tts)` (speak-only).

### Python bindings

`python/atomr_agents/tts.py` re-exports the TTS submodule.
Backend constructors (in addition to `mock_tts`):
`tts_openai`, `tts_elevenlabs`, `tts_openai_realtime`,
`tts_gemini_live`, `tts_piper`, `tts_kokoro`, `tts_moss`,
`tts_xtts`. Each accepts the standard `api_key="env:VARNAME"` /
`"file:/path"` / literal forms used by the STT constructors.

### Umbrella feature flags

`atomr-agents` umbrella adds: `tts`, `tts-{openai, elevenlabs,
openai-realtime, gemini-live, piper, kokoro, moss, xtts}`,
`tts-speaker`, `tts-voice`, `tts-full`, and a top-level
`conversation` (= `stt-full + tts-full + tts-voice + stt-voice`)
feature for live voice-agent setups.

### Wire-format fix

`Languages::Subset(...)` now serializes via a custom `Serialize`
impl as `{"kind": "subset", "codes": [...]}`; the previous
internally-tagged form failed at runtime when serializing to JSON
through the Python bridge (serde rejects sequence payloads inside
internally-tagged tuple variants).

## [0.5.0] — 2026-05-08

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
