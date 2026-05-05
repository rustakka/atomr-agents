---
name: atomr-agents-eval
description: Use when writing eval suites, picking a `Scorer` (Contains / LlmJudge / Rubric / Pairwise), gating publication on `RegressionGate`, queuing items for human annotation, or wiring online eval. Triggers on `EvalSuite { ... }`, `EvalSuite::run`, `LlmJudgeScorer::new`, `PairwiseScorer::compare`, `RegressionGate::check`, or `Registry::publish_gated`.
---

# Eval suites in atomr-agents

`agents-eval` is the test harness for *agent quality*. An
`EvalSuite` runs cases against any `Callable`; scorers report
verdicts; `RegressionGate` blocks publishes that drop pass-rate
below a baseline.

## Mental model

- An **`EvalSuite`** is `(id, cases, scorer)`.
- Each **`EvalCase`** is `(id, input: Value, expected: Value)`.
- A **`Scorer`** maps `(expected, actual)` → `ScorerOutcome
  { passed, score, note }`. The trait is sync; LLM-judge scorers
  bridge async via blocking on a tokio handle.
- An **`EvalRun`** aggregates per-case outcomes plus pass-rate +
  avg-score.
- A **`RegressionGate`** decides whether a current run is good
  enough versus a baseline.
- `Registry::publish_gated` glues these together — refuse to publish
  if the gate blocks.

## Picking a scorer

| Scorer | Use case |
|---|---|
| `ContainsScorer` | smoke test — does the answer mention the right token? |
| `LlmJudgeScorer` | binary pass/fail with an LLM-generated justification |
| `RubricScorer` | weighted multi-criterion grading (correctness, concision, tone, …) |
| `PairwiseScorer` | A vs B preference; compare two model configs side-by-side |

## Running a basic suite

```rust
use std::sync::Arc;
use atomr_agents_eval::{ContainsScorer, EvalCase, EvalSuite};

let suite = EvalSuite {
    id: "smoke".into(),
    cases: vec![
        EvalCase {
            id: "greet".into(),
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

let run = suite.run(&agent_handle).await?;
println!("pass_rate={} avg={}", run.pass_rate(), run.avg_score);
```

`agent_handle` is a `&dyn Callable` — typically the agent or a
`Pipeline` wrapping it.

## LLM-as-judge

```rust
use std::sync::Arc;
use async_trait::async_trait;
use atomr_agents_eval::{JudgeModel, LlmJudgeScorer};
use atomr_agents_core::Result;

struct OpenAiJudge { /* … */ }

#[async_trait]
impl JudgeModel for OpenAiJudge {
    async fn judge(&self, prompt: &str) -> Result<String> {
        // Call your model; return:
        //   "pass\nshort justification"  or
        //   "fail\nwhat went wrong"
        Ok("pass\nlooks good".into())
    }
}

let scorer = LlmJudgeScorer::new(Arc::new(OpenAiJudge { /* ... */ }));
```

Override `prompt_template` to customize what the judge sees. The
template must contain `{expected}` and `{actual}` placeholders.

## Rubric (multi-criterion)

```rust
use atomr_agents_eval::{RubricCriterion, RubricScorer};

let scorer = RubricScorer {
    model: Arc::new(judge),
    criteria: vec![
        RubricCriterion { name: "correctness".into(), description: "Is the answer correct?".into(),  weight: 2.0 },
        RubricCriterion { name: "concision".into(),    description: "Is it terse?".into(),            weight: 1.0 },
        RubricCriterion { name: "tone".into(),         description: "Is it polite?".into(),           weight: 0.5 },
    ],
    pass_at: 0.7,    // 70% on the weighted 0-10 scale
};
```

The judge is called once per criterion; final score is the weighted
average normalized to 0..=1.

## Pairwise

