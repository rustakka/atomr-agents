# Deep Research Harness

`atomr-agents-deep-research-harness` is a pluggable harness for
multi-step, citation-bearing research over a user query. It hosts
three v1 topologies behind one uniform input/output contract
(`ResearchRequest` → `ResearchResult` from
`atomr-agents-deep-research-core`), so callers can swap strategies
without touching the surrounding plumbing.

The harness follows the same conventions as the
[STT harness](stt-harness.md) and the
[meetings harness](meetings-harness.md):

- `Spec → Typed Harness<L, T> → BoxedHarness → HarnessRef` (with
  `HarnessRef` implementing [`Callable`](agent-pipeline.md)).
- A pluggable domain trait (here: six **role traits** — `Clarifier`,
  `Planner`, `Researcher`, `Writer`, `Critic`, `CitationVerifier`)
  threaded through a `StepCtx` to one **loop strategy**.
- A shared `ResearchHandle` that mutates the in-flight `ResearchResult`
  on behalf of the roles (mirrors `ToolHandle` from the meetings
  harness).
- Dual-channel events: framework `EventBus` + an in-process
  `tokio::broadcast` of `DeepResearchEvent`.
- Deterministic LLM-free defaults for every role so tests and the web
  UI run end-to-end without a model provider.

## Crate layout

| Crate | Purpose |
|-------|---------|
| [`atomr-agents-web-search-core`](../crates/web-search-core/) | Provider-agnostic `WebSearch` trait, request/hit types, deterministic `MockWebSearch`. |
| [`atomr-agents-web-search-tool`](../crates/web-search-tool/) | `WebSearchTool` adapter so the trait surface is callable as an `atomr_agents_tool::Tool`. |
| [`atomr-agents-deep-research-core`](../crates/deep-research-core/) | `ResearchRequest`, `ResearchResult`, `Citation`, `Plan`, `SubQuestion`, `NodeStep`, `Telemetry`, `CoverageSignals`, `Artifacts` — pure data. |
| [`atomr-agents-deep-research-harness`](../crates/deep-research-harness/) | Spec, typed/boxed/ref harness, role traits + defaults, three strategies, in-memory store, events, error. |
| [`atomr-agents-deep-research-harness-web`](../crates/deep-research-harness-web/) | Axum + embedded SPA companion. Routes for list/get/start/stop/SSE; vanilla-JS dashboard. |

## The uniform contract

```rust
let req = ResearchRequest::new("compare actor frameworks in rust")
    .with_depth(2)        // max planner / critic refinement rounds
    .with_breadth(3);     // max parallel sub-questions per round
```

The same request feeds every strategy. The result schema is also
uniform — every strategy populates `plan`, `citations`, `transcript`,
and `telemetry`; only the transcript *shape* differs.

```
┌──────────────────────────────────────────────────────────────────┐
│                       ResearchResult                             │
├────────────┬────────┬───────────┬─────────────────────────────┬──┤
│ final_     │ plan   │ citations │ transcript                  │  │
│ report     │ +sub_q │ [N] {url, │ NodeStep { role, label,     │  │
│ (Markdown) │ +out   │ title,    │ ts, summary, sub_q? }       │  │
│            │ line   │ snippet,  │ … audit trail of every role │  │
│            │        │ verified} │ that contributed            │  │
├────────────┴────────┴───────────┴─────────────────────────────┴──┤
│ coverage { answered, unresolved, confidence_per_section, gaps }  │
│ telemetry { tokens, tool_calls, wall_ms, cost_usd, per_node }    │
│ artifacts { drafts, scratchpad, raw_search_hits }                │
└──────────────────────────────────────────────────────────────────┘
```

## Role traits

| Trait | Default impl | Behavior |
|-------|--------------|----------|
| `Clarifier` | `TemplateClarifier` | Heuristic clarifying questions, auto-answered under `HitlPolicy::AutoClarify`. |
| `Planner` | `HeuristicPlanner` | Splits the query on sentence + conjunction boundaries; emits three-section outline. |
| `Researcher` | `MockResearcher` | Calls `WebSearch` per sub-question; records hits and citations; marks the sub-question `Answered` or `Unresolved`. |
| `Writer` | `ConcatWriter` | Groups citations by outline heading and emits a markdown draft. |
| `Critic` | `RegexCritic` | Flags uncited sections, unresolved sub-questions, duplicate citation URLs. |
| `CitationVerifier` | `DeterministicCitationVerifier` | Dedupe + renumber, mark `Verified`, compute coverage signals. |

`DeepResearchRoles::defaults()` returns the full deterministic set —
useful for tests, the web UI demo, and as a baseline for LLM-driven
runs (override one role at a time without rebuilding the rest).

