//! Evaluation surface: data shape for cases, results, and verdicts.
//!
//! Phase B exposes the data classes needed for round-tripping eval
//! payloads through the registry. Async scorers (RubricScorer,
//! LlmJudgeScorer, PairwiseScorer) bridge through `crate::guest`
//! once the boxed form lands.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

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

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "eval")?;
    m.add_class::<PyPairwiseChoice>()?;
    m.add_class::<PyVerdict>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
