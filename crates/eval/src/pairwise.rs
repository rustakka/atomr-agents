//! Pairwise eval — judge picks A vs B and emits a preference.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

use crate::judge::JudgeModel;
use crate::scorer::{AsyncScorer, ScorerOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairwiseChoice {
    A,
    B,
    Tie,
}

pub struct PairwiseScorer {
    pub model: Arc<dyn JudgeModel>,
    pub criteria_label: String,
}

impl PairwiseScorer {
    pub fn new(model: Arc<dyn JudgeModel>, criteria_label: impl Into<String>) -> Self {
        Self {
            model,
            criteria_label: criteria_label.into(),
        }
    }

    pub async fn compare(&self, prompt: &str, a: &Value, b: &Value) -> Result<(PairwiseChoice, String)> {
        let p = format!(
            "Pairwise preference task. Criterion: {}\n\nPrompt:\n{prompt}\n\nResponse A:\n{a}\n\nResponse B:\n{b}\n\nReply on the first line with one of: A, B, or TIE. Then on the next line a short justification.",
            self.criteria_label
        );
        let reply = self.model.judge(&p).await?;
        let choice = reply
            .lines()
            .next()
            .map(|s| s.trim().to_uppercase())
            .unwrap_or_default();
        let pc = match choice.as_str() {
            "A" => PairwiseChoice::A,
            "B" => PairwiseChoice::B,
            _ => PairwiseChoice::Tie,
        };
        let note = reply.lines().nth(1).unwrap_or("").trim().to_string();
        Ok((pc, note))
    }
}

#[async_trait]
impl AsyncScorer for PairwiseScorer {
    /// Treat `expected` as Response A and `actual` as Response B and
    /// run a pairwise judgment. The criterion label doubles as the
    /// task prompt context — sufficient for cases where the comparison
    /// criterion is fully described by `criteria_label`. Callers
    /// needing a richer prompt should use `compare()` directly.
    ///
    /// Score mapping:
    /// - A wins → score 1.0, passed=true
    /// - tie    → score 0.5, passed=true
    /// - B wins → score 0.0, passed=false
    async fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        let prompt = self.criteria_label.clone();
        match self.compare(&prompt, expected, actual).await {
            Ok((choice, note)) => {
                let (passed, score) = match choice {
                    PairwiseChoice::A => (true, 1.0),
                    PairwiseChoice::Tie => (true, 0.5),
                    PairwiseChoice::B => (false, 0.0),
                };
                ScorerOutcome { passed, score, note }
            }
            Err(e) => ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!("pairwise error: {e}"),
            },
        }
    }
}

/// Aggregate a series of pairwise comparisons into a preference rate
/// for option A.
pub fn preference_rate(votes: &[PairwiseChoice]) -> f32 {
    if votes.is_empty() {
        return 0.0;
    }
    let a_score: f32 = votes
        .iter()
        .map(|c| match c {
            PairwiseChoice::A => 1.0,
            PairwiseChoice::Tie => 0.5,
            PairwiseChoice::B => 0.0,
        })
        .sum();
    a_score / votes.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct ScriptedJudge {
        replies: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl JudgeModel for ScriptedJudge {
        async fn judge(&self, _prompt: &str) -> Result<String> {
            Ok(self.replies.lock().remove(0))
        }
    }

    #[tokio::test]
    async fn pairwise_picks_a_or_b() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["A\nclearer answer".into()]),
        });
        let s = PairwiseScorer::new(m, "helpfulness");
        let (c, note) = s
            .compare("hi", &Value::String("a".into()), &Value::String("b".into()))
            .await
            .unwrap();
        assert_eq!(c, PairwiseChoice::A);
        assert!(note.contains("clearer"));
    }

    #[tokio::test]
    async fn async_scorer_picks_a_as_pass_with_score_one() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["A\nclearer".into()]),
        });
        let s = PairwiseScorer::new(m, "helpfulness");
        let out = AsyncScorer::score(
            &s,
            &Value::String("expected".into()),
            &Value::String("actual".into()),
        )
        .await;
        assert!(out.passed);
        assert!((out.score - 1.0).abs() < 1e-6);
        assert!(out.note.contains("clearer"));
    }

    #[tokio::test]
    async fn async_scorer_b_choice_fails_with_score_zero() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["B\nbetter".into()]),
        });
        let s = PairwiseScorer::new(m, "quality");
        let out = AsyncScorer::score(
            &s,
            &Value::String("expected".into()),
            &Value::String("actual".into()),
        )
        .await;
        assert!(!out.passed);
        assert!((out.score - 0.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn async_scorer_tie_passes_with_half_score() {
        let m = Arc::new(ScriptedJudge {
            replies: Mutex::new(vec!["TIE\nequal".into()]),
        });
        let s = PairwiseScorer::new(m, "quality");
        let out = AsyncScorer::score(
            &s,
            &Value::String("expected".into()),
            &Value::String("actual".into()),
        )
        .await;
        assert!(out.passed);
        assert!((out.score - 0.5).abs() < 1e-6);
    }

    #[test]
    fn preference_rate_averages_choices() {
        let votes = vec![
            PairwiseChoice::A,
            PairwiseChoice::A,
            PairwiseChoice::B,
            PairwiseChoice::Tie,
        ];
        // 1 + 1 + 0 + 0.5 = 2.5; /4 = 0.625
        assert!((preference_rate(&votes) - 0.625).abs() < 1e-5);
    }
}
