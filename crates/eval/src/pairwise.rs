//! Pairwise eval — judge picks A vs B and emits a preference.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

use crate::judge::JudgeModel;

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
