//! Numeric / time-series scorers for financial-grade eval (FR-7).
//!
//! Grading a backtest means scoring numeric metrics (Sharpe, max
//! drawdown, turnover, hit-rate) against thresholds and detecting
//! regression versus a metric's historical golden series — not
//! regex-on-text. This module provides:
//!
//! * [`MetricScorer`] — a [`Scorer`] that reads a single numeric metric
//!   from the `actual` JSON object and evaluates a [`Threshold`].
//! * [`TimeSeriesRegressionGate`] — a [`Scorer`] that fails when the new
//!   value regresses beyond a [`Tolerance`] versus a baseline
//!   [`GoldenSeries`], respecting a [`Direction`].
//! * [`MetricHistoryStore`] — a pluggable async store of golden values
//!   keyed by `(suite, metric)`, with an in-memory impl and (behind the
//!   `sql` feature) a SQL-backed impl over SQLite / Postgres.
//!
//! Both scorers compose in `EvalSuite` alongside the text scorers and
//! feed the existing regression-gate machinery, keeping the fund's
//! grading semantics on one contract.

use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

use crate::scorer::{Scorer, ScorerOutcome};

/// Numeric predicate over a single metric value, mirroring atomr-orgs
/// `GateCriterion` threshold semantics for consistency across the fund.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Threshold {
    /// Passes when `value >= bound`.
    AtLeast(f64),
    /// Passes when `value <= bound`.
    AtMost(f64),
    /// Passes when `lo <= value <= hi` (inclusive band).
    Between(f64, f64),
    /// Passes when `value < lo` or `value > hi` (outside the band).
    Outside(f64, f64),
}

impl Threshold {
    /// Evaluate the predicate against `value`.
    pub fn evaluate(&self, value: f64) -> bool {
        match *self {
            Threshold::AtLeast(b) => value >= b,
            Threshold::AtMost(b) => value <= b,
            Threshold::Between(lo, hi) => value >= lo && value <= hi,
            Threshold::Outside(lo, hi) => value < lo || value > hi,
        }
    }
}

/// Read a numeric metric out of the `actual` JSON object. Returns `None`
/// if the key is absent or the value is not coercible to `f64`.
fn read_metric(actual: &Value, metric: &str) -> Option<f64> {
    actual.get(metric).and_then(|v| v.as_f64())
}

/// Scores a single numeric metric against a [`Threshold`].
///
/// The `actual` value handed to [`Scorer::score`] is expected to be a
/// JSON object of metrics (e.g. `{"sharpe": 1.8, "max_drawdown": 0.12}`).
/// A missing or non-numeric metric fails with an explanatory note rather
/// than panicking — a backtest that forgot to emit Sharpe should fail
/// the gate, not crash it.
pub struct MetricScorer {
    /// Key into the `actual` metrics object.
    pub metric: String,
    /// Predicate the metric must satisfy.
    pub predicate: Threshold,
}

impl MetricScorer {
    /// Construct a metric scorer.
    pub fn new(metric: impl Into<String>, predicate: Threshold) -> Self {
        Self {
            metric: metric.into(),
            predicate,
        }
    }
}

impl Scorer for MetricScorer {
    fn score(&self, _expected: &Value, actual: &Value) -> ScorerOutcome {
        match read_metric(actual, &self.metric) {
            Some(value) => {
                let passed = self.predicate.evaluate(value);
                ScorerOutcome {
                    passed,
                    score: if passed { 1.0 } else { 0.0 },
                    note: format!(
                        "metric {:?} = {} vs {:?} => {}",
                        self.metric,
                        value,
                        self.predicate,
                        if passed { "pass" } else { "fail" }
                    ),
                }
            }
            None => ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!("metric {:?} missing or non-numeric in actual", self.metric),
            },
        }
    }
}

/// How "far apart" a new value may drift from the baseline before it
/// counts as a regression.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Tolerance {
    /// Absolute distance from the baseline reference.
    Abs(f64),
    /// Fractional distance (e.g. `0.1` = 10%) relative to the baseline
    /// reference's magnitude.
    Pct(f64),
    /// Number of standard deviations of the baseline series.
    ZScore(f64),
}

