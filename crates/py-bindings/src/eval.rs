//! Evaluation surface: cases, scorers, suites, annotation queues, and
//! regression gates.
//!
//! The Rust crate exposes a small synchronous `Scorer` trait whose
//! `score()` method computes a [`ScorerOutcome`] from an `(expected,
//! actual)` JSON pair, plus async `JudgeModel` / `AnnotationQueue`
//! traits used by the LLM-judge and human-in-the-loop scorers.
//!
//! Python parity surface:
//!
//! - Value classes: [`PyPairwiseChoice`], [`PyVerdict`],
//!   [`PyRubricCriterion`], [`PyJudgeModel`], [`PyEvalCase`],
//!   [`PyEvalResult`], [`PyEvalRun`], [`PyScorerOutcome`],
//!   [`PyAnnotationItem`], [`PyRegressionResult`].
//! - Dyn handles: [`PyScorer`] (wraps `Arc<dyn Scorer>`),
//!   [`PyAnnotationQueue`] (wraps `Arc<dyn AnnotationQueue>`).
//! - Builders: `rubric_scorer`, `llm_judge_scorer`, `pairwise_scorer`,
//!   `scorer_from_factory`, `in_memory_annotation_queue`,
//!   `regression_gate`.
//! - Suite runner: [`PyEvalSuite`].
//!
//! The `JudgeModel` trait does not have a default in-process
//! implementation; callers must either supply a Python callable via
//! [`PyJudgeModel::from_callable`] or register a guest factory and
//! resolve via [`PyJudgeModel::from_factory`]. A bare
//! [`PyJudgeModel::new(name)`] is a *named placeholder* and any scorer
//! built on top of it will raise `NotImplementedError` on first use.

use std::sync::Arc;

use async_trait::async_trait;
#[allow(unused_imports)]
use atomr_agents_callable::Callable;
use atomr_agents_callable::FnCallable;
use atomr_agents_core::{AgentError, Result as AgentResult, RunId, Value};
use atomr_agents_eval::{
    AnnotationItem, AnnotationQueue, EvalCase, EvalResult, EvalRun, EvalSuite,
    InMemoryAnnotationQueue, JudgeModel, LlmJudgeScorer, PairwiseChoice, PairwiseScorer,
    RegressionGate, RegressionResult, RubricCriterion, RubricScorer, Scorer, ScorerOutcome,
    Verdict,
};
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;

use crate::callable::PyCallable;
use crate::conv::{json_to_py, py_to_json};
use crate::errors;
use crate::strategy::await_if_coro;

// ============================================================================
// Enum-shaped value classes
// ============================================================================

#[pyclass(name = "PairwiseChoice", module = "atomr_agents._native.eval", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyPairwiseChoice {
    inner: String,
}

#[pymethods]
impl PyPairwiseChoice {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let valid = ["a", "b", "tie"];
        if !valid.contains(&name) {
            return Err(PyValueError::new_err(format!(
                "unknown pairwise choice: {name:?}"
            )));
        }
        Ok(Self {
            inner: name.to_string(),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner
    }

    #[staticmethod]
    fn a() -> Self {
        Self {
            inner: "a".to_string(),
        }
    }
    #[staticmethod]
    fn b() -> Self {
        Self {
            inner: "b".to_string(),
        }
    }
    #[staticmethod]
    fn tie() -> Self {
        Self {
            inner: "tie".to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("PairwiseChoice({:?})", self.inner)
    }
}

impl PyPairwiseChoice {
    pub(crate) fn from_inner(c: PairwiseChoice) -> Self {
        let s = match c {
            PairwiseChoice::A => "a",
            PairwiseChoice::B => "b",
            PairwiseChoice::Tie => "tie",
        };
        Self {
            inner: s.to_string(),
        }
    }
}

#[pyclass(name = "Verdict", module = "atomr_agents._native.eval", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyVerdict {
    inner: String,
}

#[pymethods]
impl PyVerdict {
    /// Construct from a snake_case name. Accepts the canonical crate
    /// names (`pending`, `approved`, `rejected`, `needs_edit`) and the
    /// historical alias `needs_review` (mapped to `needs_edit`).
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let normalized = match name {
            "pending" | "approved" | "rejected" | "needs_edit" => name.to_string(),
            "needs_review" => "needs_edit".to_string(),
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown verdict: {other:?}"
                )))
            }
        };
        Ok(Self { inner: normalized })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner
    }

    #[staticmethod]
    fn pending() -> Self {
        Self {
            inner: "pending".into(),
        }
    }
    #[staticmethod]
    fn approved() -> Self {
        Self {
            inner: "approved".into(),
        }
    }
    #[staticmethod]
    fn rejected() -> Self {
        Self {
            inner: "rejected".into(),
        }
    }
    #[staticmethod]
    fn needs_edit() -> Self {
        Self {
            inner: "needs_edit".into(),
        }
    }

    fn __repr__(&self) -> String {
        format!("Verdict({:?})", self.inner)
    }
}

