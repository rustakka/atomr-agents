# Deep Research Harness — Implementation Plan (historical)

> Status: **landed**. The harness, web companion, and supporting crates
> are merged. See [`docs/deep-research-harness.md`](deep-research-harness.md)
> for the up-to-date user-facing design + usage doc. This file is
> preserved as the historical plan that drove the initial implementation.

## Context

atomr-agents already has two production harnesses (`stt-harness`, `meetings-harness`) built around a uniform shape: `Spec → Harness<L, T> → BoxedX → XRef (Callable)` with `LoopStrategy + TerminationStrategy` traits and a pluggable domain trait (`MeetingExtractor`). What it does **not** have is a harness for *deep research* — multi-step, multi-source, citation-bearing investigations over a user query.

This plan adds **`crates/deep-research-harness`** plus a small set of supporting crates so the framework can run several well-known deep-research topologies behind a single uniform input/output contract. The topology is selected via the strategy pattern, so a caller can swap NVIDIA AI-Q's `clarify → plan → search → write → verify` flow for Anthropic's `lead + parallel subagents` flow without changing the request/response shape or the surrounding plumbing.

**Goals:**
1. One `ResearchRequest` / `ResearchResult` contract spanning every topology.
2. Three v1 topologies modelled after NVIDIA AI-Q, Anthropic's multi-agent research system, and LangGraph's `open_deep_research`.
3. Reuse the existing harness conventions exactly — no inheritance, no new framework concepts. Match the `meetings-harness` pattern of a pluggable domain trait + a `ToolHandle` that mutates the in-flight result + `tokio::broadcast` domain events on top of the shared `EventBus`.
4. A new general-purpose **web-search tool layer** in `atomr-agents` (not nested inside the harness crate) so future harnesses and agents can reuse it. Pluggable providers + a deterministic mock default. Local-corpus search continues to flow through the existing `atomr-agents-retriever` crate.
5. Deterministic, LLM-free default sub-agent impls so the crate's tests, the web UI, and ingest pipelines exercise end-to-end without a model provider — matching the `RuleBasedExtractor` pattern.
6. A `deep-research-harness-web` Axum + embedded-UI companion crate for kicking off and browsing runs.

**Out of scope for v1** (called out so the trait surface accommodates them without locking them in):
- `PlanAndExecute`, `LinearWriteCritique`, `OutlineFirstSectionFanout` strategies — slot ready, deferred to v2.
- Two-tier outer shell (intent classifier routing between shallow and deep). Routed by callers; the harness itself is always "deep".
- Concrete web-search provider crates (Tavily, SerpAPI, etc.). Trait + mock land in v1; provider crates follow.

---

## Architecture overview

```
                              ┌──────────────────────────────────────────┐
                              │   DeepResearchHarness<L, T>              │
                              │   (Spec → Typed → Boxed → Ref pattern)   │
                              └──────────────────────────────────────────┘
                                              │
                                              ▼
                  ┌──────────────────────────────────────────────────┐
                  │  L: DeepResearchLoopStrategy  (the topology)     │
                  │  ──────────────────────────────────────────────  │
                  │   • ClarifyPlanSearchVerifyLoop  (AI-Q)          │
                  │   • MultiAgentParallelLoop      (Anthropic)      │
                  │   • IterativeDeepeningLoop      (LangGraph ODR)  │
                  └──────────────────────────────────────────────────┘
                                              │
                       ┌──────────────────────┼──────────────────────┐
                       ▼                      ▼                      ▼
                ┌────────────┐         ┌────────────┐          ┌──────────────┐
                │ Clarifier  │         │  Planner   │          │  Researcher  │
                └────────────┘         └────────────┘          └──────────────┘
                                              │
                       ┌──────────────────────┴──────────────────────┐
                       ▼                                             ▼
                ┌────────────┐         ┌────────────┐          ┌──────────────────┐
                │   Writer   │         │   Critic   │          │ CitationVerifier │
                └────────────┘         └────────────┘          └──────────────────┘
                                              │
                                              ▼
                              ┌──────────────────────────────────┐
                              │   ResearchHandle (shared, Arc)   │
                              │   Mutates ResearchResult via     │
                              │   typed tools.                   │
                              └──────────────────────────────────┘
```

Conceptually identical to the meetings harness: a loop strategy drives one iteration, which calls into pluggable role traits, which mutate a typed result through a tool handle.

