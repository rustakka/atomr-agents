use atomr_agents_core::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorerOutcome {
    pub passed: bool,
    pub score: f32,
    pub note: String,
}

pub trait Scorer: Send + Sync + 'static {
    fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome;
}

/// Trivial substring-presence scorer.
pub struct ContainsScorer;

impl Scorer for ContainsScorer {
    fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        let needle = expected
            .get("must_contain")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let hay = match actual {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        let passed = hay.contains(needle);
        ScorerOutcome {
            passed,
            score: if passed { 1.0 } else { 0.0 },
            note: if passed {
                format!("found {needle:?}")
            } else {
                format!("missing {needle:?} in {hay:?}")
            },
        }
    }
}