/// Which way is "good" for a metric — Sharpe is higher-is-better, max
/// drawdown / turnover are lower-is-better. Determines what counts as a
/// regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Larger values are better (e.g. Sharpe, hit-rate). A drop is a
    /// regression.
    HigherIsBetter,
    /// Smaller values are better (e.g. drawdown, turnover). A rise is a
    /// regression.
    LowerIsBetter,
}

/// The historical "golden" values of a metric from prior approved runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldenSeries(pub Vec<f64>);

impl GoldenSeries {
    /// Whether the series has no observations.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of observations.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The most recent recorded value (the conventional baseline ref).
    pub fn last(&self) -> Option<f64> {
        self.0.last().copied()
    }

    /// Arithmetic mean of the series.
    pub fn mean(&self) -> Option<f64> {
        if self.0.is_empty() {
            return None;
        }
        Some(self.0.iter().sum::<f64>() / self.0.len() as f64)
    }

    /// Population standard deviation of the series (0.0 for a single
    /// point).
    pub fn stddev(&self) -> Option<f64> {
        let mean = self.mean()?;
        if self.0.len() < 2 {
            return Some(0.0);
        }
        let var = self.0.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / self.0.len() as f64;
        Some(var.sqrt())
    }
}

/// Async store of golden metric values keyed by `(suite, metric)`.
///
/// The history is what overfitting gates compare a fresh run against, so
/// it must be durable and queryable. [`InMemoryMetricHistoryStore`] is
/// for tests; [`sql::SqlMetricHistoryStore`] (behind the `sql` feature)
/// persists to SQLite / Postgres.
#[async_trait::async_trait]
pub trait MetricHistoryStore: Send + Sync {
    /// Load the prior golden values for `(suite, metric)`, oldest first.
    async fn golden(&self, suite: &str, metric: &str) -> Result<Vec<f64>>;
    /// Append an observed value for `(suite, metric)`.
    async fn record(&self, suite: &str, metric: &str, value: f64) -> Result<()>;
}

/// In-memory [`MetricHistoryStore`] for tests and ephemeral runs.
#[derive(Default)]
pub struct InMemoryMetricHistoryStore {
    inner: parking_lot::Mutex<std::collections::HashMap<(String, String), Vec<f64>>>,
}

impl InMemoryMetricHistoryStore {
    /// New empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl MetricHistoryStore for InMemoryMetricHistoryStore {
    async fn golden(&self, suite: &str, metric: &str) -> Result<Vec<f64>> {
        Ok(self
            .inner
            .lock()
            .get(&(suite.to_string(), metric.to_string()))
            .cloned()
            .unwrap_or_default())
    }

    async fn record(&self, suite: &str, metric: &str, value: f64) -> Result<()> {
        self.inner
            .lock()
            .entry((suite.to_string(), metric.to_string()))
            .or_default()
            .push(value);
        Ok(())
    }
}

/// Detects regression of a metric versus its historical golden series.
///
/// On each fresh run the new value (read from `actual[metric]`) is
/// compared to the baseline reference (the series' last value for
/// [`Tolerance::Abs`]/[`Tolerance::Pct`], or its mean for
/// [`Tolerance::ZScore`]). A move in the *bad* direction (per
/// [`Direction`]) that exceeds tolerance fails the gate; an improvement
/// always passes.
pub struct TimeSeriesRegressionGate {
    /// Key into the `actual` metrics object.
    pub metric: String,
    /// Prior approved values.
    pub baseline: GoldenSeries,
    /// How far a regression may go before it's blocked.
    pub tolerance: Tolerance,
    /// Which direction counts as a regression.
    pub direction: Direction,
}

impl TimeSeriesRegressionGate {
    /// Construct directly from an in-memory baseline.
    pub fn new(
        metric: impl Into<String>,
        baseline: GoldenSeries,
        tolerance: Tolerance,
        direction: Direction,
    ) -> Self {
        Self {
            metric: metric.into(),
            baseline,
            tolerance,
            direction,
        }
    }