---

## New / changed crates

| Crate                                    | Purpose                                                                                          | New? |
|------------------------------------------|--------------------------------------------------------------------------------------------------|------|
| `crates/web-search-core`                 | Provider-agnostic `WebSearch` trait + `WebSearchRequest`/`WebSearchHit` types. Mock default.     | new  |
| `crates/web-search-tool`                 | `WebSearchTool` implementing `atomr_agents_tool::Tool` over any `WebSearch` provider.            | new  |
| `crates/deep-research-core`              | Shared types: `ResearchRequest`, `ResearchResult`, `Citation`, `Plan`, `SubQuestion`, telemetry. | new  |
| `crates/deep-research-harness`           | The harness crate (spec, harness, loop strategies, role traits, tools, events, store).           | new  |
| `crates/deep-research-harness-web`       | Axum + embedded UI companion (mirrors `stt-harness-web`, `meetings-harness-web`).                | new  |

All five are added to the workspace `members` list in `Cargo.toml` and given `[workspace.dependencies]` entries at the existing version (`0.10.0`).

**Why split web-search out of the harness crate**: the search abstraction should be reusable. Keeping it in `crates/web-search-core` lets agents, workflows, and future harnesses pull it in without depending on `deep-research-harness`.

**Why split research-core out of the harness crate**: same reasoning as `stt-core` / `stt-harness`. The data types (request, result, citation) should be usable by callers that want the contract without the runtime — e.g. a CLI that produces a `ResearchRequest` and a UI that renders a `ResearchResult`.

---

## Uniform contract (in `crates/deep-research-core`)

```rust
/// Uniform input across every topology.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResearchRequest {
    pub query: String,
    pub clarifications: Vec<ClarificationTurn>,   // empty on first call
    pub scope: ResearchScope,
    pub depth: u32,             // max planner/critic loops
    pub breadth: u32,           // max parallel sub-questions per round
    pub time_budget: Option<Duration>,
    pub token_budget: Option<TokenBudget>,
    pub tools_allowlist: Vec<String>,
    pub output_format: OutputFormat,           // Markdown { template: Option<String> }
    pub llm_overrides: LlmOverrides,           // per-role model ids
    pub human_in_the_loop: HitlPolicy,         // {AutoClarify, AskOnce, AskEveryRound, Off}
}

#[derive(Clone, Debug)]
pub struct ResearchScope {
    pub data_sources: Vec<DataSourceRef>,       // retriever ids, corpus ids
    pub allowed_domains: Vec<String>,           // web-search filters
    pub attachments: Vec<AttachmentRef>,
}

/// Uniform output across every topology.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResearchResult {
    pub id: String,
    pub strategy: String,                     // "clarify-plan-search-verify" | ...
    pub state: ResearchState,                 // Pending | Clarifying | Running | Verifying | Done | Failed
    pub final_report: Option<Markdown>,
    pub citations: Vec<Citation>,             // numbered, deduped
    pub plan: Option<Plan>,                   // outline + sub_questions + rationale
    pub transcript: Vec<NodeStep>,            // per-node audit (role, label, ts, summary)
    pub coverage: CoverageSignals,            // sub_questions_answered, unresolved_gaps, confidence_per_section
    pub telemetry: Telemetry,                 // tokens, tool_calls, wall_clock, cost, per_node
    pub artifacts: Artifacts,                 // drafts, scratchpad, raw_search_hits
}
```

`ResearchResult` is the analog of `MeetingAnalysis` — typed state, persisted via a store, mutated through a `ResearchHandle`, snapshot-able at any time so the web UI can stream progress.

Every strategy must populate `plan`, `citations`, `transcript`, and `telemetry`. Coverage signals are optional but recommended. Final report is required on `Done`.

---

## Trait surface (in `crates/deep-research-harness`)

Mirrors `MeetingExtractor` / `MeetingsLoopStrategy` / `MeetingsStepCtx` from `crates/meetings-harness/src/loop_strategy.rs:39`.