impl PyVerdict {
    pub(crate) fn into_inner(&self) -> Verdict {
        match self.inner.as_str() {
            "pending" => Verdict::Pending,
            "approved" => Verdict::Approved,
            "rejected" => Verdict::Rejected,
            "needs_edit" => Verdict::NeedsEdit,
            _ => Verdict::Pending,
        }
    }

    pub(crate) fn from_inner(v: Verdict) -> Self {
        let s = match v {
            Verdict::Pending => "pending",
            Verdict::Approved => "approved",
            Verdict::Rejected => "rejected",
            Verdict::NeedsEdit => "needs_edit",
        };
        Self {
            inner: s.to_string(),
        }
    }
}

// ============================================================================
// PyRubricCriterion — value class
// ============================================================================

#[pyclass(name = "RubricCriterion", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyRubricCriterion {
    pub(crate) inner: RubricCriterion,
}

#[pymethods]
impl PyRubricCriterion {
    #[new]
    #[pyo3(signature = (name, description, weight=1.0))]
    fn new(name: String, description: String, weight: f32) -> Self {
        Self {
            inner: RubricCriterion {
                name,
                description,
                weight,
            },
        }
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }
    #[getter]
    fn description(&self) -> &str {
        &self.inner.description
    }
    #[getter]
    fn weight(&self) -> f32 {
        self.inner.weight
    }

    fn __repr__(&self) -> String {
        format!(
            "RubricCriterion(name={:?}, weight={})",
            self.inner.name, self.inner.weight
        )
    }
}

// ============================================================================
// PyJudgeModel — dyn handle around `Arc<dyn JudgeModel>`
// ============================================================================

/// Python-visible handle wrapping a `JudgeModel`. The handle may be a
/// *named placeholder* (no backing implementation), an adapter over a
/// Python callable, or a guest-factory-resolved object.
///
/// A named placeholder is allowed at construction time so callers can
/// declare scorer pipelines declaratively; the first attempt to *use*
/// a placeholder judge raises `NotImplementedError`.
#[pyclass(name = "JudgeModel", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyJudgeModel {
    pub(crate) name: String,
    pub(crate) inner: Option<Arc<dyn JudgeModel>>,
}

#[pymethods]
impl PyJudgeModel {
    /// Construct a *named placeholder* judge model. Any scorer built
    /// on this judge raises `NotImplementedError` when invoked. Use
    /// [`from_callable`] or [`from_factory`] to attach a real
    /// implementation.
    #[new]
    fn new(name: String) -> Self {
        Self { name, inner: None }
    }

    /// Build a `JudgeModel` from a Python `async def judge(prompt) ->
    /// str` (or sync callable returning a string).
    #[staticmethod]
    #[pyo3(signature = (target, name=None))]
    fn from_callable(target: PyObject, name: Option<String>) -> Self {
        let label = name.unwrap_or_else(|| "py_judge".to_string());
        let adapter = PyJudgeModelAdapter {
            target: Arc::new(target),
        };
        Self {
            name: label,
            inner: Some(Arc::new(adapter)),
        }
    }

