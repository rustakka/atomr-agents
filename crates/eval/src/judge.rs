//! LLM-judge scorer + rubric-based scorer.
//!
//! `JudgeModel` is the trait callers plug a model in through; the
//! scorer simply prompts it and parses the response. The
//! `Scorer` trait is sync; we use a blocking call from `tokio` for
//! the async judge (or, in unit tests, a stub that returns a fixed
//! response). For production async use, wrap `LlmJudgeScorer` in an
//! `OnlineEvaluator` that owns its own runtime — see `online_eval`
//! below.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use serde::{Deserialize, Serialize};

use crate::scorer::{Scorer, ScorerOutcome};

#[async_trait]
pub trait JudgeModel: Send + Sync + 'static {
    async fn judge(&self, prompt: &str) -> Result<String>;
}

/// Single-criterion graded scorer — "did the actual output answer the
/// expected question correctly?". The judge replies `pass` / `fail`
/// followed by a short justification.
pub struct LlmJudgeScorer {
    pub model: Arc<dyn JudgeModel>,
    pub prompt_template: String,
}

impl LlmJudgeScorer {
    pub fn new(model: Arc<dyn JudgeModel>) -> Self {
        Self {
            model,
            prompt_template: include_str_template_default(),
        }
    }
}

fn include_str_template_default() -> String {
    "You are an evaluator. Given the expected outcome and the actual output, reply on the first line with exactly 'pass' or 'fail' and on the next line a one-sentence justification.\n\nExpected:\n{expected}\n\nActual:\n{actual}".into()
}

impl Scorer for LlmJudgeScorer {
    fn score(
        &self,
        expected: &atomr_agents_core::Value,
        actual: &atomr_agents_core::Value,
    ) -> ScorerOutcome {
        let prompt = self
            .prompt_template
            .replace("{expected}", &expected.to_string())
            .replace("{actual}", &actual.to_string());
        // Run the async judge synchronously. Callers running inside
        // a tokio runtime can use `OnlineEvaluator` instead.
        let model = self.model.clone();
        let reply = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::try_current()
                .map(|h| h.block_on(model.judge(&prompt)))
                .unwrap_or_else(|_| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(model.judge(&prompt))
                })
        });
        let reply = reply.unwrap_or_else(|e| format!("fail\n{e}"));
        let first = reply.lines().next().unwrap_or("").trim().to_lowercase();
        let passed = first == "pass";
        ScorerOutcome {
            passed,
            score: if passed { 1.0 } else { 0.0 },
            note: reply.lines().nth(1).unwrap_or("").trim().to_string(),
        }
    }
}

// --------------------------------------------------------------------
// RubricScorer — multi-criterion grading. Each criterion is judged
// individually; the final score is the average.
// --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubricCriterion {
    pub name: String,
    pub description: String,
    pub weight: f32,
}

pub struct RubricScorer {
    pub model: Arc<dyn JudgeModel>,
    pub criteria: Vec<RubricCriterion>,
    /// Pass threshold on the (weighted) average score.
    pub pass_at: f32,
}

impl Scorer for RubricScorer {
    fn score(
        &self,
        expected: &atomr_agents_core::Value,
        actual: &atomr_agents_core::Value,
    ) -> ScorerOutcome {
        let mut total = 0.0;
        let mut total_w = 0.0;
        let mut notes = Vec::new();
        for c in &self.criteria {
            let prompt = format!(
                "Score from 0 to 10 ONLY. Criterion: {} — {}.\nExpected:\n{}\nActual:\n{}\nFirst line: integer score. Second line: short justification.",
                c.name, c.description, expected, actual
            );
            let model = self.model.clone();
            let reply = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::try_current()
                    .map(|h| h.block_on(model.judge(&prompt)))
                    .unwrap_or_else(|_| {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .unwrap();
                        rt.block_on(model.judge(&prompt))
                    })
            });
            let reply = reply.unwrap_or_else(|e| format!("0\n{e}"));
            let score: f32 = reply
                .lines()
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0.0);
            total += score * c.weight;
            total_w += c.weight;
            notes.push(format!("{}={}", c.name, score));
        }
        let avg = if total_w > 0.0 { total / total_w } else { 0.0 };
        let normalized = (avg / 10.0).clamp(0.0, 1.0);
        ScorerOutcome {
            passed: normalized >= self.pass_at,
            score: normalized,
            note: notes.join(", "),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::Value;
    use parking_lot::Mutex;

    struct ScriptedJudge {
        replies: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl JudgeModel for ScriptedJudge {
        async fn judge(&self, _prompt: &str) -> Result<String> {
            let mut g = self.replies.lock();
            if g.is_empty() {
                return Ok("fail\nout of replies".into());
            }
            Ok(g.remove(0))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn judge_pass_passes() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["pass\nlooks good".into()]),
        });
        let s = LlmJudgeScorer::new(m);
        let r = s.score(&Value::String("yes".into()), &Value::String("yes!".into()));
        assert!(r.passed);
        assert!(r.note.contains("looks good"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rubric_averages_weighted_scores() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["10\nperfect".into(), "5\nokay".into()]),
        });
        let s = RubricScorer {
            model: m,
            criteria: vec![
                RubricCriterion {
                    name: "correctness".into(),
                    description: "is the answer correct".into(),
                    weight: 1.0,
                },
                RubricCriterion {
                    name: "concision".into(),
                    description: "is it terse".into(),
                    weight: 1.0,
                },
            ],
            pass_at: 0.6,
        };
        let r = s.score(&Value::Null, &Value::Null);
        // (10*1 + 5*1) / (1+1) / 10 = 0.75
        assert!((r.score - 0.75).abs() < 1e-5);
        assert!(r.passed);
    }
}