    /// Construct by loading the baseline from a [`MetricHistoryStore`].
    ///
    /// `Scorer::score` is synchronous, so the (async) history load is
    /// done here at construction time; the resulting gate then scores
    /// synchronously.
    pub async fn from_store(
        store: &dyn MetricHistoryStore,
        suite: &str,
        metric: impl Into<String>,
        tolerance: Tolerance,
        direction: Direction,
    ) -> Result<Self> {
        let metric = metric.into();
        let baseline = GoldenSeries(store.golden(suite, &metric).await?);
        Ok(Self {
            metric,
            baseline,
            tolerance,
            direction,
        })
    }

    /// The reference value the new observation is compared against.
    fn reference(&self) -> Option<f64> {
        match self.tolerance {
            Tolerance::ZScore(_) => self.baseline.mean(),
            _ => self.baseline.last().or_else(|| self.baseline.mean()),
        }
    }

    /// Signed regression magnitude: positive means "worse than
    /// reference" in the configured direction.
    fn regression_amount(&self, value: f64, reference: f64) -> f64 {
        match self.direction {
            // A drop below reference is a regression.
            Direction::HigherIsBetter => reference - value,
            // A rise above reference is a regression.
            Direction::LowerIsBetter => value - reference,
        }
    }

    /// Returns `Some(true)` if the regression breaches tolerance,
    /// `Some(false)` if within tolerance, `None` if undecidable (no
    /// baseline / undefined z-score).
    fn breaches(&self, value: f64) -> Option<bool> {
        let reference = self.reference()?;
        let regression = self.regression_amount(value, reference);
        if regression <= 0.0 {
            // Improvement (or equal) never regresses.
            return Some(false);
        }
        match self.tolerance {
            Tolerance::Abs(t) => Some(regression > t),
            Tolerance::Pct(t) => {
                let denom = reference.abs();
                if denom == 0.0 {
                    // Any positive regression off a zero reference is
                    // unbounded in pct terms => breach.
                    Some(regression > 0.0)
                } else {
                    Some(regression / denom > t)
                }
            }
            Tolerance::ZScore(t) => {
                let sd = self.baseline.stddev()?;
                if sd == 0.0 {
                    // No variance: any regression is "infinitely" many
                    // sigmas => breach.
                    Some(regression > 0.0)
                } else {
                    Some(regression / sd > t)
                }
            }
        }
    }
}

impl Scorer for TimeSeriesRegressionGate {
    fn score(&self, _expected: &Value, actual: &Value) -> ScorerOutcome {
        let Some(value) = read_metric(actual, &self.metric) else {
            return ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!("metric {:?} missing or non-numeric in actual", self.metric),
            };
        };
        match self.breaches(value) {
            Some(true) => ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!(
                    "metric {:?} regressed: value {} vs ref {:?} beyond {:?} ({:?})",
                    self.metric,
                    value,
                    self.reference(),
                    self.tolerance,
                    self.direction
                ),
            },
            Some(false) => ScorerOutcome {
                passed: true,
                score: 1.0,
                note: format!(
                    "metric {:?} = {} within {:?} of ref {:?}",
                    self.metric,
                    value,
                    self.tolerance,
                    self.reference()
                ),
            },
            None => ScorerOutcome {
                // No baseline yet: nothing to regress against; treat as
                // pass (first run establishes the golden series).
                passed: true,
                score: 1.0,
                note: format!(
                    "metric {:?} = {} has no baseline; accepting as first golden",
                    self.metric, value
                ),
            },
        }
    }
}

/// Map a backend display error onto an `AgentError`.
#[cfg(feature = "sql")]
fn backend_err<E: std::fmt::Display>(e: E) -> atomr_agents_core::AgentError {
    atomr_agents_core::AgentError::Internal(format!("SqlMetricHistoryStore backend error: {e}"))
}

