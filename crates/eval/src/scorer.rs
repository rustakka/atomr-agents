use async_trait::async_trait;
use atomr_agents_core::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorerOutcome {
    pub passed: bool,
    pub score: f32,
    pub note: String,
}

/// Sync scorer — pure-CPU comparators (substring match, JSON shape,
/// regex, etc.). Most scorers should implement this.
pub trait Scorer: Send + Sync + 'static {
    fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome;
}

/// Async-friendly scorer for impls that genuinely await — LLM judges,
/// retrieval-grounded checks, anything network-bound. The blanket impl
/// below promotes every sync `Scorer` into an `AsyncScorer`, so callers
/// who hold `Arc<dyn AsyncScorer>` can accept both transparently.
#[async_trait]
pub trait AsyncScorer: Send + Sync + 'static {
    async fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome;
}

#[async_trait]
impl<S: Scorer> AsyncScorer for S {
    async fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        Scorer::score(self, expected, actual)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blanket_async_promotes_sync_scorer() {
        // The blanket `impl<S: Scorer> AsyncScorer for S` means
        // ContainsScorer can be awaited directly — no wrapper, no
        // explicit cast on the call.
        let s = ContainsScorer;
        let out = AsyncScorer::score(
            &s,
            &serde_json::json!({"must_contain": "hi"}),
            &Value::String("oh hi there".into()),
        )
        .await;
        assert!(out.passed);
        assert!((out.score - 1.0).abs() < 1e-6);

        let out2 = AsyncScorer::score(
            &s,
            &serde_json::json!({"must_contain": "missing"}),
            &Value::String("oh hi there".into()),
        )
        .await;
        assert!(!out2.passed);
    }

    #[tokio::test]
    async fn blanket_works_through_trait_object() {
        // Confirm that an `Arc<dyn AsyncScorer>` constructed from a
        // sync Scorer dispatches correctly.
        use std::sync::Arc;
        let s: Arc<dyn AsyncScorer> = Arc::new(ContainsScorer);
        let out = s
            .score(
                &serde_json::json!({"must_contain": "yes"}),
                &Value::String("yes please".into()),
            )
            .await;
        assert!(out.passed);
    }
}
