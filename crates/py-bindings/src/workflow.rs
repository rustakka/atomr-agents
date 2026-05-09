//! Workflow data types — DAG, step kinds, run outcomes.
//!
//! Phase B exposes the static shape (Step kinds as a string-tagged
//! discriminator). Async `WorkflowRunner.run` requires a boxed
//! runtime form that doesn't yet exist; the registration here keeps
//! the submodule available so future PRs can drop in the runner
//! without churning the import path.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

#[pyclass(name = "StepKind", module = "atomr_agents._native.workflow", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyStepKind {
    inner: String,
}

#[pymethods]
impl PyStepKind {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let valid = ["invoke", "branch", "parallel", "loop", "map", "human"];
        if !valid.contains(&name) {
            return Err(PyValueError::new_err(format!("unknown step kind: {name:?}")));
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
    fn invoke() -> Self {
        Self {
            inner: "invoke".to_string(),
        }
    }
    #[staticmethod]
    fn branch() -> Self {
        Self {
            inner: "branch".to_string(),
        }
    }
    #[staticmethod]
    fn parallel() -> Self {
        Self {
            inner: "parallel".to_string(),
        }
    }
    #[staticmethod]
    fn loop_() -> Self {
        Self {
            inner: "loop".to_string(),
        }
    }
    #[staticmethod]
    fn map() -> Self {
        Self {
            inner: "map".to_string(),
        }
    }
    #[staticmethod]
    fn human() -> Self {
        Self {
            inner: "human".to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("StepKind({:?})", self.inner)
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "workflow")?;
    m.add_class::<PyStepKind>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