    /// Resolve a guest-registered judge model by key. Equivalent to
    /// `from_callable(guest.lookup("judge_model", key))`. We re-use
    /// the generic `scorer` guest kind since `JudgeModel` is a
    /// scorer-flavoured dependency.
    #[staticmethod]
    fn from_factory(key: String) -> PyResult<Self> {
        // We piggy-back on the existing "scorer" guest namespace —
        // judge models are typically registered alongside scorers.
        let target = crate::guest::must_lookup("scorer", &key)
            .or_else(|_| crate::guest::must_lookup("judge_model", &key))?;
        let adapter = PyJudgeModelAdapter { target };
        Ok(Self {
            name: format!("guest:{key}"),
            inner: Some(Arc::new(adapter)),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    fn __repr__(&self) -> String {
        let kind = if self.inner.is_some() { "bound" } else { "placeholder" };
        format!("JudgeModel(name={:?}, {kind})", self.name)
    }
}

impl PyJudgeModel {
    fn require_inner(&self) -> PyResult<Arc<dyn JudgeModel>> {
        self.inner.clone().ok_or_else(|| {
            PyNotImplementedError::new_err(format!(
                "JudgeModel({:?}) is a placeholder; bind a real implementation via \
                 JudgeModel.from_callable(...) or JudgeModel.from_factory(...)",
                self.name
            ))
        })
    }
}

/// Adapter wrapping a Python callable as a Rust `JudgeModel`.
pub(crate) struct PyJudgeModelAdapter {
    pub(crate) target: Arc<PyObject>,
}

#[async_trait]
impl JudgeModel for PyJudgeModelAdapter {
    async fn judge(&self, prompt: &str) -> AgentResult<String> {
        let target = self.target.clone();
        let prompt = prompt.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("judge")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.clone()
            } else if bound.hasattr("__class__")? {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = if instance.hasattr("judge")? {
                instance.getattr("judge")?.call1((prompt,))?
            } else {
                instance.call1((prompt,))?
            };
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py judge: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<String>())
            .map_err(|e| AgentError::Internal(format!("py judge result: {e}")))
    }
}

// ============================================================================
// PyEvalCase / PyEvalResult / PyEvalRun / PyScorerOutcome — value classes
// ============================================================================

#[pyclass(name = "EvalCase", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyEvalCase {
    pub(crate) inner: EvalCase,
}

#[pymethods]
impl PyEvalCase {
    #[new]
    fn new(py: Python<'_>, id: String, input: &Bound<'_, PyAny>, expected: &Bound<'_, PyAny>) -> PyResult<Self> {
        let input_v = py_to_json(py, input)?;
        let expected_v = py_to_json(py, expected)?;
        Ok(Self {
            inner: EvalCase {
                id,
                input: input_v,
                expected: expected_v,
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }
    #[getter]
    fn input(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.input)
    }
    #[getter]
    fn expected(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.expected)
    }

    fn __repr__(&self) -> String {
        format!("EvalCase(id={:?})", self.inner.id)
    }
}

#[pyclass(name = "ScorerOutcome", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyScorerOutcome {
    pub(crate) inner: ScorerOutcome,
}

#[pymethods]
impl PyScorerOutcome {
    #[new]
    #[pyo3(signature = (passed, score, note=String::new()))]
    fn new(passed: bool, score: f32, note: String) -> Self {
        Self {
            inner: ScorerOutcome {
                passed,
                score,
                note,
            },
        }
    }

    #[getter]
    fn passed(&self) -> bool {
        self.inner.passed
    }
    #[getter]
    fn score(&self) -> f32 {
        self.inner.score
    }
    #[getter]
    fn note(&self) -> &str {
        &self.inner.note
    }

    fn __repr__(&self) -> String {
        format!(
            "ScorerOutcome(passed={}, score={:.3}, note={:?})",
            self.inner.passed, self.inner.score, self.inner.note
        )
    }
}

#[pyclass(name = "EvalResult", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyEvalResult {
    pub(crate) inner: EvalResult,
}