## Strategies

Six built-in topologies, one struct each, all implementing
`DeepResearchLoopStrategy` — three v1 (clarify-plan-search-verify,
multi-agent-parallel, iterative-deepening) plus three v2 (plan-and-
execute, linear-write-critique, outline-first-section-fanout).

### `ClarifyPlanSearchVerifyLoop` (default)

```
clarify → plan → research (per sub-question, sequential)
        → write → critique
        → loop back to research if gaps && refinement_rounds < depth
        → verify → done
```

NVIDIA AI-Q-style. Best when audit trails and citation quality matter
more than wall-clock time.

### `MultiAgentParallelLoop`

```
clarify → plan → fan-out researcher per sub-question (capped by breadth)
        → write → critique → verify → done
```

Anthropic-style multi-agent research. Lowest wall-clock when sub-
questions are independent.

### `IterativeDeepeningLoop`

```
clarify → plan → loop[supervisor (= critic) → research (next sub-q)]
        until done || rounds >= depth
        → write → verify → done
```

LangGraph `open_deep_research`-style. The supervisor doubles as the
critic (the `think_tool`); gaps spawn new sub-questions on the fly.

### `PlanAndExecuteLoop`

```
clarify → plan → for each step:
                   execute (researcher)
                   → critique
                   → if !done && gaps && rounds<depth: re-plan
                     (replaces remaining sub-questions)
                     else: advance to next step
        → write → verify → done
```

Plan-and-execute topology. The planner emits an ordered list of
sub-questions as steps; the researcher runs each one; the critic runs
**after every step** (not only at end). When the critic finds gaps and
depth allows, the planner is re-invoked mid-flow and the new plan
replaces remaining sub-questions. Best when each step's outcome should
gate the next step's plan.

### `LinearWriteCritiqueLoop`

```
clarify → plan → research all sub-questions sequentially
        → write → critique
        → loop back to write until done || rounds >= depth
        → verify → done
```

Simplest "draft then refine" topology — never loops back to research,
only to write. The `Writer` trait receives `&ResearchHandle`, so a
refining writer reads `handle.snapshot()` and the latest critique
transcript entry to revise the draft. Useful when evidence is fixed up
front and only the prose needs iteration.

### `OutlineFirstSectionFanoutLoop`

```
clarify → plan → group sub-questions by Plan::outline section
        → fan out one task per section (parallel, capped by breadth)
        → each task sequentially researches its section's sub-questions
        → single writer pass → verify → done
```

Section-centric instead of sub-question-centric. Sub-questions with
`section: None` go into a synthetic `"Uncategorized"` bucket. Trades
some intra-section parallelism for clean per-section attribution —
useful when the report's outline is naturally section-shaped (e.g.
"Background / Findings / Conclusion") and each section is independent.

## Web search

`crates/web-search-core` is provider-agnostic on purpose: agents,
workflows, and future harnesses can use `WebSearch` without depending
on `deep-research-harness`. The crate ships a deterministic
`MockWebSearch` so tests run offline. Concrete providers live in their
own crates and follow the `web-search-provider-<name>` naming
convention:

- **`atomr-agents-web-search-provider-tavily`** —
  `POST https://api.tavily.com/search` with `TAVILY_API_KEY` in the
  JSON body. Surfaces Tavily's cleaned-text extracts via
  `WebSearchHit.content`.
- **`atomr-agents-web-search-provider-serpapi`** —
  `GET https://serpapi.com/search` (Google engine by default) with
  `SERPAPI_KEY` in the query string. Recency requests are translated
  to Google's `tbs=qdr:d|w|m|y` knob.
- **`atomr-agents-web-search-provider-brave`** —
  `GET https://api.search.brave.com/res/v1/web/search` with
  `BRAVE_API_KEY` in the `X-Subscription-Token` header. Recency maps
  to Brave's `freshness=pd|pw|pm|py`.

All three crates share the `atomr-agents-stt-remote-core` HTTP plumbing
(timeouts, retries, secret refs). Each ships an `integration` Cargo
feature that gates a live-API test on the corresponding env var; the
test skips silently when the var is unset, so CI can run with the
feature on without flaking.

`crates/web-search-tool` wraps any `WebSearch` provider as an
`atomr_agents_tool::Tool` (with a JSON-schema descriptor), so an
LLM-driven role can call `web_search` through the regular tool-call
pipeline.

Local-corpus search continues to flow through
`atomr-agents-retriever`. The `ResearchHandle` carries an optional
`Arc<dyn Retriever>` alongside the `Arc<dyn WebSearch>`, so a
researcher impl can query both and merge the hits.

## Two-tier shell