```rust
// ---- The loop strategy (= the topology). One impl per topology. ----
#[async_trait]
pub trait DeepResearchLoopStrategy: Send + Sync + 'static {
    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome>;
    /// Strategy identifier recorded in `ResearchResult::strategy`.
    fn name(&self) -> &str;
}

pub struct DeepResearchStepCtx<'a> {
    pub state:      &'a mut DeepResearchState,
    pub handle:     &'a ResearchHandle,
    pub store:      Arc<dyn ResearchStore>,
    pub clarifier:  &'a dyn Clarifier,
    pub planner:    &'a dyn Planner,
    pub researcher: &'a dyn Researcher,
    pub writer:     &'a dyn Writer,
    pub critic:     &'a dyn Critic,
    pub verifier:   &'a dyn CitationVerifier,
    pub events:     &'a broadcast::Sender<DeepResearchEvent>,
    pub bus:        &'a EventBus,
}

pub enum DeepResearchStepOutcome { Continue { label: String }, Done { label: String } }

// ---- Role traits. All pluggable; default impls are deterministic and LLM-free. ----
#[async_trait] pub trait Clarifier:        Send + Sync + 'static { async fn clarify(&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<ClarifyOutcome>; }
#[async_trait] pub trait Planner:          Send + Sync + 'static { async fn plan    (&self, req: &ResearchRequest, handle: &ResearchHandle) -> Result<Plan>; }
#[async_trait] pub trait Researcher:       Send + Sync + 'static { async fn research(&self, sub: &SubQuestion,     handle: &ResearchHandle) -> Result<()>; }
#[async_trait] pub trait Writer:           Send + Sync + 'static { async fn write   (&self, plan: &Plan,           handle: &ResearchHandle) -> Result<()>; }
#[async_trait] pub trait Critic:           Send + Sync + 'static { async fn critique(&self,                       handle: &ResearchHandle) -> Result<CritiqueOutcome>; }
#[async_trait] pub trait CitationVerifier: Send + Sync + 'static { async fn verify  (&self,                       handle: &ResearchHandle) -> Result<()>; }

// ---- The shared handle. Mirrors meetings ToolHandle. ----
pub struct ResearchHandle {
    inner:   Arc<Mutex<ResearchResult>>,
    request: Arc<ResearchRequest>,
    search:  Arc<dyn WebSearch>,                          // from web-search-core
    retriever: Option<Arc<dyn atomr_agents_retriever::Retriever>>,
    events:  Option<broadcast::Sender<DeepResearchEvent>>,
}
// Methods: snapshot(), record_clarification(), append_sub_question(), record_search_hit(),
//          append_draft_section(), append_citation(), record_critique(),
//          mark_verified(), set_final_report(), finalize().
```

### Default deterministic impls (LLM-free, ship in this crate)

- `TemplateClarifier`: returns a deterministic list of clarifying questions derived from the query (regex-detected ambiguity markers); if `HitlPolicy::AutoClarify`, fills them with reasonable defaults from `ResearchScope`.
- `HeuristicPlanner`: splits query into sub-questions by sentence + conjunction analysis; assigns a flat outline.
- `MockResearcher`: calls `WebSearchTool` with mock provider + optional retriever; records hits.
- `ConcatWriter`: concatenates per-sub-question summaries under outline headings.
- `RegexCritic`: flags sections with no citations, duplicate citations, or unmet sub-questions.
- `DeterministicCitationVerifier`: dedupes citations, renumbers, sanitizes URLs, checks every numbered marker has a matching entry.

### LLM-driven impls (ship in this crate, behind `agent` feature flag)

Each role has an `AgentBased{Role}` impl that wraps an `atomr_agents_agent::Agent` (so the existing per-turn retrieve→select-tools→model→tool-loop pipeline drives it). The `LlmOverrides` field in `ResearchRequest` picks the model per role.

---

## The three v1 strategies

All three are `DeepResearchLoopStrategy` impls in `crates/deep-research-harness/src/strategies/`. Each consumes the same role traits via `DeepResearchStepCtx`; they differ in how they orchestrate them.

### 1. `ClarifyPlanSearchVerifyLoop` (NVIDIA AI-Q signature; default)

File: `strategies/clarify_plan_search_verify.rs`

```
iter 1:  clarifier.clarify  → records clarifications (or skips if already present in request)
iter 2:  planner.plan       → outline + sub_questions
iter 3:  for each sub_question (sequential): researcher.research
iter 4:  writer.write       → first draft
iter 5:  critic.critique    → flags gaps; if gaps and depth < max_depth, loops back to iter 3 over new sub_questions; else continue
iter 6:  verifier.verify    → deterministic citation pass
         Done
```