#[pymethods]
impl PyEvalResult {
    #[getter]
    fn case_id(&self) -> &str {
        &self.inner.case_id
    }
    #[getter]
    fn outcome(&self) -> PyScorerOutcome {
        PyScorerOutcome {
            inner: self.inner.outcome.clone(),
        }
    }
    #[getter]
    fn elapsed_ms(&self) -> u64 {
        self.inner.elapsed_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "EvalResult(case_id={:?}, passed={}, score={:.3}, elapsed_ms={})",
            self.inner.case_id, self.inner.outcome.passed, self.inner.outcome.score, self.inner.elapsed_ms
        )
    }
}

#[pyclass(name = "EvalRun", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyEvalRun {
    pub(crate) inner: EvalRun,
}

#[pymethods]
impl PyEvalRun {
    #[getter]
    fn passed(&self) -> u32 {
        self.inner.passed
    }
    #[getter]
    fn failed(&self) -> u32 {
        self.inner.failed
    }
    #[getter]
    fn avg_score(&self) -> f32 {
        self.inner.avg_score
    }
    #[getter]
    fn results(&self) -> Vec<PyEvalResult> {
        self.inner
            .results
            .iter()
            .map(|r| PyEvalResult { inner: r.clone() })
            .collect()
    }

    fn pass_rate(&self) -> f32 {
        self.inner.pass_rate()
    }

    fn __repr__(&self) -> String {
        format!(
            "EvalRun(passed={}, failed={}, avg_score={:.3})",
            self.inner.passed, self.inner.failed, self.inner.avg_score
        )
    }
}

// ============================================================================
// PyScorer — dyn handle wrapping `Arc<dyn Scorer>`
// ============================================================================

#[pyclass(name = "Scorer", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyScorer {
    pub(crate) inner: Arc<dyn Scorer>,
}

#[pymethods]
impl PyScorer {
    /// `await scorer.score(expected, actual) -> ScorerOutcome`.
    ///
    /// The underlying `Scorer::score` is synchronous, but we expose an
    /// awaitable to keep the Python surface uniform with other async
    /// adapters. The blocking call is dispatched on
    /// `tokio::task::spawn_blocking` so a containing event loop is
    /// not held while a judge model runs.
    fn score<'py>(
        &self,
        py: Python<'py>,
        expected: &Bound<'_, PyAny>,
        actual: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let expected_v = py_to_json(py, expected)?;
        let actual_v = py_to_json(py, actual)?;
        let scorer = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let outcome = tokio::task::spawn_blocking(move || scorer.score(&expected_v, &actual_v))
                .await
                .map_err(|e| {
                    PyErr::new::<errors::EvalError, _>(format!("scorer task: {e}"))
                })?;
            Ok(PyScorerOutcome { inner: outcome })
        })
    }

    /// Synchronous variant for scripting / REPL use.
    fn score_sync(
        &self,
        py: Python<'_>,
        expected: &Bound<'_, PyAny>,
        actual: &Bound<'_, PyAny>,
    ) -> PyResult<PyScorerOutcome> {
        let expected_v = py_to_json(py, expected)?;
        let actual_v = py_to_json(py, actual)?;
        let scorer = self.inner.clone();
        let outcome = py.allow_threads(|| scorer.score(&expected_v, &actual_v));
        Ok(PyScorerOutcome { inner: outcome })
    }

    fn __repr__(&self) -> String {
        "Scorer(handle)".into()
    }
}

/// Adapter wrapping a Python target as a Rust `Scorer`. The Python
/// target's `score(expected, actual)` may return either a
/// `ScorerOutcome`-shaped dict (`{"passed": bool, "score": float,
/// "note": str}`) or a `PyScorerOutcome` instance. Coroutines are
/// awaited via the shared tokio runtime.
pub(crate) struct PyScorerAdapter {
    pub(crate) target: Arc<PyObject>,
}

impl Scorer for PyScorerAdapter {
    fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        let target = self.target.clone();
        let expected = expected.clone();
        let actual = actual.clone();

