use std::sync::Arc;

use atomr_agents_callable::Callable;
use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, Result, TimeBudget, TokenBudget, Value};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::scorer::{Scorer, ScorerOutcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub input: Value,
    pub expected: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub case_id: String,
    pub outcome: ScorerOutcome,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvalRun {
    pub passed: u32,
    pub failed: u32,
    pub avg_score: f32,
    pub results: Vec<EvalResult>,
}

impl EvalRun {
    pub fn pass_rate(&self) -> f32 {
        let total = self.passed + self.failed;
        if total == 0 {
            return 0.0;
        }
        self.passed as f32 / total as f32
    }
}

pub struct EvalSuite {
    pub id: String,
    pub cases: Vec<EvalCase>,
    pub scorer: Arc<dyn Scorer>,
}

impl EvalSuite {
    pub async fn run(&self, callable: &dyn Callable) -> Result<EvalRun> {
        let mut run = EvalRun::default();
        let mut total_score = 0.0f32;
        for case in &self.cases {
            let t0 = std::time::Instant::now();
            let actual = callable.call(case.input.clone(), default_ctx()).await?;
            let outcome = self.scorer.score(&case.expected, &actual);
            if outcome.passed {
                run.passed += 1;
            } else {
                run.failed += 1;
            }
            total_score += outcome.score;
            run.results.push(EvalResult {
                case_id: case.id.clone(),
                outcome,
                elapsed_ms: t0.elapsed().as_millis() as u64,
            });
        }
        let total = (run.passed + run.failed) as f32;
        run.avg_score = if total == 0.0 { 0.0 } else { total_score / total };
        Ok(run)
    }
}

fn default_ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(8192),
        time: TimeBudget::new(Duration::from_secs(30)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(8),
        trace: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::ContainsScorer;
    use atomr_agents_callable::FnCallable;

    #[tokio::test]
    async fn suite_scores_cases() {
        let suite = EvalSuite {
            id: "demo".into(),
            cases: vec![
                EvalCase {
                    id: "c1".into(),
                    input: serde_json::json!("hi"),
                    expected: serde_json::json!({"must_contain": "hi"}),
                },
                EvalCase {
                    id: "c2".into(),
                    input: serde_json::json!("bye"),
                    expected: serde_json::json!({"must_contain": "hello"}),
                },
            ],
            scorer: Arc::new(ContainsScorer),
        };
        let echo = FnCallable::labeled("echo", |v: Value, _ctx| async move { Ok(v) });
        let r = suite.run(&echo).await.unwrap();
        assert_eq!(r.passed, 1);
        assert_eq!(r.failed, 1);
        assert!((r.pass_rate() - 0.5).abs() < 1e-6);
    }
}
