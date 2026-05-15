//! LLM-judge scorer + rubric-based scorer.
//!
//! `JudgeModel` is the trait callers plug a model in through; the
//! scorer simply prompts it and parses the response. The judges
//! implement [`AsyncScorer`] directly so they can `await` without a
//! blocking bridge.
//!
//! Note: these scorers do **not** implement the sync [`Scorer`] trait.
//! The blanket `impl<S: Scorer> AsyncScorer for S` in `crate::scorer`
//! would otherwise conflict with the explicit `AsyncScorer` impls
//! here, and the whole point of the explicit impls is to drop the
//! `tokio::task::block_in_place` workaround that a sync impl would
//! force on us. Callers stuck on a sync surface can wrap the model
//! manually or move to `Arc<dyn AsyncScorer>`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use serde::{Deserialize, Serialize};

use crate::scorer::{AsyncScorer, ScorerOutcome};

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

    fn build_prompt(&self, expected: &atomr_agents_core::Value, actual: &atomr_agents_core::Value) -> String {
        self.prompt_template
            .replace("{expected}", &expected.to_string())
            .replace("{actual}", &actual.to_string())
    }
}

fn include_str_template_default() -> String {
    "You are an evaluator. Given the expected outcome and the actual output, reply on the first line with exactly 'pass' or 'fail' and on the next line a one-sentence justification.\n\nExpected:\n{expected}\n\nActual:\n{actual}".into()
}

fn parse_judge_reply(reply: &str) -> ScorerOutcome {
    let first = reply.lines().next().unwrap_or("").trim().to_lowercase();
    let passed = first == "pass";
    ScorerOutcome {
        passed,
        score: if passed { 1.0 } else { 0.0 },
        note: reply.lines().nth(1).unwrap_or("").trim().to_string(),
    }
}

#[async_trait]
impl AsyncScorer for LlmJudgeScorer {
    async fn score(
        &self,
        expected: &atomr_agents_core::Value,
        actual: &atomr_agents_core::Value,
    ) -> ScorerOutcome {
        let prompt = self.build_prompt(expected, actual);
        match self.model.judge(&prompt).await {
            Ok(reply) => parse_judge_reply(&reply),
            Err(e) => ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!("judge error: {e}"),
            },
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

impl RubricScorer {
    fn build_criterion_prompt(
        c: &RubricCriterion,
        expected: &atomr_agents_core::Value,
        actual: &atomr_agents_core::Value,
    ) -> String {
        format!(
            "Score from 0 to 10 ONLY. Criterion: {} — {}.\nExpected:\n{}\nActual:\n{}\nFirst line: integer score. Second line: short justification.",
            c.name, c.description, expected, actual
        )
    }

    fn aggregate(results: &[(&RubricCriterion, f32)], pass_at: f32) -> ScorerOutcome {
        let mut total = 0.0;
        let mut total_w = 0.0;
        let mut notes = Vec::with_capacity(results.len());
        for (c, score) in results {
            total += score * c.weight;
            total_w += c.weight;
            notes.push(format!("{}={}", c.name, score));
        }
        let avg = if total_w > 0.0 { total / total_w } else { 0.0 };
        let normalized = (avg / 10.0).clamp(0.0, 1.0);
        ScorerOutcome {
            passed: normalized >= pass_at,
            score: normalized,
            note: notes.join(", "),
        }
    }
}

fn parse_rubric_score(reply: &str) -> f32 {
    reply
        .lines()
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0)
}

#[async_trait]
impl AsyncScorer for RubricScorer {
    async fn score(
        &self,
        expected: &atomr_agents_core::Value,
        actual: &atomr_agents_core::Value,
    ) -> ScorerOutcome {
        let mut scored: Vec<(&RubricCriterion, f32)> = Vec::with_capacity(self.criteria.len());
        for c in &self.criteria {
            let prompt = Self::build_criterion_prompt(c, expected, actual);
            let reply = match self.model.judge(&prompt).await {
                Ok(r) => r,
                Err(e) => format!("0\njudge error: {e}"),
            };
            scored.push((c, parse_rubric_score(&reply)));
        }
        Self::aggregate(&scored, self.pass_at)
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

    #[tokio::test]
    async fn async_judge_pass_passes() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["pass\nlooks good".into()]),
        });
        let s = LlmJudgeScorer::new(m);
        let r = AsyncScorer::score(&s, &Value::String("yes".into()), &Value::String("yes!".into())).await;
        assert!(r.passed);
        assert!(r.note.contains("looks good"));
    }

    #[tokio::test]
    async fn async_judge_propagates_error_into_outcome() {
        struct FailingJudge;
        #[async_trait]
        impl JudgeModel for FailingJudge {
            async fn judge(&self, _prompt: &str) -> Result<String> {
                Err(atomr_agents_core::AgentError::Tool("boom".into()))
            }
        }
        let s = LlmJudgeScorer::new(Arc::new(FailingJudge));
        let r = AsyncScorer::score(&s, &Value::Null, &Value::Null).await;
        assert!(!r.passed);
        assert!(r.note.contains("judge error"));
    }

    #[tokio::test]
    async fn async_rubric_averages_weighted_scores() {
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
        let r = AsyncScorer::score(&s, &Value::Null, &Value::Null).await;
        assert!((r.score - 0.75).abs() < 1e-5);
        assert!(r.passed);
    }
}