        // First, sync-invoke the python side. If a coroutine comes
        // back, we await it on the shared runtime.
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let exp_py = json_to_py(py, &expected)?;
            let act_py = json_to_py(py, &actual)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("score")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.clone()
            } else if bound.hasattr("__class__")? {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = if instance.hasattr("score")? {
                instance.getattr("score")?.call1((exp_py, act_py))?
            } else {
                instance.call1((exp_py, act_py))?
            };
            Ok(r.unbind())
        });

        let coro_or_val = match coro_or_val {
            Ok(v) => v,
            Err(e) => {
                return ScorerOutcome {
                    passed: false,
                    score: 0.0,
                    note: format!("py scorer error: {e}"),
                };
            }
        };

        // Resolve a possible coroutine. We're called from a sync
        // context that may or may not be inside a tokio runtime —
        // mirror the pattern used by LlmJudgeScorer.
        let final_val = tokio::task::block_in_place(|| match tokio::runtime::Handle::try_current() {
            Ok(h) => h.block_on(await_if_coro(coro_or_val)),
            Err(_) => crate::runtime::shared().block_on(await_if_coro(coro_or_val)),
        });
        let final_val = match final_val {
            Ok(v) => v,
            Err(e) => {
                return ScorerOutcome {
                    passed: false,
                    score: 0.0,
                    note: format!("py scorer await: {e}"),
                };
            }
        };

        // Try a few result shapes: PyScorerOutcome, dict, or {bool}.
        let parsed = Python::with_gil(|py| -> PyResult<ScorerOutcome> {
            let bound = final_val.bind(py);
            if let Ok(outcome) = bound.extract::<PyScorerOutcome>() {
                return Ok(outcome.inner);
            }
            let v = py_to_json(py, bound)?;
            if let Some(obj) = v.as_object() {
                let passed = obj.get("passed").and_then(|x| x.as_bool()).unwrap_or(false);
                let score = obj
                    .get("score")
                    .and_then(|x| x.as_f64())
                    .map(|x| x as f32)
                    .unwrap_or(if passed { 1.0 } else { 0.0 });
                let note = obj
                    .get("note")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(ScorerOutcome {
                    passed,
                    score,
                    note,
                });
            }
            if let Some(b) = v.as_bool() {
                return Ok(ScorerOutcome {
                    passed: b,
                    score: if b { 1.0 } else { 0.0 },
                    note: String::new(),
                });
            }
            Ok(ScorerOutcome {
                passed: false,
                score: 0.0,
                note: format!("unrecognized scorer result: {v}"),
            })
        });

        parsed.unwrap_or_else(|e| ScorerOutcome {
            passed: false,
            score: 0.0,
            note: format!("py scorer parse: {e}"),
        })
    }
}

// ----- Scorer factories ----------------------------------------------------

/// Build a [`RubricScorer`] from a list of criteria. Requires a real
/// [`JudgeModel`]; passing a placeholder raises
/// `NotImplementedError`.
#[pyfunction]
#[pyo3(signature = (judge, criteria, pass_at=0.6))]
fn rubric_scorer(
    judge: PyJudgeModel,
    criteria: Vec<PyRubricCriterion>,
    pass_at: f32,
) -> PyResult<PyScorer> {
    let model = judge.require_inner()?;
    let scorer = RubricScorer {
        model,
        criteria: criteria.into_iter().map(|c| c.inner).collect(),
        pass_at,
    };
    Ok(PyScorer {
        inner: Arc::new(scorer),
    })
}

/// Build an [`LlmJudgeScorer`]. The crate's constructor only takes the
/// `JudgeModel`; an optional `prompt_template` override is wired
/// post-construction.
#[pyfunction]
#[pyo3(signature = (judge, prompt_template=None))]
fn llm_judge_scorer(judge: PyJudgeModel, prompt_template: Option<String>) -> PyResult<PyScorer> {
    let model = judge.require_inner()?;
    let mut scorer = LlmJudgeScorer::new(model);
    if let Some(t) = prompt_template {
        scorer.prompt_template = t;
    }
    Ok(PyScorer {
        inner: Arc::new(scorer),
    })
}

