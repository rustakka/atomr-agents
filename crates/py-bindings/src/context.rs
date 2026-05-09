//! Context fragment assembly — priority-based bin-packing under a
//! token budget.

use atomr_agents_context::{ContextAssembler, ContextFragment, RenderedContext};
use pyo3::prelude::*;

use crate::core::PyTokenBudget;
use crate::errors;

/// Convert a `String` into a `'static str` by leaking it.
/// `ContextFragment.source` is `&'static str` upstream; Python users
/// pass arbitrary strings, so we leak each unique label. The leak is
/// bounded by the number of distinct source labels, which is small
/// in practice (a handful per pipeline).
fn leak_static(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[pyclass(name = "ContextFragment", module = "atomr_agents._native.context")]
#[derive(Clone)]
pub struct PyContextFragment {
    pub(crate) inner: ContextFragment,
}

#[pymethods]
impl PyContextFragment {
    #[new]
    fn new(source: String, priority: u8, estimated_tokens: u32, text: String) -> Self {
        Self {
            inner: ContextFragment {
                source: leak_static(source),
                priority,
                estimated_tokens,
                text,
            },
        }
    }

    #[getter]
    fn source(&self) -> &str {
        self.inner.source
    }

    #[getter]
    fn priority(&self) -> u8 {
        self.inner.priority
    }

    #[getter]
    fn estimated_tokens(&self) -> u32 {
        self.inner.estimated_tokens
    }

    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    fn __repr__(&self) -> String {
        format!(
            "ContextFragment(source={:?}, priority={}, est_tokens={})",
            self.inner.source, self.inner.priority, self.inner.estimated_tokens
        )
    }
}

#[pyclass(name = "RenderedContext", module = "atomr_agents._native.context")]
pub struct PyRenderedContext {
    pub(crate) inner: RenderedContext,
}

#[pymethods]
impl PyRenderedContext {
    #[getter]
    fn total_tokens(&self) -> u32 {
        self.inner.total_tokens
    }

    fn fragments(&self) -> Vec<PyContextFragment> {
        self.inner
            .fragments
            .iter()
            .cloned()
            .map(|inner| PyContextFragment { inner })
            .collect()
    }

    fn join(&self, sep: &str) -> String {
        self.inner.join(sep)
    }

    fn __repr__(&self) -> String {
        format!(
            "RenderedContext(fragments={}, total_tokens={})",
            self.inner.fragments.len(),
            self.inner.total_tokens
        )
    }
}

/// `assemble(fragments, budget)` — packs the given fragments into the
/// remaining budget. The budget is mutated in place (use the budget's
/// `remaining` getter after to see consumption).
#[pyfunction]
fn assemble(
    fragments: Vec<PyContextFragment>,
    budget: &mut PyTokenBudget,
) -> PyResult<PyRenderedContext> {
    let frags: Vec<ContextFragment> = fragments.into_iter().map(|f| f.inner).collect();
    let rendered = ContextAssembler::assemble(frags, &mut budget.inner).map_err(|e| {
        PyErr::new::<errors::BudgetExhausted, _>(format!("context assemble: {e}"))
    })?;
    Ok(PyRenderedContext { inner: rendered })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "context")?;
    m.add_class::<PyContextFragment>()?;
    m.add_class::<PyRenderedContext>()?;
    m.add_function(wrap_pyfunction!(assemble, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
