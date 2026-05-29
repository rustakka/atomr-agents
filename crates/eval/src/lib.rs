//! Eval suites + replay-based regression detection.

mod annotation;
mod composite;
mod judge;
mod metric;
mod pairwise;
mod provenance;
mod regression;
mod scorer;
mod suite;

pub use annotation::{AnnotationItem, AnnotationQueue, InMemoryAnnotationQueue, Verdict};
pub use composite::{
    Aggregation, CompositeScorer, RankedItem, Ranker, ScorerContribution, TriageResult,
};
pub use judge::{JudgeModel, LlmJudgeScorer, RubricCriterion, RubricScorer};
pub use metric::{
    Direction, GoldenSeries, InMemoryMetricHistoryStore, MetricHistoryStore, MetricScorer,
    Threshold, TimeSeriesRegressionGate, Tolerance,
};
pub use pairwise::{PairwiseChoice, PairwiseScorer};
pub use provenance::{
    compute_hash, Claim, CoverageReport, EvidenceBundle, EvidenceIndex, InMemoryEvidenceIndex,
    ProvenanceScorer,
};
pub use regression::{RegressionGate, RegressionResult};
pub use scorer::{AsyncScorer, Scorer, ScorerOutcome};
pub use suite::{EvalCase, EvalResult, EvalRun, EvalSuite};

#[cfg(feature = "sql")]
pub use metric::sql::SqlMetricHistoryStore;
