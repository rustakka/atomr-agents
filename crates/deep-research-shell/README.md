# atomr-agents-deep-research-shell

Two-tier outer shell for the
[deep-research harness](../deep-research-harness/). An
`IntentClassifier` routes each `ResearchRequest` to either a fast
**shallow path** (one `WebSearch` call rendered as a numbered markdown
report) or the **full deep harness**. The shell itself implements
`atomr_agents_callable::Callable`, so it slots into agents, workflows,
and tool registries exactly like the underlying harness.

```text
ResearchRequest
       │
       ▼
┌──────────────────┐         ┌────────────────────────┐
│ IntentClassifier │ Shallow │ DirectSearchShallow    │
│  (default:       ├────────►│  (one WebSearch call,  │──┐
│   Heuristic…)    │         │   numbered references) │  │
└──────────────────┘         └────────────────────────┘  │  ResearchResult
       │ Deep                                            │
       ▼                                                 │
┌────────────────────────────────────────────────┐       │
│ DeepResearchHarnessRef (full clarify→plan→…)   │───────┘
└────────────────────────────────────────────────┘
```

## Defaults

- **`HeuristicIntentClassifier`** — deterministic, LLM-free. Classifies
  shallow when the query is short (`< 80` chars), has at most one `?`,
  `depth <= 1`, and contains no comparative markers (`"compare"`,
  `"versus"`, `" vs "`, `"trade-off"`, `"analyze"`, `"deep dive"`,
  `"research"`, `"contrast"`, `"differences between"`, `"how do "`).
  Every threshold is tunable via `with_*` builders.
- **`DirectSearchShallow`** — issues one
  `WebSearch::search(...)` with `max_results = req.breadth.max(3)`,
  honours `req.scope.allowed_domains` / `blocked_domains`, builds a
  numbered markdown report, and emits citations marked
  `CitationStatus::Verified`.

## Usage

```rust
use std::sync::Arc;
use atomr_agents_callable::Callable;
use atomr_agents_core::{
    AgentContext, AgentId, CallCtx, HarnessId, IterationBudget, MoneyBudget,
    RunId, TimeBudget, TokenBudget,
};
use atomr_agents_deep_research_core::ResearchRequest;
use atomr_agents_deep_research_harness::{
    ClarifyPlanSearchVerifyLoop, DeepResearchHarness, DeepResearchHarnessRef,
    DeepResearchHarnessSpec, DeepResearchRoles, InMemoryResearchStore,
    IterationCapTermination,
};
use atomr_agents_deep_research_shell::{
    DeepResearchShell, DirectSearchShallow, HeuristicIntentClassifier,
};
use atomr_agents_web_search_core::MockWebSearch;

# async fn demo() -> atomr_agents_core::Result<()> {
let web = Arc::new(MockWebSearch::new());

let harness = DeepResearchHarness::new(
    DeepResearchHarnessSpec::new("dr-shell"),
    Arc::new(InMemoryResearchStore::new()),
    web.clone(),
    DeepResearchRoles::defaults(),
    ClarifyPlanSearchVerifyLoop::new(),
    IterationCapTermination::new(64),
);
let deep_ref = DeepResearchHarnessRef::new(
    HarnessId::from("dr-shell"),
    Arc::new(harness.into_boxed()),
);

let shell = DeepResearchShell::new(
    Arc::new(HeuristicIntentClassifier::new()),
    Arc::new(DirectSearchShallow::new(web)),
    deep_ref,
);

let ctx = CallCtx {
    agent_id: None,
    tokens: TokenBudget::new(1_000),
    time: TimeBudget::new(std::time::Duration::from_secs(30)),
    money: MoneyBudget::from_usd(1.0),
    iterations: IterationBudget::new(16),
    trace: vec![],
};
let v = shell
    .call(serde_json::json!({ "query": "rust" }), ctx)
    .await?;
let result: atomr_agents_deep_research_core::ResearchResult =
    serde_json::from_value(v).unwrap();
assert_eq!(result.strategy, "shallow-direct");
# Ok(())
# }
```

## Extending

- Drop in your own `IntentClassifier` (e.g. an LLM-backed one once the
  `agent` feature on the deep-research-harness lands).
- Replace `DirectSearchShallow` with a more elaborate shallow path
  (cache hits, retriever-only search, …).
- Compose the shell behind `with_retry` / `with_timeout` /
  `with_fallbacks` from `atomr-agents-callable`.

## Verification

```
cargo test  -p atomr-agents-deep-research-shell
cargo check --workspace
```