Mirrors AI-Q's "publication-ready" deep flow. Recommended default. Auditability is the differentiator: every citation traces back to a recorded search hit, and the verifier pass is uniform across strategies.

### 2. `MultiAgentParallelLoop` (Anthropic multi-agent research)

File: `strategies/multi_agent_parallel.rs`

```
iter 1:  clarifier.clarify (optional)
iter 2:  planner.plan      → outline + sub_questions + assigns subagent_count
iter 3:  fan out: tokio::spawn one researcher.research per sub_question (capped by request.breadth)
         joined via futures::future::join_all
iter 4:  writer.write      → composes sections; lead writes from accumulated handle state
iter 5:  verifier.verify
         Done
```

Each spawned researcher owns a clone of the `ResearchHandle` (cheap: `Arc<Mutex<…>>`). Concurrency cap = `min(plan.sub_questions.len(), request.breadth)`. Per-subagent telemetry threaded into `Telemetry::per_node`.

### 3. `IterativeDeepeningLoop` (LangGraph open_deep_research)

File: `strategies/iterative_deepening.rs`

```
iter N (loop):
  supervisor_decision = critic.critique(handle)           // doubles as the "think_tool"
  if supervisor_decision == Done:                         break
  if supervisor_decision suggests new sub-questions:
      planner.plan(refined_request) appends them
  researcher.research(next_sub_question)
  // crucial: compress before persisting — Researcher impl is responsible for
  // emitting a compressed summary into the handle rather than raw hits.
final: writer.write; verifier.verify; Done
```

Termination interplay: `IterationCapTermination` from the base harness caps the supervisor loop. The depth field in `ResearchRequest` is the soft cap; the harness's `TerminationStrategy` is the hard cap. The "compress before returning" behavior lives in the LLM-driven `AgentBasedResearcher`, gated by a `compress_findings: bool` field on its config — the deterministic `MockResearcher` always writes compressed summaries anyway.

---

## Web-search tool layer

### `crates/web-search-core`

```rust
#[derive(Clone, Debug)] pub struct WebSearchRequest  { pub query: String, pub max_results: u32, pub allowed_domains: Vec<String>, pub recency_days: Option<u32>, … }
#[derive(Clone, Debug)] pub struct WebSearchHit      { pub url: Url, pub title: String, pub snippet: String, pub published: Option<DateTime<Utc>>, pub source: String, … }

#[async_trait]
pub trait WebSearch: Send + Sync + 'static {
    async fn search(&self, req: &WebSearchRequest) -> Result<Vec<WebSearchHit>>;
    fn provider_name(&self) -> &str;
}

/// Deterministic test fixture; returns hits from an in-memory corpus.
pub struct MockWebSearch { /* corpus: Vec<(query_substr, Vec<WebSearchHit>)> */ }
```

### `crates/web-search-tool`

```rust
pub struct WebSearchTool { provider: Arc<dyn WebSearch> }

#[async_trait]
impl atomr_agents_tool::Tool for WebSearchTool { /* descriptor: name="web_search", input/output schemas matching the core types */ }
```

This is the *first* web-search tool in the workspace, and it's deliberately placed outside `deep-research-harness` so agents, workflows, and skills can use it directly via the `ToolSet` infrastructure. Future provider crates (Tavily, SerpAPI, DuckDuckGo, Brave) implement `WebSearch` and depend only on `web-search-core`.

Local-corpus search reuses `atomr-agents-retriever` directly — the `Researcher` role accepts both an `Arc<dyn WebSearch>` and an `Option<Arc<dyn Retriever>>` and queries both, merging hits into the handle.

---

## Crate layout for `crates/deep-research-harness`

