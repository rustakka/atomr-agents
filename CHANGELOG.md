# Changelog

All notable changes to atomr-agents are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/rustakka/atomr-agents/compare/HEAD