/// Build a [`PairwiseScorer`]. Note: `PairwiseScorer` is *not* itself
/// a `Scorer` — it exposes an async `compare()` API. To keep the
/// Python surface uniform we expose `pairwise_scorer` as a dedicated
/// builder returning a [`PyPairwiseScorer`] handle instead of a
/// generic `PyScorer`.
#[pyfunction]
#[pyo3(signature = (judge, criteria_label="helpfulness".to_string()))]
fn pairwise_scorer(judge: PyJudgeModel, criteria_label: String) -> PyResult<PyPairwiseScorer> {
    let model = judge.require_inner()?;
    Ok(PyPairwiseScorer {
        inner: Arc::new(PairwiseScorer::new(model, criteria_label)),
    })
}

/// Resolve a guest-registered scorer by key (registered via
/// `guest.register_scorer_factory`).
#[pyfunction]
fn scorer_from_factory(key: String) -> PyResult<PyScorer> {
    let target = crate::guest::must_lookup("scorer", &key)?;
    Ok(PyScorer {
        inner: Arc::new(PyScorerAdapter { target }),
    })
}

// ============================================================================
// PyPairwiseScorer — dedicated handle (PairwiseScorer is not a Scorer)
// ============================================================================

#[pyclass(name = "PairwiseScorer", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyPairwiseScorer {
    pub(crate) inner: Arc<PairwiseScorer>,
}

#[pymethods]
impl PyPairwiseScorer {
    /// `await scorer.compare(prompt, a, b) -> (PairwiseChoice, note)`.
    fn compare<'py>(
        &self,
        py: Python<'py>,
        prompt: String,
        a: &Bound<'_, PyAny>,
        b: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let a_v = py_to_json(py, a)?;
        let b_v = py_to_json(py, b)?;
        let scorer = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (choice, note) = scorer
                .compare(&prompt, &a_v, &b_v)
                .await
                .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
            Ok((PyPairwiseChoice::from_inner(choice), note))
        })
    }

    fn __repr__(&self) -> String {
        format!("PairwiseScorer(criteria={:?})", self.inner.criteria_label)
    }
}

// ============================================================================
// PyEvalSuite
// ============================================================================

/// Eval suite wrapping a list of cases plus *one* primary `Scorer`.
///
/// The Rust `EvalSuite` only supports a single scorer; if multiple
/// scorers are provided here we run each scorer over every case
/// sequentially and aggregate by averaging pass-rates. The first
/// scorer's outcome populates the case's `EvalResult.outcome` for
/// downstream consumers.
#[pyclass(name = "EvalSuite", module = "atomr_agents._native.eval")]
pub struct PyEvalSuite {
    name: String,
    cases: Vec<EvalCase>,
    scorers: Vec<Arc<dyn Scorer>>,
}