/// SQL-backed [`MetricHistoryStore`] (SQLite / Postgres via sqlx `any`).
///
/// Mirrors the backend pattern in `crates/state/src/backends.rs`: one
/// code path targets both dialects via the `any` driver; the dialect is
/// auto-detected from the connection URL. Golden values are stored in a
/// flat `eval_metric_history(suite, metric, value, ts)` table so they're
/// independently queryable for cost/overfitting audits.
#[cfg(feature = "sql")]
pub mod sql {
    use super::{backend_err, MetricHistoryStore};
    use atomr_agents_core::Result;
    use atomr_persistence_sql::SqlConfig;
    use sqlx::any::AnyPoolOptions;
    use sqlx::AnyPool;

    /// SQL-backed metric history store.
    pub struct SqlMetricHistoryStore {
        pool: AnyPool,
        // Monotonic insertion sequence so ordering is stable even when
        // multiple records land in the same millisecond. Dialect-neutral
        // (avoids SQLite-only `rowid` / Postgres serial differences).
        seq: std::sync::atomic::AtomicI64,
    }

    impl SqlMetricHistoryStore {
        /// Connect using a URL (e.g. `sqlite::memory:`,
        /// `postgres://user:pass@host/db`). The dialect is detected from
        /// the URL; the `eval_metric_history` table is created if absent.
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Self::connect_with(SqlConfig::new(url)).await
        }

        /// Connect from environment (see
        /// [`SqlConfig::from_env`](atomr_persistence_sql::SqlConfig::from_env)).
        pub async fn from_env() -> Result<Self> {
            Self::connect_with(SqlConfig::from_env()).await
        }

        /// Connect using an explicit [`SqlConfig`].
        pub async fn connect_with(cfg: SqlConfig) -> Result<Self> {
            sqlx::any::install_default_drivers();
            let pool = AnyPoolOptions::new()
                .max_connections(cfg.max_connections)
                .connect(&cfg.url)
                .await
                .map_err(backend_err)?;
            Self::from_pool(pool).await
        }

        /// Build from an existing pool (e.g. shared with other stores).
        pub async fn from_pool(pool: AnyPool) -> Result<Self> {
            let this = Self {
                pool,
                seq: std::sync::atomic::AtomicI64::new(0),
            };
            this.ensure_schema().await?;
            this.init_seq().await?;
            Ok(this)
        }

