//! Composite / weighted scoring + ranked triage (FR-8).
//!
//! Opportunity triage and thesis review need ONE reproducible composite
//! score that aggregates rubric + judge + pairwise + deterministic
//! numeric gates, with an auditable per-scorer breakdown and
//! percentile/rank output, so candidates rank consistently and the
//! decision is defensible.
//!
//! * [`CompositeScorer`] combines component [`Scorer`]s under a weight
//!   and an [`Aggregation`], producing a [`TriageResult`] with a
//!   per-component [`ScorerContribution`] breakdown.
//! * [`Ranker`] ranks a batch of items by composite score, attaching a
//!   stable rank and percentile to each [`RankedItem`].
//!
//! Aggregation is deterministic given identical component outputs; when
//! LLM components run under the recording checkpointer (FR-1) their
//! recorded outputs make the composite reproducible in replay.

use atomr_agents_core::Value;
use serde::{Deserialize, Serialize};

use crate::scorer::{Scorer, ScorerOutcome};

/// How component scores combine into one composite.
#[derive(Clone)]
pub enum Aggregation {
    /// Sum of `raw * weight` across components.
    WeightedSum,
    /// Weighted sum divided by the total weight (weighted average).
    WeightedMean,
    /// The minimum raw component score (worst-link; the composite is no
    /// stronger than its weakest check). Weights are ignored.
    Min,
    /// Caller-supplied reducer over the per-component contributions.
    Custom(fn(&[ScorerContribution]) -> f32),
}

impl std::fmt::Debug for Aggregation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Aggregation::WeightedSum => f.write_str("WeightedSum"),
            Aggregation::WeightedMean => f.write_str("WeightedMean"),
            Aggregation::Min => f.write_str("Min"),
            Aggregation::Custom(_) => f.write_str("Custom(fn)"),
        }
    }
}

/// One component's contribution to a composite score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorerContribution {
    /// Stable component label (`component[i]` if none supplied).
    pub name: String,
    /// The component's own `score` (0.0..=1.0 by convention).
    pub raw: f32,
    /// `raw * weight` — the value that actually feeds weighted
    /// aggregations.
    pub weighted: f32,
}

/// The result of running a [`CompositeScorer`]: a single composite plus
/// the auditable per-component breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriageResult {
    /// Aggregated composite score.
    pub composite: f32,
    /// Per-component contributions, in declaration order.
    pub breakdown: Vec<ScorerContribution>,
}

/// Aggregates several weighted [`Scorer`]s into one composite score.
///
/// Components are `(scorer, weight)` pairs; weights are declarative so
/// they (and the [`Aggregation`]) can be serialized into the eval
/// record for audit.
pub struct CompositeScorer {
    /// Weighted component scorers, in declaration order.
    pub components: Vec<(Box<dyn Scorer>, f64)>,
    /// How the component scores combine.
    pub aggregation: Aggregation,
}

impl CompositeScorer {
    /// Construct a composite scorer.
    pub fn new(components: Vec<(Box<dyn Scorer>, f64)>, aggregation: Aggregation) -> Self {
        Self {
            components,
            aggregation,
        }
    }

    /// Score `actual` against `expected`, returning the composite plus a
    /// per-component breakdown.
    pub fn triage(&self, expected: &Value, actual: &Value) -> TriageResult {
        let breakdown: Vec<ScorerContribution> = self
            .components
            .iter()
            .enumerate()
            .map(|(i, (scorer, weight))| {
                let raw = scorer.score(expected, actual).score;
                ScorerContribution {
                    name: format!("component[{i}]"),
                    raw,
                    weighted: raw * (*weight as f32),
                }
            })
            .collect();

        let composite = match self.aggregation {
            Aggregation::WeightedSum => breakdown.iter().map(|c| c.weighted).sum(),
            Aggregation::WeightedMean => {
                let total_w: f64 = self.components.iter().map(|(_, w)| *w).sum();
                if total_w == 0.0 {
                    0.0
                } else {
                    breakdown.iter().map(|c| c.weighted).sum::<f32>() / total_w as f32
                }
            }
            Aggregation::Min => {
                // Worst-link; empty set degenerates to 0.0 rather than +inf.
                if breakdown.is_empty() {
                    0.0
                } else {
                    breakdown.iter().map(|c| c.raw).fold(f32::INFINITY, f32::min)
                }
            }
            Aggregation::Custom(f) => f(&breakdown),
        };

        TriageResult {
            composite,
            breakdown,
        }
    }
}