```
crates/deep-research-harness/
├── Cargo.toml
├── src/
│   ├── lib.rs              # re-exports + crate-level docs
│   ├── spec.rs             # DeepResearchHarnessSpec, DeepResearchConfig, builder methods
│   ├── harness.rs          # DeepResearchHarness<L, T>, run_impl, HarnessDispatch impl
│   ├── boxed.rs            # BoxedDeepResearchHarness
│   ├── dispatch.rs         # DeepResearchHarnessRef (Callable impl)
│   ├── state.rs            # DeepResearchState, DeepResearchStepEvent
│   ├── loop_strategy.rs    # DeepResearchLoopStrategy trait, DeepResearchStepCtx, StepOutcome
│   ├── termination.rs      # IterationCap + DepthCap + BudgetCap + CoverageReached
│   ├── handle.rs           # ResearchHandle + tools
│   ├── tools/              # AppendSubQuestion, RecordSearchHit, AppendDraftSection,
│   │   ├── mod.rs          # AppendCitation, RecordCritique, FinalizeReport — each `Tool` impl
│   │   └── …               # so LLM-driven impls dispatch through them uniformly
│   ├── roles/              # Trait + default impls per role
│   │   ├── mod.rs
│   │   ├── clarifier.rs    # Clarifier trait + TemplateClarifier + AgentBasedClarifier
│   │   ├── planner.rs      # Planner   + HeuristicPlanner + AgentBasedPlanner
│   │   ├── researcher.rs   # Researcher + MockResearcher  + AgentBasedResearcher
│   │   ├── writer.rs       # Writer    + ConcatWriter    + AgentBasedWriter
│   │   ├── critic.rs       # Critic    + RegexCritic     + AgentBasedCritic
│   │   └── verifier.rs     # CitationVerifier + DeterministicCitationVerifier
│   ├── strategies/         # The topology slot — one impl per
│   │   ├── mod.rs
│   │   ├── clarify_plan_search_verify.rs   # AI-Q
│   │   ├── multi_agent_parallel.rs         # Anthropic
│   │   └── iterative_deepening.rs          # LangGraph ODR
│   ├── store.rs            # ResearchStore trait + InMemoryResearchStore (+ Checkpointer impl, feature-gated)
│   ├── events.rs           # DeepResearchEvent (broadcast) + framework EventBus wiring
│   └── error.rs            # DeepResearchError
└── tests/
    └── integration_test.rs  # one per strategy, all using deterministic defaults
```

Files to take direct inspiration from:
- `crates/meetings-harness/src/loop_strategy.rs:39` — `MeetingsStepCtx` shape.
- `crates/meetings-harness/src/extractor.rs:73` — pluggable trait shape.
- `crates/meetings-harness/src/tools/mod.rs` — tool-centric mutation pattern.
- `crates/stt-harness/src/harness.rs` — typed/boxed/ref triad.
- `crates/harness/examples/research_harness.rs:61` — already prototypes the loop sketch we're formalizing.

---

## Persistence + events

**Persistence**: `ResearchStore` trait (`list`, `get`, `put`, `delete`) with `InMemoryResearchStore` shipped, and a feature-gated `CheckpointerResearchStore` (mirrors `CheckpointerConversationStore` in stt-harness). The store is what the web UI reads.

**Events**: same dual-channel pattern as the other harnesses.
- Framework: `Event::HarnessIteration` to the shared `EventBus`.
- Domain: `DeepResearchEvent` enum over `tokio::broadcast::Sender`. Variants: `Started`, `ClarificationsRecorded`, `PlanComposed`, `SearchHitRecorded`, `SubQuestionDone`, `DraftSectionAppended`, `CritiqueRecorded`, `VerificationComplete`, `Finalized`, `Failed`.

This lets the web UI subscribe to a run and render incremental progress without polling.

---

## `crates/deep-research-harness-web`

Mirror `crates/meetings-harness-web` / `crates/stt-harness-web` structure:
- Axum server exposing:
  - `POST /research` — create a run (body = `ResearchRequest`)
  - `GET  /research/:id` — current `ResearchResult` snapshot
  - `GET  /research/:id/events` — SSE stream of `DeepResearchEvent`
  - `GET  /research` — list runs
- Embedded UI (`rust-embed`) — single-page app: query box, scope controls, live transcript pane (per-node steps), draft pane, citations pane, telemetry footer.
- Strategy selector dropdown wired to the three v1 topologies.

UI patterns and embedding setup are copied from `crates/stt-harness-web` — same crates (`axum`, `tower-http`, `rust-embed`) already in `[workspace.dependencies]`.

---

## Reuse map

What we *don't* invent — we lean on existing atomr-agents pieces:

