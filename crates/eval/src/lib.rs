//! Eval suites + replay-based regression detection.

mod annotation;
mod judge;
mod pairwise;
mod regression;
mod scorer;
mod suite;

pub use annotation::{AnnotationItem, AnnotationQueue, InMemoryAnnotationQueue, Verdict};
pub use judge::{JudgeModel, LlmJudgeScorer, RubricCriterion, RubricScorer};
pub use pairwise::{PairwiseChoice, PairwiseScorer};
pub use regression::{RegressionGate, RegressionResult};
pub use scorer::{AsyncScorer, Scorer, ScorerOutcome};
pub use suite::{EvalCase, EvalResult, EvalRun, EvalSuite};