`atomr-agents-deep-research-shell` wraps the deep harness with an
intent-classifier in front. Short queries take a fast **shallow path**
(one `WebSearch` call rendered as a numbered markdown report); long /
comparative / multi-question queries route to the full deep harness.
The shell itself implements
[`Callable`](agent-pipeline.md), so it slots into agents, workflows,
and tool registries exactly like the underlying harness.

| Piece | Default impl | Behavior |
|-------|--------------|----------|
| `IntentClassifier` | `HeuristicIntentClassifier` | Deterministic. Routes shallow when the query is `< 80` chars, has `<= 1` `?`, `depth <= 1`, and contains no comparative markers (`compare`, `versus`, ` vs `, `trade-off`, `analyze`, `deep dive`, `research`, `contrast`, `differences between`, `how do `). Every threshold has a `with_*` builder. |
| `ShallowResearcher` | `DirectSearchShallow` | Issues one `WebSearch::search(...)` with `max_results = req.breadth.max(3)`, honours `req.scope.allowed_domains` / `blocked_domains`, builds a numbered markdown report, and emits citations marked `CitationStatus::Verified`. Returns `ResearchResult { strategy: "shallow-direct", ... }`. |

```rust
use std::sync::Arc;
use atomr_agents_deep_research_harness::{DeepResearchHarnessRef /* …same as before */};
use atomr_agents_deep_research_shell::{
    DeepResearchShell, DirectSearchShallow, HeuristicIntentClassifier,
};

let shell = DeepResearchShell::new(
    Arc::new(HeuristicIntentClassifier::new()),
    Arc::new(DirectSearchShallow::new(web_search.clone())),
    deep_ref,
);

// Same Callable contract as the underlying harness: ResearchRequest in,
// ResearchResult out, classifier sits transparently in front.
let v = shell.call(serde_json::json!({"query": "rust", "depth": 1}), ctx).await?;
```

## Persistence + events

- **`ResearchStore`** — trait + `InMemoryResearchStore` default. A
  feature-gated `state` integration mirroring the meetings-harness
  `CheckpointerMeetingsStore` is the future extension point.
- **`DeepResearchEvent`** — `Started`, `ClarificationsRecorded`,
  `PlanComposed`, `SubQuestionStarted`, `SubQuestionDone`,
  `SearchHitRecorded`, `DraftSectionAppended`, `CitationAppended`,
  `CritiqueRecorded`, `VerificationComplete`, `TranscriptStep`,
  `Finalized`, `Failed`. Broadcast over `tokio::sync::broadcast`.

The harness persists the in-flight `ResearchResult` after every
iteration, so the web UI sees incremental progress without polling
when the SSE stream is wired up.

## Usage

### Code

```rust
use std::sync::Arc;
use atomr_agents_deep_research_harness::{
    ClarifyPlanSearchVerifyLoop, DeepResearchHarness, DeepResearchHarnessSpec,
    DeepResearchRoles, InMemoryResearchStore, IterationCapTermination,
};
use atomr_agents_deep_research_core::ResearchRequest;
use atomr_agents_web_search_core::MockWebSearch;

let harness = DeepResearchHarness::new(
    DeepResearchHarnessSpec::new("dr-1"),
    Arc::new(InMemoryResearchStore::new()),
    Arc::new(MockWebSearch::new() /* + fixtures */),
    DeepResearchRoles::defaults(),
    ClarifyPlanSearchVerifyLoop::new(),
    IterationCapTermination::new(64),
);

let result = harness.run(
    ResearchRequest::new("compare actor frameworks in rust")
        .with_depth(2).with_breadth(3),
).await?;

assert!(result.final_report.is_some());
assert!(!result.citations.is_empty());
```

### Web

```bash
cargo run -p atomr-agents-deep-research-harness-web --features embed-ui
# 127.0.0.1:7200
```

The dashboard exposes:

- Strategy dropdown (all built-in topologies).
- Live event log (SSE).
- Plan / citations / report panes that refresh on a 1-second cadence
  while the run is in flight.

## Verification

```
cargo test  -p atomr-agents-web-search-core
cargo test  -p atomr-agents-web-search-tool
cargo test  -p atomr-agents-deep-research-core
cargo test  -p atomr-agents-deep-research-harness
cargo test  -p atomr-agents-deep-research-harness-web
cargo check --workspace
```

All 34 tests pass on a clean workspace.

## Roadmap (v2)

- `AgentBased{Role}` impls (behind an `agent` feature flag) wrapping
  `atomr_agents_agent::Agent` for LLM-driven planning, drafting,
  critique, and verification.

The three additional strategies, the Tavily / SerpAPI / Brave provider
crates, and the two-tier shell (`atomr-agents-deep-research-shell`)
listed earlier in this document landed alongside this work.