#[pymethods]
impl PyEvalSuite {
    #[new]
    fn new(name: String, cases: Vec<PyEvalCase>, scorers: Vec<PyScorer>) -> PyResult<Self> {
        if scorers.is_empty() {
            return Err(PyValueError::new_err(
                "EvalSuite requires at least one scorer",
            ));
        }
        Ok(Self {
            name,
            cases: cases.into_iter().map(|c| c.inner).collect(),
            scorers: scorers.into_iter().map(|s| s.inner).collect(),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    #[getter]
    fn cases(&self) -> Vec<PyEvalCase> {
        self.cases
            .iter()
            .map(|c| PyEvalCase { inner: c.clone() })
            .collect()
    }

    /// `await suite.run(target, ctx=None) -> EvalRun`.
    ///
    /// `target` must be a [`PyCallable`]. The suite calls
    /// `target.call(case.input, ctx)` for each case and feeds the
    /// returned value into every configured scorer.
    #[pyo3(signature = (target, ctx=None))]
    fn run<'py>(
        &self,
        py: Python<'py>,
        target: PyCallable,
        ctx: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let call_ctx = crate::conv::callctx_from_pydict(py, ctx)?;
        let suite_name = self.name.clone();
        let cases = self.cases.clone();
        let scorers = self.scorers.clone();
        let handle = target.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // Run each scorer as its own EvalSuite invocation to reuse
            // the crate's per-case timing / pass-rate accounting.
            let mut combined = EvalRun::default();
            let mut total_score = 0.0f32;
            let mut total_cases = 0u32;
            for (i, scorer) in scorers.iter().enumerate() {
                let suite = EvalSuite {
                    id: format!("{suite_name}#{i}"),
                    cases: cases.clone(),
                    scorer: scorer.clone(),
                };
                // Wrap the user's CallableHandle in an FnCallable that
                // forwards via a stored Arc, since EvalSuite::run takes
                // `&dyn Callable`.
                let handle = handle.clone();
                let call_ctx = call_ctx.clone();
                let forwarder = FnCallable::labeled("eval_target", move |v: Value, _ctx| {
                    let handle = handle.clone();
                    let call_ctx = call_ctx.clone();
                    async move { handle.call(v, call_ctx).await }
                });
                let run = suite
                    .run(&forwarder)
                    .await
                    .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
                combined.passed += run.passed;
                combined.failed += run.failed;
                total_score += run.avg_score * (run.passed + run.failed) as f32;
                total_cases += run.passed + run.failed;
                // Only keep the first scorer's per-case results so
                // EvalRun.results stays case-aligned. Additional
                // scorers contribute to the aggregate pass/fail.
                if i == 0 {
                    combined.results = run.results;
                }
            }
            combined.avg_score = if total_cases == 0 {
                0.0
            } else {
                total_score / total_cases as f32
            };
            Ok(PyEvalRun { inner: combined })
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "EvalSuite(name={:?}, cases={}, scorers={})",
            self.name,
            self.cases.len(),
            self.scorers.len()
        )
    }
}

// ============================================================================
// PyAnnotationItem + PyAnnotationQueue
// ============================================================================

#[pyclass(name = "AnnotationItem", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyAnnotationItem {
    pub(crate) inner: AnnotationItem,
}

#[pymethods]
impl PyAnnotationItem {
    #[new]
    #[pyo3(signature = (id, run_id, prompt, output, verdict=None, note=None, created_at_ms=0))]
    fn new(
        py: Python<'_>,
        id: String,
        run_id: String,
        prompt: String,
        output: &Bound<'_, PyAny>,
        verdict: Option<PyVerdict>,
        note: Option<String>,
        created_at_ms: i64,
    ) -> PyResult<Self> {
        let output_v = py_to_json(py, output)?;
        let verdict = verdict.map(|v| v.into_inner()).unwrap_or(Verdict::Pending);
        Ok(Self {
            inner: AnnotationItem {
                id,
                run_id: RunId::from(run_id),
                prompt,
                output: output_v,
                verdict,
                note,
                created_at_ms,
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }
    #[getter]
    fn run_id(&self) -> &str {
        self.inner.run_id.as_str()
    }
    #[getter]
    fn prompt(&self) -> &str {
        &self.inner.prompt
    }
    #[getter]
    fn output(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.output)
    }
    #[getter]
    fn verdict(&self) -> PyVerdict {
        PyVerdict::from_inner(self.inner.verdict)
    }
    #[getter]
    fn note(&self) -> Option<&str> {
        self.inner.note.as_deref()
    }
    #[getter]
    fn created_at_ms(&self) -> i64 {
        self.inner.created_at_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "AnnotationItem(id={:?}, verdict={:?})",
            self.inner.id, self.inner.verdict
        )
    }
}

#[pyclass(name = "AnnotationQueue", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyAnnotationQueue {
    pub(crate) inner: Arc<dyn AnnotationQueue>,
}

