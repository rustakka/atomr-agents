//! Evaluation surface: data shape for cases, results, and verdicts +
//! a Python adapter that promotes a Python class implementing
//! `score(expected, actual)` (sync OR async) into a Rust
//! [`AsyncScorer`].

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Value;
use atomr_agents_eval::{AsyncScorer, ScorerOutcome};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::conv::{json_to_py, py_to_json};

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

#[pyclass(name = "Verdict", module = "atomr_agents._native.eval", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyVerdict {
    inner: String,
}

#[pymethods]
impl PyVerdict {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let valid = ["approved", "rejected", "needs_review"];
        if !valid.contains(&name) {
            return Err(PyValueError::new_err(format!(
                "unknown verdict: {name:?}"
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

    fn __repr__(&self) -> String {
        format!("Verdict({:?})", self.inner)
    }
}

// ----- ScorerOutcome data class --------------------------------------------

/// Python view onto [`ScorerOutcome`]. Constructed by `AsyncScorer.score`
/// and by Python tests/fixtures that want to round-trip an outcome.
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
            inner: ScorerOutcome { passed, score, note },
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
            "ScorerOutcome(passed={}, score={}, note={:?})",
            self.inner.passed, self.inner.score, self.inner.note
        )
    }
}

// ----- PyAsyncScorerAdapter -------------------------------------------------
//
// Wraps a Python class/instance as a Rust `AsyncScorer`. The Python
// side exposes `score(expected, actual) -> dict | ScorerOutcome` (or
// the async equivalent). Mirrors the `PyToolAdapter` pattern in
// `crate::guest`: GIL acquisition, instance-vs-class detection,
// coroutine detection via `inspect.iscoroutine`, awaiting via
// `pyo3_async_runtimes::tokio::into_future`, JSON round-trip.

pub struct PyAsyncScorerAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyAsyncScorerAdapter {
    pub fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }

    fn fail(&self, stage: &str, e: impl std::fmt::Display) -> ScorerOutcome {
        ScorerOutcome {
            passed: false,
            score: 0.0,
            note: format!("guest scorer {} {}: {}", self.label, stage, e),
        }
    }
}

#[async_trait]
impl AsyncScorer for PyAsyncScorerAdapter {
    async fn score(&self, expected: &Value, actual: &Value) -> ScorerOutcome {
        let target = self.target.clone();

        // Step 1: under the GIL, instantiate the target if needed and
        // call `.score(expected, actual)`. Returns either the final
        // value or a coroutine to be awaited below.
        let coro_or_val = match Python::with_gil(|py| -> PyResult<PyObject> {
            let exp = json_to_py(py, expected)?;
            let act = json_to_py(py, actual)?;
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("score")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let m = instance.getattr("score")?;
            let result = m.call1((exp, act))?;
            Ok(result.unbind())
        }) {
            Ok(v) => v,
            Err(e) => return self.fail("call", e),
        };

        // Step 2: if the return value is a coroutine, await it on the
        // tokio runtime via pyo3-async-runtimes. The branch is needed
        // because `into_future` only accepts coroutines.
        let final_val = {
            let maybe_future = match Python::with_gil(|py| -> PyResult<Option<_>> {
                let bound = coro_or_val.bind(py);
                let inspect = py.import_bound("inspect")?;
                let is_coro: bool = inspect
                    .getattr("iscoroutine")?
                    .call1((bound,))?
                    .extract()?;
                if is_coro {
                    Ok(Some(pyo3_async_runtimes::tokio::into_future(bound.clone())?))
                } else {
                    Ok(None)
                }
            }) {
                Ok(v) => v,
                Err(e) => return self.fail("coroutine check", e),
            };

            match maybe_future {
                Some(fut) => match fut.await {
                    Ok(v) => v,
                    Err(e) => return self.fail("await", e),
                },
                None => coro_or_val,
            }
        };

        // Step 3: deserialize the result. Accept either a dict-shaped
        // value (json_to_py round-trip → ScorerOutcome via serde) or a
        // PyScorerOutcome instance (return its inner directly).
        match Python::with_gil(|py| -> PyResult<ScorerOutcome> {
            let bound = final_val.bind(py);
            if let Ok(po) = bound.extract::<PyRef<PyScorerOutcome>>() {
                return Ok(po.inner.clone());
            }
            let v = py_to_json(py, bound)?;
            serde_json::from_value::<ScorerOutcome>(v).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "ScorerOutcome deserialize: {e}"
                ))
            })
        }) {
            Ok(out) => out,
            Err(e) => self.fail("result", e),
        }
    }
}

// ----- PyAsyncScorer -------------------------------------------------------
//
// Thin Python-facing wrapper holding `Arc<dyn AsyncScorer>`. The
// trait-object indirection lets us hand back whatever scorer the
// builder constructed (a `PyAsyncScorerAdapter`, a Rust `LlmJudgeScorer`,
// etc.) under one Python type.

#[pyclass(name = "AsyncScorer", module = "atomr_agents._native.eval")]
pub struct PyAsyncScorer {
    pub(crate) inner: Arc<dyn AsyncScorer>,
    pub(crate) key: String,
}

#[pymethods]
impl PyAsyncScorer {
    /// Score one expected/actual pair. Returns a Python coroutine that
    /// resolves to a [`PyScorerOutcome`].
    fn score<'py>(
        &self,
        py: Python<'py>,
        expected: PyObject,
        actual: PyObject,
    ) -> PyResult<Bound<'py, PyAny>> {
        let exp = py_to_json(py, expected.bind(py))?;
        let act = py_to_json(py, actual.bind(py))?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = inner.score(&exp, &act).await;
            Python::with_gil(|py| {
                Py::new(py, PyScorerOutcome { inner: out }).map(|p| p.into_any())
            })
        })
    }

    #[getter]
    fn key(&self) -> &str {
        &self.key
    }

    fn __repr__(&self) -> String {
        format!("AsyncScorer(key={:?})", self.key)
    }
}

/// Build an [`PyAsyncScorer`] from a previously-registered Python
/// scorer factory. The key must have been registered via
/// `atomr_agents._native.guest.register_scorer_factory(key, target)`.
#[pyfunction]
fn build_guest_async_scorer(key: String) -> PyResult<PyAsyncScorer> {
    let target = crate::guest::lookup_guest("scorer", &key).ok_or_else(|| {
        pyo3::exceptions::PyKeyError::new_err(format!(
            "no scorer registered with key {key:?}"
        ))
    })?;
    let adapter = PyAsyncScorerAdapter::new(target, key.clone());
    Ok(PyAsyncScorer {
        inner: Arc::new(adapter),
        key,
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "eval")?;
    m.add_class::<PyPairwiseChoice>()?;
    m.add_class::<PyVerdict>()?;
    m.add_class::<PyScorerOutcome>()?;
    m.add_class::<PyAsyncScorer>()?;
    m.add_function(wrap_pyfunction!(build_guest_async_scorer, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
