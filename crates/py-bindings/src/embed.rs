//! `Embedder` + `AnnIndex` bindings.
//!
//! Concrete in-memory factories (`mock_embedder`, `in_memory_ann_index`)
//! return dyn handles. Python implementations are registered through
//! `guest.embedder(...)` / `guest.ann_index(...)` and materialized via
//! `embedder_from_factory(key)` / `ann_index_from_factory(key)`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result as AgentResult};
use atomr_agents_embed::{AnnId, AnnIndex, Embedder, InMemoryAnnIndex, MockEmbedder};
use pyo3::prelude::*;

use crate::strategy::await_if_coro;

// ----- PyEmbedder dyn handle ----------------------------------------------

#[pyclass(name = "Embedder", module = "atomr_agents._native.embed")]
#[derive(Clone)]
pub struct PyEmbedder {
    pub(crate) inner: Arc<dyn Embedder>,
}

#[pymethods]
impl PyEmbedder {
    fn dim(&self) -> usize {
        self.inner.dim()
    }

    fn embed<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.embed(&text).await.map_err(crate::errors::map)
        })
    }

    fn embed_batch<'py>(
        &self,
        py: Python<'py>,
        texts: Vec<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.embed_batch(&texts).await.map_err(crate::errors::map)
        })
    }

    fn __repr__(&self) -> String {
        format!("Embedder(dim={})", self.inner.dim())
    }
}

// ----- Python guest adapter -----------------------------------------------

pub(crate) struct PyEmbedderAdapter {
    pub(crate) target: Arc<PyObject>,
}

#[async_trait]
impl Embedder for PyEmbedderAdapter {
    fn dim(&self) -> usize {
        // Best-effort: ask the Python target. Fall back to 0.
        Python::with_gil(|py| {
            let bound = self.target.bind(py);
            match bound.getattr("dim") {
                Ok(d) => match d.is_callable() {
                    true => d.call0().and_then(|r| r.extract()).unwrap_or(0_usize),
                    false => d.extract().unwrap_or(0_usize),
                },
                Err(_) => 0,
            }
        })
    }

    async fn embed(&self, text: &str) -> AgentResult<Vec<f32>> {
        let target = self.target.clone();
        let text = text.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("embed")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("embed")?.call1((text,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py embedder: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<Vec<f32>>())
            .map_err(|e| AgentError::Internal(format!("py embedder result: {e}")))
    }
}

// ----- Factory functions --------------------------------------------------

#[pyfunction]
fn mock_embedder(dim: usize) -> PyEmbedder {
    PyEmbedder {
        inner: Arc::new(MockEmbedder::new(dim)),
    }
}

#[pyfunction]
fn embedder_from_factory(key: String) -> PyResult<PyEmbedder> {
    let target = crate::guest::must_lookup("embedder", &key)?;
    Ok(PyEmbedder {
        inner: Arc::new(PyEmbedderAdapter { target }),
    })
}

// ----- PyAnnIndex dyn handle ----------------------------------------------

#[pyclass(name = "AnnIndex", module = "atomr_agents._native.embed")]
#[derive(Clone)]
pub struct PyAnnIndex {
    pub(crate) inner: Arc<dyn AnnIndex>,
}

#[pymethods]
impl PyAnnIndex {
    fn upsert<'py>(
        &self,
        py: Python<'py>,
        id: AnnId,
        vec: Vec<f32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.upsert(id, vec).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn search<'py>(
        &self,
        py: Python<'py>,
        query: Vec<f32>,
        top_k: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let r = inner.search(&query, top_k).await.map_err(crate::errors::map)?;
            Ok(r) // pyo3 turns Vec<(u64, f32)> into list[tuple[int, float]]
        })
    }

    fn len<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.len().await.map_err(crate::errors::map)
        })
    }

    fn __repr__(&self) -> String {
        "AnnIndex(handle)".into()
    }
}

// ----- AnnIndex adapter ---------------------------------------------------

pub(crate) struct PyAnnIndexAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl AnnIndex for PyAnnIndexAdapter {
    async fn upsert(&self, id: AnnId, vec: Vec<f32>) -> AgentResult<()> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("upsert")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("upsert")?.call1((id, vec))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py ann upsert: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }

    async fn search(&self, query: &[f32], top_k: usize) -> AgentResult<Vec<(AnnId, f32)>> {
        let target = self.target.clone();
        let query = query.to_vec();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("search")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("search")?.call1((query, top_k))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py ann search: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<Vec<(AnnId, f32)>>())
            .map_err(|e| AgentError::Internal(format!("py ann result: {e}")))
    }

    async fn len(&self) -> AgentResult<usize> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("len")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("len")?.call0()?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py ann len: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<usize>())
            .map_err(|e| AgentError::Internal(format!("py ann len result: {e}")))
    }
}

#[pyfunction]
fn in_memory_ann_index(dim: usize) -> PyAnnIndex {
    PyAnnIndex {
        inner: Arc::new(InMemoryAnnIndex::new(dim)),
    }
}

#[pyfunction]
fn ann_index_from_factory(key: String) -> PyResult<PyAnnIndex> {
    let target = crate::guest::must_lookup("ann_index", &key)?;
    Ok(PyAnnIndex {
        inner: Arc::new(PyAnnIndexAdapter { target }),
    })
}

// ----- EmbeddingToolStrategy ----------------------------------------------

// Returns a PyToolStrategy by wrapping the concrete
// `EmbeddingToolStrategy`. The strategy `select`s tools by embedding
// similarity over the available tool set. This binding is a no-op
// scaffold until tools have a `description_embedding()` accessor.
//
// For now, expose only the factory shape — the underlying type does
// not need extra construction inputs beyond an embedder.

#[pyfunction]
fn embedding_tool_strategy(_embedder: PyEmbedder) -> crate::strategy::PyToolStrategy {
    // The underlying EmbeddingToolStrategy is generic and currently
    // requires more wiring (tool descriptions + threshold). Until
    // Phase 4 expands tool/skill strategies, expose an empty
    // StaticToolStrategy so downstream code compiles.
    use atomr_agents_strategy::ToolStrategy;
    struct EmptyTools;
    #[async_trait::async_trait]
    impl ToolStrategy for EmptyTools {
        async fn select(
            &self,
            _ctx: &atomr_agents_core::AgentContext,
            _budget: &mut atomr_agents_core::TokenBudget,
        ) -> AgentResult<Vec<atomr_agents_strategy::ToolRef>> {
            Ok(Vec::new())
        }
    }
    crate::strategy::PyToolStrategy {
        inner: Arc::new(EmptyTools),
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "embed")?;
    m.add_class::<PyEmbedder>()?;
    m.add_class::<PyAnnIndex>()?;
    m.add_function(wrap_pyfunction!(mock_embedder, &m)?)?;
    m.add_function(wrap_pyfunction!(embedder_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(in_memory_ann_index, &m)?)?;
    m.add_function(wrap_pyfunction!(ann_index_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(embedding_tool_strategy, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