| Need                          | Existing piece                                          |
|-------------------------------|---------------------------------------------------------|
| Base loop contract            | `crates/harness/src/harness.rs::run_impl`               |
| Termination strategy          | `IterationCapTermination` from `crates/harness`         |
| Per-iteration events          | `EventBus` from `crates/observability`                  |
| Local-corpus search           | `Retriever` trait from `crates/retriever`               |
| LLM-driven roles              | `Agent` runtime from `crates/agent`                     |
| Tool plumbing                 | `Tool` / `ToolDescriptor` / `ToolSet` from `crates/tool`|
| Persistence (feature-gated)   | `Checkpointer` from `crates/state`                      |
| Token / time / money budgets  | `TokenBudget`, `TimeBudget`, `MoneyBudget` from `crates/core` |
| Composability                 | `Callable` from `crates/callable`                       |
| Python facade (later)         | `crates/py-bindings`                                    |

---

## Suggested PR breakdown

Five PRs, sized to be reviewable individually:

1. **`web-search-core` + `web-search-tool`** — provider-agnostic trait, mock impl, `Tool` wrapper, workspace wiring. No deep-research code yet.
2. **`deep-research-core`** — request/result/citation/plan types + serde + unit tests. Pure data.
3. **`deep-research-harness` scaffold** — spec, harness, loop strategy trait, role traits, deterministic defaults, in-memory store, events, error. Stub strategies that return `Done` with empty output, so the typed/boxed/ref triad compiles and runs.
4. **Three strategies** — `ClarifyPlanSearchVerifyLoop`, `MultiAgentParallelLoop`, `IterativeDeepeningLoop`, with integration tests against deterministic role impls.
5. **`deep-research-harness-web`** — Axum server, SSE event stream, embedded UI.

`AgentBased{Role}` LLM-driven impls can be added incrementally within PRs 3–4 or as a 6th PR, behind the `agent` feature flag — they're additive.

---

## Critical files to modify / create

**Workspace wiring (modify):**
- `Cargo.toml` — add 5 new crate members + `[workspace.dependencies]` entries at `0.10.0`.

**New crate roots (create):**
- `crates/web-search-core/{Cargo.toml,src/lib.rs}`
- `crates/web-search-tool/{Cargo.toml,src/lib.rs}`
- `crates/deep-research-core/{Cargo.toml,src/lib.rs}`
- `crates/deep-research-harness/` — full structure shown above.
- `crates/deep-research-harness-web/` — mirrors `crates/stt-harness-web`.

**Docs to update / add (after code lands):**
- `docs/architecture.md` — add deep-research-harness to the harness section.
- `docs/deep-research-harness.md` — new, mirroring `docs/meetings-harness.md`. This planning doc (`docs/deep-research-harness-plan.md`) is removed at that point.

---

## Verification

After each PR:

1. **Compile**: `cargo build -p <new-crate>` from repo root.
2. **Test the new crate**: `cargo test -p <new-crate>`.
3. **Workspace check**: `cargo check --workspace` to confirm no regression in any sibling crate.

End-to-end for the harness (after PR 4):

```rust
// Pseudocode for the integration test:
let spec = DeepResearchHarnessSpec::new("dr-test")
    .with_strategy_name("clarify-plan-search-verify");
let harness = DeepResearchHarness::new(
    spec,
    ClarifyPlanSearchVerifyLoop::new(),
    IterationCapTermination { cap: 12 },
    /* roles: */ TemplateClarifier, HeuristicPlanner, MockResearcher::with_corpus(...),
                 ConcatWriter, RegexCritic, DeterministicCitationVerifier,
    /* search: */ Arc::new(MockWebSearch::with_fixture(...)),
);
let request = ResearchRequest {
    query: "compare actor frameworks in Rust".into(),
    depth: 2, breadth: 3, ..Default::default()
};
let result: ResearchResult = harness.run(request).await?;
assert!(result.final_report.is_some());
assert!(!result.citations.is_empty());
assert_eq!(result.strategy, "clarify-plan-search-verify");
assert!(result.transcript.iter().any(|s| s.role == "verifier"));
```

Equivalent tests for each of the other two strategies, asserting the strategy-specific transcript shape (e.g. parallel researcher entries for `MultiAgentParallelLoop`, multiple critic→planner rounds for `IterativeDeepeningLoop`).

For the web crate (after PR 5):

1. `cargo run -p deep-research-harness-web` — server starts.
2. Open the embedded UI in a browser, kick off a run with the mock provider, watch the SSE-driven transcript fill in, confirm the final report renders with numbered citations.
3. Switch the strategy dropdown between the three topologies; verify the same request shape produces three differently-shaped transcripts but the same `ResearchResult` schema.

Linting / quality (run before each PR):
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