        async fn ensure_schema(&self) -> Result<()> {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS eval_metric_history (\
                   suite  TEXT             NOT NULL, \
                   metric TEXT             NOT NULL, \
                   value  DOUBLE PRECISION NOT NULL, \
                   ts     BIGINT           NOT NULL, \
                   seq    BIGINT           NOT NULL)",
            )
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }

        // Resume the in-process sequence past any rows already persisted
        // (e.g. when reconnecting to an existing database).
        async fn init_seq(&self) -> Result<()> {
            let row: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT MAX(seq) FROM eval_metric_history")
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(backend_err)?;
            let max = row.and_then(|(m,)| m).unwrap_or(0);
            self.seq.store(max, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl MetricHistoryStore for SqlMetricHistoryStore {
        async fn golden(&self, suite: &str, metric: &str) -> Result<Vec<f64>> {
            // Oldest first (by insertion sequence) so the last element is
            // the most recent golden. `seq` is dialect-neutral.
            let rows: Vec<(f64,)> = sqlx::query_as(
                "SELECT value FROM eval_metric_history \
                 WHERE suite = ? AND metric = ? ORDER BY seq ASC",
            )
            .bind(suite)
            .bind(metric)
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(rows.into_iter().map(|(v,)| v).collect())
        }

        async fn record(&self, suite: &str, metric: &str, value: f64) -> Result<()> {
            let ts = chrono::Utc::now().timestamp_millis();
            let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            sqlx::query(
                "INSERT INTO eval_metric_history (suite, metric, value, ts, seq) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(suite)
            .bind(metric)
            .bind(value)
            .bind(ts)
            .bind(seq)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Runs in-process against SQLite — no external service needed.
        #[tokio::test]
        async fn sqlite_record_and_golden_roundtrip() {
            let store = SqlMetricHistoryStore::connect("sqlite::memory:")
                .await
                .unwrap();
            assert!(store.golden("s", "sharpe").await.unwrap().is_empty());

            store.record("s", "sharpe", 1.0).await.unwrap();
            store.record("s", "sharpe", 2.0).await.unwrap();
            store.record("s", "drawdown", 0.1).await.unwrap();

            let g = store.golden("s", "sharpe").await.unwrap();
            assert_eq!(g, vec![1.0, 2.0]);
            assert_eq!(store.golden("s", "drawdown").await.unwrap(), vec![0.1]);
            assert!(store.golden("s", "missing").await.unwrap().is_empty());
        }

        #[tokio::test]
        async fn sqlite_gate_from_store_uses_history() {
            use crate::metric::{Direction, Scorer, Tolerance, TimeSeriesRegressionGate};
            let store = SqlMetricHistoryStore::connect("sqlite::memory:")
                .await
                .unwrap();
            store.record("bt", "sharpe", 2.0).await.unwrap();
            let gate = TimeSeriesRegressionGate::from_store(
                &store,
                "bt",
                "sharpe",
                Tolerance::Abs(0.2),
                Direction::HigherIsBetter,
            )
            .await
            .unwrap();
            // 2.0 -> 1.5 is a 0.5 drop, beyond 0.2 abs tolerance.
            let out = Scorer::score(&gate, &serde_json::json!({}), &serde_json::json!({"sharpe": 1.5}));
            assert!(!out.passed);
        }

        // Postgres path: same code, exercised only when a server is set.
        #[tokio::test]
        async fn postgres_roundtrip_when_configured() {
            let Ok(url) = std::env::var("ATOMR_IT_SQL_URL") else {
                eprintln!("skipping: ATOMR_IT_SQL_URL not set");
                return;
            };
            if !url.starts_with("postgres") {
                return;
            }
            let store = SqlMetricHistoryStore::connect(url).await.unwrap();
            let suite = format!("it-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
            store.record(&suite, "sharpe", 1.5).await.unwrap();
            store.record(&suite, "sharpe", 1.6).await.unwrap();
            let g = store.golden(&suite, "sharpe").await.unwrap();
            assert_eq!(g, vec![1.5, 1.6]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn threshold_evaluate_all_variants() {
        assert!(Threshold::AtLeast(1.0).evaluate(1.0));
        assert!(!Threshold::AtLeast(1.0).evaluate(0.9));
        assert!(Threshold::AtMost(1.0).evaluate(1.0));
        assert!(!Threshold::AtMost(1.0).evaluate(1.1));
        assert!(Threshold::Between(0.0, 1.0).evaluate(0.5));
        assert!(!Threshold::Between(0.0, 1.0).evaluate(1.5));
        assert!(Threshold::Outside(0.0, 1.0).evaluate(2.0));
        assert!(!Threshold::Outside(0.0, 1.0).evaluate(0.5));
    }

    #[test]
    fn metric_scorer_pass_and_fail() {
        let s = MetricScorer::new("sharpe", Threshold::AtLeast(1.0));
        let pass = s.score(&json!({}), &json!({"sharpe": 1.8}));
        assert!(pass.passed);
        assert_eq!(pass.score, 1.0);

        let fail = s.score(&json!({}), &json!({"sharpe": 0.4}));
        assert!(!fail.passed);
        assert_eq!(fail.score, 0.0);
    }

    #[test]
    fn metric_scorer_missing_metric_fails() {
        let s = MetricScorer::new("sharpe", Threshold::AtLeast(1.0));
        let out = s.score(&json!({}), &json!({"other": 1.0}));
        assert!(!out.passed);
        assert!(out.note.contains("missing"));

        // Non-numeric value.
        let out2 = s.score(&json!({}), &json!({"sharpe": "high"}));
        assert!(!out2.passed);
    }

    #[test]
    fn golden_series_stats() {
        let g = GoldenSeries(vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert_eq!(g.mean(), Some(5.0));
        assert_eq!(g.stddev(), Some(2.0));
        assert_eq!(g.last(), Some(9.0));
        assert!(GoldenSeries::default().mean().is_none());
        assert_eq!(GoldenSeries(vec![3.0]).stddev(), Some(0.0));
    }

    #[test]
    fn regression_abs_higher_is_better() {
        let gate = TimeSeriesRegressionGate::new(
            "sharpe",
            GoldenSeries(vec![2.0]),
            Tolerance::Abs(0.3),
            Direction::HigherIsBetter,
        );
        // Drop of 0.5 > 0.3 => regress.
        assert!(!gate.score(&json!({}), &json!({"sharpe": 1.5})).passed);
        // Drop of 0.2 < 0.3 => ok.
        assert!(gate.score(&json!({}), &json!({"sharpe": 1.8})).passed);
        // Improvement => ok.
        assert!(gate.score(&json!({}), &json!({"sharpe": 3.0})).passed);
    }

    #[test]
    fn regression_pct_lower_is_better() {
        // drawdown lower is better; baseline 0.10, 10% pct tolerance.
        let gate = TimeSeriesRegressionGate::new(
            "drawdown",
            GoldenSeries(vec![0.10]),
            Tolerance::Pct(0.10),
            Direction::LowerIsBetter,
        );
        // Rise to 0.12 => +0.02 / 0.10 = 20% > 10% => regress.
        assert!(!gate.score(&json!({}), &json!({"drawdown": 0.12})).passed);
        // Rise to 0.105 => 5% <= 10% => ok.
        assert!(gate.score(&json!({}), &json!({"drawdown": 0.105})).passed);
        // Lower drawdown => improvement => ok.
        assert!(gate.score(&json!({}), &json!({"drawdown": 0.05})).passed);
    }

    #[test]
    fn regression_zscore() {
        // mean 5, stddev 2 (from the series above), higher is better.
        let series = GoldenSeries(vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        let gate = TimeSeriesRegressionGate::new(
            "sharpe",
            series,
            Tolerance::ZScore(1.0),
            Direction::HigherIsBetter,
        );
        // value 2.0 -> drop of 3.0 from mean 5 => 1.5 sigma > 1.0 => regress.
        assert!(!gate.score(&json!({}), &json!({"sharpe": 2.0})).passed);
        // value 4.0 -> drop of 1.0 => 0.5 sigma <= 1.0 => ok.
        assert!(gate.score(&json!({}), &json!({"sharpe": 4.0})).passed);
    }

    #[test]
    fn regression_no_baseline_accepts_first() {
        let gate = TimeSeriesRegressionGate::new(
            "sharpe",
            GoldenSeries::default(),
            Tolerance::Abs(0.1),
            Direction::HigherIsBetter,
        );
        let out = gate.score(&json!({}), &json!({"sharpe": 1.0}));
        assert!(out.passed);
    }

    #[test]
    fn regression_missing_metric_fails() {
        let gate = TimeSeriesRegressionGate::new(
            "sharpe",
            GoldenSeries(vec![2.0]),
            Tolerance::Abs(0.1),
            Direction::HigherIsBetter,
        );
        let out = gate.score(&json!({}), &json!({"other": 1.0}));
        assert!(!out.passed);
        assert!(out.note.contains("missing"));
    }

    #[tokio::test]
    async fn in_memory_store_roundtrip_and_from_store() {
        let store = InMemoryMetricHistoryStore::new();
        store.record("bt", "sharpe", 2.0).await.unwrap();
        store.record("bt", "sharpe", 2.1).await.unwrap();
        assert_eq!(store.golden("bt", "sharpe").await.unwrap(), vec![2.0, 2.1]);

        let gate = TimeSeriesRegressionGate::from_store(
            &store,
            "bt",
            "sharpe",
            Tolerance::Abs(0.3),
            Direction::HigherIsBetter,
        )
        .await
        .unwrap();
        // baseline last = 2.1; value 1.5 => drop 0.6 > 0.3 => regress.
        assert!(!gate.score(&json!({}), &json!({"sharpe": 1.5})).passed);
    }
}