#[pymethods]
impl PyAnnotationQueue {
    /// `await queue.enqueue(item) -> None`.
    fn enqueue<'py>(&self, py: Python<'py>, item: PyAnnotationItem) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .enqueue(item.inner)
                .await
                .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
            Ok(())
        })
    }

    /// `await queue.next_pending() -> AnnotationItem | None`.
    fn next_pending<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let r = inner
                .next_pending()
                .await
                .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
            Ok(r.map(|item| PyAnnotationItem { inner: item }))
        })
    }

    /// `await queue.submit(id, verdict, note=None) -> None`.
    #[pyo3(signature = (id, verdict, note=None))]
    fn submit<'py>(
        &self,
        py: Python<'py>,
        id: String,
        verdict: PyVerdict,
        note: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let v = verdict.into_inner();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .submit(&id, v, note)
                .await
                .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
            Ok(())
        })
    }

    /// `await queue.list() -> list[AnnotationItem]`.
    fn list<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let items = inner
                .list()
                .await
                .map_err(|e: AgentError| PyErr::new::<errors::EvalError, _>(e.to_string()))?;
            Ok(items
                .into_iter()
                .map(|item| PyAnnotationItem { inner: item })
                .collect::<Vec<_>>())
        })
    }

    fn __repr__(&self) -> String {
        "AnnotationQueue(handle)".into()
    }
}

/// Build an in-memory annotation queue.
#[pyfunction]
fn in_memory_annotation_queue() -> PyAnnotationQueue {
    PyAnnotationQueue {
        inner: Arc::new(InMemoryAnnotationQueue::new()),
    }
}

// ============================================================================
// Regression gate
// ============================================================================

#[pyclass(name = "RegressionResult", module = "atomr_agents._native.eval")]
#[derive(Clone)]
pub struct PyRegressionResult {
    pub(crate) inner: RegressionResult,
}

#[pymethods]
impl PyRegressionResult {
    #[getter]
    fn baseline_pass_rate(&self) -> f32 {
        self.inner.baseline_pass_rate
    }
    #[getter]
    fn current_pass_rate(&self) -> f32 {
        self.inner.current_pass_rate
    }
    #[getter]
    fn delta(&self) -> f32 {
        self.inner.delta
    }
    #[getter]
    fn blocked(&self) -> bool {
        self.inner.blocked
    }
    #[getter]
    fn reason(&self) -> &str {
        &self.inner.reason
    }

    fn __repr__(&self) -> String {
        format!(
            "RegressionResult(blocked={}, delta={:.3}, reason={:?})",
            self.inner.blocked, self.inner.delta, self.inner.reason
        )
    }
}

/// Compare two [`PyEvalRun`]s, blocking when the current run regresses
/// past the configured tolerance.
#[pyfunction]
#[pyo3(signature = (baseline_run, current_run, tolerance=0.05))]
fn regression_gate(
    baseline_run: &PyEvalRun,
    current_run: &PyEvalRun,
    tolerance: f32,
) -> PyRegressionResult {
    let gate = RegressionGate { tolerance };
    PyRegressionResult {
        inner: gate.check(&baseline_run.inner, &current_run.inner),
    }
}

// ============================================================================
// Module registration
// ============================================================================

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "eval")?;

    // Value classes.
    m.add_class::<PyPairwiseChoice>()?;
    m.add_class::<PyVerdict>()?;
    m.add_class::<PyRubricCriterion>()?;
    m.add_class::<PyJudgeModel>()?;
    m.add_class::<PyEvalCase>()?;
    m.add_class::<PyScorerOutcome>()?;
    m.add_class::<PyEvalResult>()?;
    m.add_class::<PyEvalRun>()?;
    m.add_class::<PyAnnotationItem>()?;
    m.add_class::<PyRegressionResult>()?;

    // Dyn handles.
    m.add_class::<PyScorer>()?;
    m.add_class::<PyPairwiseScorer>()?;
    m.add_class::<PyEvalSuite>()?;
    m.add_class::<PyAnnotationQueue>()?;

    // Factories.
    m.add_function(wrap_pyfunction!(rubric_scorer, &m)?)?;
    m.add_function(wrap_pyfunction!(llm_judge_scorer, &m)?)?;
    m.add_function(wrap_pyfunction!(pairwise_scorer, &m)?)?;
    m.add_function(wrap_pyfunction!(scorer_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(in_memory_annotation_queue, &m)?)?;
    m.add_function(wrap_pyfunction!(regression_gate, &m)?)?;

    parent.add_submodule(&m)?;
    Ok(())
}
