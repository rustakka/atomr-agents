//! Intent classifier: route an incoming [`ResearchRequest`] to either
//! the fast shallow path or the full deep-research harness.
//!
//! The default deterministic implementation is
//! [`HeuristicIntentClassifier`]; an LLM-backed
//! `AgentBasedIntentClassifier` would slot in here once the `agent`
//! feature on the deep-research-harness lands (see PR 4 of the v2
//! plan). The trait is intentionally object-safe so callers can swap
//! implementations behind an `Arc<dyn IntentClassifier>`.

use async_trait::async_trait;
use atomr_agents_deep_research_core::ResearchRequest;

use crate::error::Result;

/// Which tier should service a given [`ResearchRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResearchTier {
    /// Single web-search round, no clarifier / planner / critic loop.
    Shallow,
    /// Full deep-research harness pipeline.
    Deep,
}

/// Object-safe trait every intent classifier implements.
#[async_trait]
pub trait IntentClassifier: Send + Sync + 'static {
    /// Decide which tier should handle `req`.
    async fn classify(&self, req: &ResearchRequest) -> Result<ResearchTier>;
}

// NOTE: an `AgentBasedIntentClassifier` (LLM-driven, structured-output)
// would live alongside this module guarded by the deep-research-harness
// `agent` feature. PR 4 of the v2 plan introduces that feature and the
// `InferenceClientFactory` plumbing it needs — until then the only
// in-tree impl is the heuristic one below.

/// Deterministic, LLM-free intent classifier.
///
/// Classifies a request as [`ResearchTier::Shallow`] when **all** of
/// the following hold; otherwise it routes to [`ResearchTier::Deep`]:
///
/// 1. `req.query.chars().count()` is strictly less than
///    [`HeuristicIntentClassifier::max_shallow_query_chars`].
/// 2. The query contains zero or one `?` characters (multiple sub-
///    questions imply deep).
/// 3. `req.depth` is `<= ` [`HeuristicIntentClassifier::max_shallow_depth`].
/// 4. The query contains none of the configured comparative markers
///    (case-insensitive substring match).
///
/// All thresholds are tunable via the `with_*` builder methods.
#[derive(Debug, Clone)]
pub struct HeuristicIntentClassifier {
    /// Strict upper bound on `chars().count()` of a shallow query.
    pub max_shallow_query_chars: usize,
    /// Maximum allowed `?` count in a shallow query.
    pub max_shallow_question_marks: usize,
    /// Maximum allowed `req.depth` for a shallow query.
    pub max_shallow_depth: u32,
    /// Case-insensitive substrings whose presence forces the request
    /// to the deep tier.
    pub comparative_markers: Vec<String>,
}

impl Default for HeuristicIntentClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl HeuristicIntentClassifier {
    /// Build a heuristic classifier with the documented defaults.
    pub fn new() -> Self {
        Self {
            max_shallow_query_chars: 80,
            max_shallow_question_marks: 1,
            max_shallow_depth: 1,
            comparative_markers: default_comparative_markers(),
        }
    }

    /// Override the strict character-count upper bound.
    pub fn with_max_shallow_query_chars(mut self, n: usize) -> Self {
        self.max_shallow_query_chars = n;
        self
    }

    /// Override the maximum allowed `?` count for shallow queries.
    pub fn with_max_shallow_question_marks(mut self, n: usize) -> Self {
        self.max_shallow_question_marks = n;
        self
    }

    /// Override the maximum allowed `req.depth` for shallow queries.
    pub fn with_max_shallow_depth(mut self, n: u32) -> Self {
        self.max_shallow_depth = n;
        self
    }

    /// Replace the comparative-marker list outright.
    pub fn with_comparative_markers<I, S>(mut self, markers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.comparative_markers = markers.into_iter().map(Into::into).collect();
        self
    }

    /// Pure synchronous classifier used by the `async` trait method.
    /// Public so callers and tests can exercise it without `await`.
    pub fn classify_sync(&self, req: &ResearchRequest) -> ResearchTier {
        let query = req.query.as_str();

        // Rule 1: query length.
        let char_count = query.chars().count();
        if char_count >= self.max_shallow_query_chars {
            return ResearchTier::Deep;
        }

        // Rule 2: '?' count.
        let qm = query.chars().filter(|c| *c == '?').count();
        if qm > self.max_shallow_question_marks {
            return ResearchTier::Deep;
        }

        // Rule 3: depth.
        if req.depth > self.max_shallow_depth {
            return ResearchTier::Deep;
        }

        // Rule 4: comparative markers (case-insensitive).
        let lowered = query.to_lowercase();
        for marker in &self.comparative_markers {
            if lowered.contains(&marker.to_lowercase()) {
                return ResearchTier::Deep;
            }
        }

        ResearchTier::Shallow
    }
}

#[async_trait]
impl IntentClassifier for HeuristicIntentClassifier {
    async fn classify(&self, req: &ResearchRequest) -> Result<ResearchTier> {
        Ok(self.classify_sync(req))
    }
}

/// Canonical default comparative-marker list.
///
/// Each marker is treated as a case-insensitive substring of the query.
/// Some markers include surrounding whitespace deliberately to avoid
/// false positives (e.g. `" vs "` matches `"tokio vs async-std"` but
/// not `"oversight"`).
fn default_comparative_markers() -> Vec<String> {
    vec![
        "compare".into(),
        "versus".into(),
        " vs ".into(),
        " vs.".into(),
        "trade-off".into(),
        "tradeoff".into(),
        "analyze".into(),
        "deep dive".into(),
        "research".into(),
        "contrast".into(),
        "differences between".into(),
        // The multi-entity comparative shape — narrower than a bare
        // "how" which would be far too aggressive.
        "how do ".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::ResearchRequest;

    #[test]
    fn defaults_route_short_queries_shallow() {
        let c = HeuristicIntentClassifier::new();
        let req = ResearchRequest::new("rust").with_depth(1);
        assert_eq!(c.classify_sync(&req), ResearchTier::Shallow);
    }

    #[test]
    fn comparative_markers_force_deep() {
        let c = HeuristicIntentClassifier::new();
        let req = ResearchRequest::new("compare actor frameworks");
        assert_eq!(c.classify_sync(&req), ResearchTier::Deep);
    }

    #[test]
    fn depth_above_threshold_forces_deep() {
        let c = HeuristicIntentClassifier::new();
        let req = ResearchRequest::new("rust").with_depth(3);
        assert_eq!(c.classify_sync(&req), ResearchTier::Deep);
    }

    #[test]
    fn long_queries_force_deep() {
        let c = HeuristicIntentClassifier::new();
        let long = "a".repeat(120);
        let req = ResearchRequest::new(long).with_depth(0);
        assert_eq!(c.classify_sync(&req), ResearchTier::Deep);
    }

    #[test]
    fn multiple_question_marks_force_deep() {
        let c = HeuristicIntentClassifier::new();
        let req = ResearchRequest::new("what? when? where?").with_depth(0);
        assert_eq!(c.classify_sync(&req), ResearchTier::Deep);
    }

    #[test]
    fn builders_override_thresholds() {
        let c = HeuristicIntentClassifier::new()
            .with_max_shallow_query_chars(10)
            .with_max_shallow_depth(0)
            .with_comparative_markers(Vec::<String>::new());
        let req = ResearchRequest::new("hello world").with_depth(0);
        // 11 chars >= 10 → deep.
        assert_eq!(c.classify_sync(&req), ResearchTier::Deep);

        let short = ResearchRequest::new("hi").with_depth(0);
        assert_eq!(c.classify_sync(&short), ResearchTier::Shallow);
    }
}