impl Scorer for CompositeScorer {
    fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        let triage = self.triage(expected, actual);
        // Composite is "passed" when every component passed (raw == 1.0
        // by the 1.0/0.0 convention). For non-binary components we treat
        // a composite >= the weighted-mean midpoint as passed; the
        // explicit per-component pass test below is the strict rule.
        let all_passed = triage.breakdown.iter().all(|c| c.raw >= 1.0);
        ScorerOutcome {
            passed: all_passed,
            score: triage.composite,
            note: format!(
                "composite {:.4} over {} components ({:?})",
                triage.composite,
                triage.breakdown.len(),
                self.aggregation
            ),
        }
    }
}

/// A batch item with its triage result and ranking position.
#[derive(Debug, Clone)]
pub struct RankedItem<T> {
    /// The original item.
    pub item: T,
    /// The composite triage for this item.
    pub triage: TriageResult,
    /// 1-based rank (1 = best composite).
    pub rank: usize,
    /// Percentile in `[0.0, 100.0]` — fraction of items this item scores
    /// at least as well as.
    pub percentile: f64,
}

/// Ranks a batch of items by composite score (descending).
pub struct Ranker;

impl Ranker {
    /// Rank `items` (each paired with the `actual` Value to score)
    /// against `scorer`. Ordering is by composite descending with a
    /// stable tie-break on original input order.
    pub fn rank<T>(
        items: Vec<(T, Value)>,
        scorer: &CompositeScorer,
    ) -> Vec<RankedItem<T>> {
        // Compute composites preserving original index for stable ties.
        let mut scored: Vec<(usize, T, TriageResult)> = items
            .into_iter()
            .enumerate()
            .map(|(i, (item, actual))| {
                let triage = scorer.triage(&Value::Null, &actual);
                (i, item, triage)
            })
            .collect();

        let n = scored.len();

        // Stable sort by composite desc, original index asc on ties.
        scored.sort_by(|a, b| {
            b.2.composite
                .partial_cmp(&a.2.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        scored
            .into_iter()
            .enumerate()
            .map(|(pos, (_orig, item, triage))| {
                let rank = pos + 1;
                // Percentile: fraction of the batch ranked at or below
                // this item (best = ~100, worst = lower).
                let percentile = if n <= 1 {
                    100.0
                } else {
                    (n - rank) as f64 / (n - 1) as f64 * 100.0
                };
                RankedItem {
                    item,
                    triage,
                    rank,
                    percentile,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // A scorer returning a fixed score (independent of inputs) so tests
    // are deterministic.
    struct Const(f32);
    impl Scorer for Const {
        fn score(&self, _e: &Value, _a: &Value) -> ScorerOutcome {
            ScorerOutcome {
                passed: self.0 >= 1.0,
                score: self.0,
                note: String::new(),
            }
        }
    }

    // A scorer that reads a numeric metric, for ranking by data.
    struct ReadMetric(&'static str);
    impl Scorer for ReadMetric {
        fn score(&self, _e: &Value, a: &Value) -> ScorerOutcome {
            let v = a.get(self.0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            ScorerOutcome {
                passed: v >= 1.0,
                score: v,
                note: String::new(),
            }
        }
    }

    #[test]
    fn weighted_sum_breakdown() {
        let scorer = CompositeScorer::new(
            vec![
                (Box::new(Const(1.0)), 0.5),
                (Box::new(Const(0.5)), 0.5),
            ],
            Aggregation::WeightedSum,
        );
        let t = scorer.triage(&json!({}), &json!({}));
        // 1.0*0.5 + 0.5*0.5 = 0.75
        assert!((t.composite - 0.75).abs() < 1e-6);
        assert_eq!(t.breakdown.len(), 2);
        assert!((t.breakdown[0].weighted - 0.5).abs() < 1e-6);
        assert!((t.breakdown[1].weighted - 0.25).abs() < 1e-6);
    }

    #[test]
    fn weighted_mean() {
        let scorer = CompositeScorer::new(
            vec![
                (Box::new(Const(1.0)), 1.0),
                (Box::new(Const(0.0)), 3.0),
            ],
            Aggregation::WeightedMean,
        );
        let t = scorer.triage(&json!({}), &json!({}));
        // (1*1 + 0*3) / (1+3) = 0.25
        assert!((t.composite - 0.25).abs() < 1e-6);
    }

    #[test]
    fn min_aggregation() {
        let scorer = CompositeScorer::new(
            vec![
                (Box::new(Const(0.9)), 1.0),
                (Box::new(Const(0.3)), 1.0),
            ],
            Aggregation::Min,
        );
        let t = scorer.triage(&json!({}), &json!({}));
        assert!((t.composite - 0.3).abs() < 1e-6);
    }

    #[test]
    fn custom_aggregation() {
        fn pick_max(c: &[ScorerContribution]) -> f32 {
            c.iter().map(|x| x.raw).fold(0.0, f32::max)
        }
        let scorer = CompositeScorer::new(
            vec![
                (Box::new(Const(0.2)), 1.0),
                (Box::new(Const(0.8)), 1.0),
            ],
            Aggregation::Custom(pick_max),
        );
        let t = scorer.triage(&json!({}), &json!({}));
        assert!((t.composite - 0.8).abs() < 1e-6);
    }

    #[test]
    fn composite_scorer_outcome_pass_when_all_pass() {
        let pass = CompositeScorer::new(
            vec![(Box::new(Const(1.0)), 1.0), (Box::new(Const(1.0)), 1.0)],
            Aggregation::WeightedMean,
        );
        assert!(pass.score(&json!({}), &json!({})).passed);

        let fail = CompositeScorer::new(
            vec![(Box::new(Const(1.0)), 1.0), (Box::new(Const(0.5)), 1.0)],
            Aggregation::WeightedMean,
        );
        assert!(!fail.score(&json!({}), &json!({})).passed);
    }

    #[test]
    fn ranker_orders_and_percentiles() {
        let scorer = CompositeScorer::new(
            vec![(Box::new(ReadMetric("v")), 1.0)],
            Aggregation::WeightedSum,
        );
        let items = vec![
            ("low", json!({"v": 0.1})),
            ("high", json!({"v": 0.9})),
            ("mid", json!({"v": 0.5})),
        ];
        let ranked = Ranker::rank(items, &scorer);
        assert_eq!(ranked[0].item, "high");
        assert_eq!(ranked[0].rank, 1);
        assert!((ranked[0].percentile - 100.0).abs() < 1e-6);
        assert_eq!(ranked[1].item, "mid");
        assert!((ranked[1].percentile - 50.0).abs() < 1e-6);
        assert_eq!(ranked[2].item, "low");
        assert_eq!(ranked[2].rank, 3);
        assert!((ranked[2].percentile - 0.0).abs() < 1e-6);
    }

    #[test]
    fn ranker_stable_on_ties() {
        let scorer = CompositeScorer::new(
            vec![(Box::new(ReadMetric("v")), 1.0)],
            Aggregation::WeightedSum,
        );
        let items = vec![
            ("a", json!({"v": 0.5})),
            ("b", json!({"v": 0.5})),
            ("c", json!({"v": 0.5})),
        ];
        let ranked = Ranker::rank(items, &scorer);
        // Stable: original order preserved on equal composites.
        assert_eq!(ranked[0].item, "a");
        assert_eq!(ranked[1].item, "b");
        assert_eq!(ranked[2].item, "c");
    }

    #[test]
    fn ranker_single_item_percentile() {
        let scorer =
            CompositeScorer::new(vec![(Box::new(Const(0.4)), 1.0)], Aggregation::WeightedSum);
        let ranked = Ranker::rank(vec![("only", json!({}))], &scorer);
        assert_eq!(ranked.len(), 1);
        assert!((ranked[0].percentile - 100.0).abs() < 1e-6);
    }
}