```rust
use atomr_agents_eval::{PairwiseChoice, PairwiseScorer, preference_rate};

let scorer = PairwiseScorer::new(judge, "helpfulness");
let (choice, note) = scorer.compare(&prompt, &response_a, &response_b).await?;
match choice {
    PairwiseChoice::A   => /* A wins */,
    PairwiseChoice::B   => /* B wins */,
    PairwiseChoice::Tie => /* draw */,
}

// Aggregate over many comparisons:
let rate_a_preferred = preference_rate(&votes);  // 0.0..=1.0
```

## Regression gating

```rust
use atomr_agents_eval::RegressionGate;

let gate = RegressionGate { tolerance: 0.05 };
let result = gate.check(&baseline_run, &current_run);
if result.blocked {
    return Err(anyhow!("regression: {}", result.reason));
}
```

Wire this into `Registry::publish_gated` to block harness/agent
publishes on regression:

```rust
use atomr_agents_registry::{ArtifactKind, ArtifactRecord, EvalSummary, Registry};
use semver::Version;

let registry = Registry::new();
registry.publish_gated(
    ArtifactRecord {
        kind: ArtifactKind::Harness,
        id: "coding-harness".into(),
        version: Version::new(0, 2, 0),
        payload: harness_spec_value,
        published_at_ms: chrono::Utc::now().timestamp_millis(),
        baseline_pass_rate: None,
        current_pass_rate: None,
    },
    Some(&EvalSummary { pass_rate: baseline_run.pass_rate() }),
    &EvalSummary { pass_rate: current_run.pass_rate() },
    /* tolerance */ 0.05,
)?;
```

`publish_gated` errors with `AgentError::PolicyDenied(...)` if
`current.pass_rate + tolerance < baseline.pass_rate`.

## Annotation queue

For human review of failing samples:

```rust
use std::sync::Arc;
use atomr_agents_eval::{AnnotationItem, AnnotationQueue, InMemoryAnnotationQueue, Verdict};
use atomr_agents_core::RunId;

let q: Arc<dyn AnnotationQueue> = Arc::new(InMemoryAnnotationQueue::new());

q.enqueue(AnnotationItem {
    id: "case-12".into(),
    run_id: RunId::from("run-abc"),
    prompt: "what's the weather?".into(),
    output: serde_json::json!("It's sunny."),
    verdict: Verdict::Pending,
    note: None,
    created_at_ms: chrono::Utc::now().timestamp_millis(),
}).await?;

while let Some(item) = q.next_pending().await? {
    // Render in a UI; collect the reviewer's choice:
    q.submit(&item.id, Verdict::Approved, Some("matches expected".into())).await?;
}
```

## Online eval

The framework doesn't ship a separate online-eval daemon — wire an
`AgentMiddleware` that samples K% of production turns into the same
`EvalSuite::run` codepath asynchronously, push failing samples into
the `AnnotationQueue`. The eval suite runs against any `Callable`;
nothing about it requires offline execution.

## Canonical references

- [`docs/eval.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/eval.md)
- [`crates/eval/src/`](https://github.com/rustakka/atomr-agents/tree/main/crates/eval/src) — every scorer + the gate
- [`crates/registry/src/lib.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/registry/src/lib.rs) — `publish_gated`

## Common mistakes

- **Calling LLM-judge scorers from inside another async task without
  multi-thread tokio.** The blocking bridge inside the scorer needs
  a multi-thread runtime; mark tests `#[tokio::test(flavor = "multi_thread")]`.
- **Same-suite caching.** `EvalSuite::run` runs every case every
  time. Memoize at the model layer (`SemanticLlmCache`) if you're
  re-running often.
- **Floating-point comparison in `RegressionGate`.** Default
  tolerance `0.05` is sane; below `0.01` you'll get false positives
  from f32 noise.
- **Pairwise A/B without seed control.** If the underlying model has
  randomness, randomize which variant is "A" each comparison to
  avoid position bias.
- **Annotation queue without `created_at_ms`.** The queue uses it
  for ordering; without it you get insertion-order which may not
  match what reviewers expect.
