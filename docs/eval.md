# Eval

The `agents-eval` crate is the test harness for *agent quality*: an
`EvalSuite` runs a set of `EvalCase`s against any `Callable` and
reports pass-rate, average score, and per-case verdicts.
`RegressionGate` blocks publication when pass-rate drops below
baseline; the `Registry::publish_gated` API wires this into version
publishing.

## Anatomy

```rust
use std::sync::Arc;
use atomr_agents_eval::{ContainsScorer, EvalCase, EvalSuite};

let suite = EvalSuite {
    id: "smoke".into(),
    cases: vec![
        EvalCase {
            id: "greeting".into(),
            input:    serde_json::json!("Hi"),
            expected: serde_json::json!({"must_contain": "hello"}),
        },
        EvalCase {
            id: "math".into(),
            input:    serde_json::json!("2+2"),
            expected: serde_json::json!({"must_contain": "4"}),
        },
    ],
    scorer: Arc::new(ContainsScorer),
};

let result = suite.run(&my_callable).await?;
println!("pass_rate = {}", result.pass_rate());
```

`EvalSuite::run` invokes the supplied `Callable` once per case
(under a default `CallCtx`), scores each output, and returns an
`EvalRun`:

```rust
pub struct EvalRun {
    pub passed: u32,
    pub failed: u32,
    pub avg_score: f32,
    pub results: Vec<EvalResult>,
}
```

## Scorers

| Scorer | Behavior |
|---|---|
| `ContainsScorer` | string-presence check (`expected.must_contain` substring of `actual`) |
| `LlmJudgeScorer` | binary pass/fail from a `JudgeModel`; the judge's prompt receives `expected` + `actual` |
| `RubricScorer` | weighted multi-criterion grading (0–10 per criterion); pass-threshold on the weighted average |
| `PairwiseScorer` | A/B preference (returns `PairwiseChoice::{A, B, Tie}`) — useful for relative comparisons across config variants |

The `Scorer` trait is sync (`fn score(&self, expected, actual) ->
ScorerOutcome`) — `LlmJudgeScorer` and `RubricScorer` block on a
tokio `Handle` internally to bridge the async `JudgeModel`. For
production async evaluation, run them inside a tokio runtime and
prefer the direct async API on `PairwiseScorer::compare`.

### LlmJudgeScorer

```rust
use std::sync::Arc;
use atomr_agents_eval::{JudgeModel, LlmJudgeScorer};

struct OpenAiJudge { /* … */ }

#[async_trait::async_trait]
impl JudgeModel for OpenAiJudge {
    async fn judge(&self, prompt: &str) -> atomr_agents_core::Result<String> {
        // Call your model; return "pass\nshort justification" or "fail\n…".
    }
}

let scorer = LlmJudgeScorer::new(Arc::new(OpenAiJudge { /* … */ }));
```

The default prompt asks the judge to reply on the first line with
`pass` or `fail` and on the second line a one-sentence justification.
Override via `LlmJudgeScorer::prompt_template` — the template must
contain `{expected}` and `{actual}` placeholders.

### RubricScorer

```rust
use atomr_agents_eval::{RubricCriterion, RubricScorer};

let scorer = RubricScorer {
    model: Arc::new(judge),
    criteria: vec![
        RubricCriterion { name: "correctness".into(), description: "Is the answer correct?".into(),  weight: 2.0 },
        RubricCriterion { name: "concision".into(),    description: "Is it terse?".into(),            weight: 1.0 },
        RubricCriterion { name: "tone".into(),         description: "Is it polite?".into(),           weight: 0.5 },
    ],
    pass_at: 0.7,    // 70% on the weighted 0–10 scale
};
```

### PairwiseScorer

```rust
use atomr_agents_eval::{PairwiseChoice, PairwiseScorer, preference_rate};

let scorer = PairwiseScorer::new(judge, "helpfulness");
let (choice, note) = scorer.compare(&prompt, &response_a, &response_b).await?;

// Aggregate over many comparisons.
let votes: Vec<PairwiseChoice> = ...;
let a_preferred = preference_rate(&votes);  // 0.0..=1.0
```

## RegressionGate

`RegressionGate` decides whether a new eval run is good enough to
publish, given a baseline:

```rust
use atomr_agents_eval::{EvalRun, RegressionGate};

let gate = RegressionGate { tolerance: 0.05 };
let result = gate.check(&baseline_run, &current_run);

if result.blocked {
    eprintln!("regression: {}", result.reason);
    return Err(/* … */);
}
```

`RegressionResult.blocked` is `true` iff `current.pass_rate() <
baseline.pass_rate() - tolerance`. Pair it with
`Registry::publish_gated` to make eval-regression an enforced gate
on every harness publish:

```rust
use atomr_agents_registry::{ArtifactKind, ArtifactRecord, EvalSummary, Registry};
use semver::Version;

let registry = Registry::new();
registry.publish_gated(
    ArtifactRecord {
        kind: ArtifactKind::Harness,
        id: "coding-harness".into(),
        version: Version::new(0, 2, 0),
        payload: serde_json::json!({/* spec */}),
        published_at_ms: chrono::Utc::now().timestamp_millis(),
        baseline_pass_rate: None,
        current_pass_rate: None,
    },
    Some(&EvalSummary { pass_rate: 0.95 }),  // baseline
    &EvalSummary { pass_rate: current_run.pass_rate() },
    /* tolerance = */ 0.05,
)?;  // Errors with PolicyDenied if regressed.
```

## Annotation queue

`AnnotationQueue` captures items for human review. Use it to bridge
LLM-judge / pairwise eval with manual spot-check:

```rust
use atomr_agents_eval::{AnnotationItem, AnnotationQueue, InMemoryAnnotationQueue, Verdict};
use atomr_agents_core::RunId;

let q: Arc<dyn AnnotationQueue> = Arc::new(InMemoryAnnotationQueue::new());
q.enqueue(AnnotationItem {
    id: "case-1".into(),
    run_id: RunId::from("run-abc"),
    prompt: "what's 2+2?".into(),
    output: serde_json::json!("4"),
    verdict: Verdict::Pending,
    note: None,
    created_at_ms: chrono::Utc::now().timestamp_millis(),
}).await?;

while let Some(item) = q.next_pending().await? {
    // Render in the reviewer's UI; collect their verdict.
    q.submit(&item.id, Verdict::Approved, Some("looks good".into())).await?;
}
```

## Online eval

The framework doesn't ship a separate online-eval daemon — the
recommended pattern is to wire an `AgentMiddleware` that samples K%
of production turns into the same `EvalSuite::run` codepath
asynchronously, and pushes failing samples into the
`AnnotationQueue` for review. The eval suite is just a `Callable`
over a list of cases; nothing about it requires offline execution.

## Where to go from here

- [Observability](observability.md) — eval consumes the same
  `Event` stream that powers the run-tree.
- [Workflows and HITL](workflows-and-hitl.md) — replay a captured
  workflow run against a new harness version to compare.
- [Migrating from LangGraph](migrating-from-langgraph.md) — how
  LangSmith eval concepts map onto the framework.
