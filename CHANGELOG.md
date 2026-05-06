# Changelog

All notable changes to atomr-agents are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Catch up to current sibling workspaces.** Path-dep version pins
  bumped to **atomr 0.3.1**, **atomr-infer 0.6.0**, **atomr-accel
  0.3.1**. No source changes were required for the bump itself —
  atomr-agents existing usage of the sibling APIs is forward-compatible.
- **`Event::AgentTurn`** carries two new u32 fields, `reasoning_tokens`
  and `cached_tokens`, sourced from `atomr_infer_core::tokens::TokenUsage`.
  Both are `#[serde(default)]` so existing event JSON deserialises
  unchanged. Surfaces o1-style reasoning-token billing and Anthropic
  prompt-cache / OpenAI cached-input pricing in cost reports.

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

- **Publish gate.** Releasing this version to crates.io requires
  **atomr-infer 0.6.0** and **atomr-accel 0.3.1** to be published
  upstream first; both are currently at 0.4.0 / 0.1.0 on crates.io.
  Local builds and tests work today against the path deps.
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
