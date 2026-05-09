//! Persistent harness loop — `HarnessSpec` data shape.
//!
//! Like Agent / WorkflowRunner, `Harness<L, T>` is monomorphized over
//! the loop / termination strategy traits. Phase B exposes `HarnessSpec`
//! and the `IterationCapTermination` static class. Async `Harness.run()`
//! ships once a `BoxedHarness` form lands upstream.

use atomr_agents_core::{HarnessId, TokenBudget};
use atomr_agents_harness::{HarnessSpec, IterationCapTermination};
use pyo3::prelude::*;
use semver::Version;

use crate::conv::parse_version;

#[pyclass(name = "HarnessSpec", module = "atomr_agents._native.harness")]
#[derive(Clone)]
pub struct PyHarnessSpec {
    pub(crate) inner: HarnessSpec,
}

#[pymethods]
impl PyHarnessSpec {
    #[new]
    #[pyo3(signature = (id, version, initial_token_budget=8000, eval_suite_id=None))]
    fn new(
        id: String,
        version: &str,
        initial_token_budget: u32,
        eval_suite_id: Option<String>,
    ) -> PyResult<Self> {
        let v: Version = parse_version(version)?;
        Ok(Self {
            inner: HarnessSpec {
                id: HarnessId::from(id),
                version: v,
                eval_suite_id,
                initial_budget: TokenBudget::new(initial_token_budget),
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    #[getter]
    fn eval_suite_id(&self) -> Option<String> {
        self.inner.eval_suite_id.clone()
    }

    #[getter]
    fn initial_token_budget(&self) -> u32 {
        self.inner.initial_budget.remaining
    }

    fn __repr__(&self) -> String {
        format!(
            "HarnessSpec(id={:?}, version={:?}, tokens={})",
            self.inner.id.as_str(),
            self.inner.version.to_string(),
            self.inner.initial_budget.remaining,
        )
    }
}

#[pyclass(name = "IterationCapTermination", module = "atomr_agents._native.harness")]
pub struct PyIterationCapTermination {
    pub(crate) inner: IterationCapTermination,
}

#[pymethods]
impl PyIterationCapTermination {
    #[new]
    fn new(cap: u64) -> Self {
        Self {
            inner: IterationCapTermination { cap },
        }
    }

    #[getter]
    fn cap(&self) -> u64 {
        self.inner.cap
    }

    fn __repr__(&self) -> String {
        format!("IterationCapTermination(cap={})", self.inner.cap)
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "harness")?;
    m.add_class::<PyHarnessSpec>()?;
    m.add_class::<PyIterationCapTermination>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
