//! `MemoryStore` + `LongStore` + memory strategies.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{
    AgentError, MemoryItem, MemoryNamespace, Result as AgentResult, Value,
};
use atomr_agents_memory::{
    InMemoryLongStore, InMemoryStore, LongStore, MemoryStore, Namespace, RecencyMemoryStrategy,
    StoreItem, SummarizingMemoryStrategy,
};
use atomr_agents_strategy::{ChainedMemoryStrategy, MemoryStrategy};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::conv::{json_to_py, py_to_json};
use crate::strategy::await_if_coro;

// ----- PyMemoryStore -------------------------------------------------------

#[pyclass(name = "MemoryStore", module = "atomr_agents._native.memory")]
#[derive(Clone)]
pub struct PyMemoryStore {
    pub(crate) inner: Arc<dyn MemoryStore>,
}

#[pymethods]
impl PyMemoryStore {
    fn put<'py>(
        &self,
        py: Python<'py>,
        item: crate::core::PyMemoryItem,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let item_inner = item.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.put(item_inner).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn list<'py>(
        &self,
        py: Python<'py>,
        namespace: crate::core::PyMemoryNamespace,
        limit: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let ns = namespace.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let items = inner.list(&ns, limit).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                let out = pyo3::types::PyList::empty_bound(py);
                for it in items {
                    let py_item = crate::core::PyMemoryItem { inner: it };
                    out.append(Py::new(py, py_item)?)?;
                }
                Ok(out.unbind().into())
            })
        })
    }

    fn __repr__(&self) -> String {
        "MemoryStore(handle)".into()
    }
}

pub(crate) struct PyMemoryStoreAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl MemoryStore for PyMemoryStoreAdapter {
    async fn put(&self, item: MemoryItem) -> AgentResult<()> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_item = Py::new(py, crate::core::PyMemoryItem { inner: item })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("put")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("put")?.call1((py_item,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py memory put: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }

    async fn list(
        &self,
        namespace: &MemoryNamespace,
        limit: usize,
    ) -> AgentResult<Vec<MemoryItem>> {
        let target = self.target.clone();
        let ns = namespace.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, crate::core::PyMemoryNamespace { inner: ns })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("list")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("list")?.call1((py_ns, limit))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py memory list: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<MemoryItem>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let item = item?;
                let mi: crate::core::PyMemoryItem = item.extract()?;
                out.push(mi.inner);
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("py memory list result: {e}")))
    }
}

// ----- PyLongStore --------------------------------------------------------

#[pyclass(name = "Namespace", module = "atomr_agents._native.memory")]
#[derive(Clone)]
pub struct PyNamespace {
    pub(crate) inner: Namespace,
}

#[pymethods]
impl PyNamespace {
    #[new]
    fn new(parts: Vec<String>) -> Self {
        Self {
            inner: Namespace(parts),
        }
    }

    #[staticmethod]
    fn from_parts(parts: Vec<String>) -> Self {
        Self {
            inner: Namespace(parts),
        }
    }

    #[getter]
    fn parts(&self) -> Vec<String> {
        self.inner.0.clone()
    }

    fn __repr__(&self) -> String {
        format!("Namespace({:?})", self.inner.0)
    }
}

#[pyclass(name = "StoreItem", module = "atomr_agents._native.memory")]
#[derive(Clone)]
pub struct PyStoreItem {
    pub(crate) inner: StoreItem,
}

#[pymethods]
impl PyStoreItem {
    #[getter]
    fn namespace(&self) -> PyNamespace {
        PyNamespace {
            inner: self.inner.namespace.clone(),
        }
    }

    #[getter]
    fn key(&self) -> &str {
        &self.inner.key
    }

    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.value)
    }

    #[getter]
    fn embedding(&self) -> Option<Vec<f32>> {
        self.inner.embedding.clone()
    }

    #[getter]
    fn score(&self) -> f32 {
        self.inner.score
    }

    #[getter]
    fn created_at_ms(&self) -> i64 {
        self.inner.created_at_ms
    }

    #[getter]
    fn updated_at_ms(&self) -> i64 {
        self.inner.updated_at_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "StoreItem(key={:?}, ns={:?}, score={:.3})",
            self.inner.key, self.inner.namespace.0, self.inner.score
        )
    }
}

#[pyclass(name = "LongStore", module = "atomr_agents._native.memory")]
#[derive(Clone)]
pub struct PyLongStore {
    pub(crate) inner: Arc<dyn LongStore>,
}

#[pymethods]
impl PyLongStore {
    fn put<'py>(
        &self,
        py: Python<'py>,
        namespace: PyNamespace,
        key: String,
        value: &Bound<'py, PyAny>,
        embedding: Option<Vec<f32>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let v = py_to_json(py, value)?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .put(&namespace.inner, &key, v, embedding)
                .await
                .map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn get<'py>(
        &self,
        py: Python<'py>,
        namespace: PyNamespace,
        key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = inner
                .get(&namespace.inner, &key)
                .await
                .map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                match out {
                    Some(item) => Ok(Py::new(py, PyStoreItem { inner: item })?.into_py(py)),
                    None => Ok(py.None()),
                }
            })
        })
    }

    fn delete<'py>(
        &self,
        py: Python<'py>,
        namespace: PyNamespace,
        key: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner
                .delete(&namespace.inner, &key)
                .await
                .map_err(crate::errors::map)?;
            Ok(())
        })
    }

    #[pyo3(signature = (namespace, top_k, query_embedding=None))]
    fn search<'py>(
        &self,
        py: Python<'py>,
        namespace: PyNamespace,
        top_k: usize,
        query_embedding: Option<Vec<f32>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let q_ref = query_embedding.as_deref();
            let items = inner
                .search(&namespace.inner, q_ref, top_k)
                .await
                .map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                let out = pyo3::types::PyList::empty_bound(py);
                for it in items {
                    let py_item = PyStoreItem { inner: it };
                    out.append(Py::new(py, py_item)?)?;
                }
                Ok(out.unbind().into())
            })
        })
    }

    fn list_namespaces<'py>(
        &self,
        py: Python<'py>,
        prefix: PyNamespace,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let nss = inner
                .list_namespaces(&prefix.inner)
                .await
                .map_err(crate::errors::map)?;
            Ok(nss.into_iter().map(|n| n.0).collect::<Vec<Vec<String>>>())
        })
    }

    fn __repr__(&self) -> String {
        "LongStore(handle)".into()
    }
}

pub(crate) struct PyLongStoreAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl LongStore for PyLongStoreAdapter {
    async fn put(
        &self,
        namespace: &Namespace,
        key: &str,
        value: Value,
        embedding: Option<Vec<f32>>,
    ) -> AgentResult<()> {
        let target = self.target.clone();
        let ns = namespace.clone();
        let key = key.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, PyNamespace { inner: ns })?;
            let val_obj = json_to_py(py, &value)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("put")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance
                .getattr("put")?
                .call1((py_ns, key, val_obj, embedding))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py long_store put: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }

    async fn get(&self, namespace: &Namespace, key: &str) -> AgentResult<Option<StoreItem>> {
        let target = self.target.clone();
        let ns = namespace.clone();
        let key = key.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, PyNamespace { inner: ns })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("get")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("get")?.call1((py_ns, key))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py long_store get: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Option<StoreItem>> {
            let bound = final_val.bind(py);
            if bound.is_none() {
                return Ok(None);
            }
            let item: PyStoreItem = bound.extract()?;
            Ok(Some(item.inner))
        })
        .map_err(|e| AgentError::Internal(format!("py long_store get result: {e}")))
    }

    async fn delete(&self, namespace: &Namespace, key: &str) -> AgentResult<()> {
        let target = self.target.clone();
        let ns = namespace.clone();
        let key = key.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, PyNamespace { inner: ns })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("delete")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("delete")?.call1((py_ns, key))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py long_store delete: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }

    async fn search(
        &self,
        namespace: &Namespace,
        query_embedding: Option<&[f32]>,
        top_k: usize,
    ) -> AgentResult<Vec<StoreItem>> {
        let target = self.target.clone();
        let ns = namespace.clone();
        let q = query_embedding.map(|v| v.to_vec());
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, PyNamespace { inner: ns })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("search")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("search")?.call1((py_ns, q, top_k))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py long_store search: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<StoreItem>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let it = item?;
                let si: PyStoreItem = it.extract()?;
                out.push(si.inner);
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("py long_store search result: {e}")))
    }

    async fn list_namespaces(&self, prefix: &Namespace) -> AgentResult<Vec<Namespace>> {
        let target = self.target.clone();
        let p = prefix.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let py_ns = Py::new(py, PyNamespace { inner: p })?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("list_namespaces")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("list_namespaces")?.call1((py_ns,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py long_store list_namespaces: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<Namespace>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let parts: Vec<String> = item?.extract()?;
                out.push(Namespace(parts));
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("py long_store list_namespaces result: {e}")))
    }
}

// ----- Factories ----------------------------------------------------------

#[pyfunction]
fn in_memory_store() -> PyMemoryStore {
    PyMemoryStore {
        inner: Arc::new(InMemoryStore::new()),
    }
}

#[pyfunction]
fn memory_store_from_factory(key: String) -> PyResult<PyMemoryStore> {
    let target = crate::guest::must_lookup("memory", &key)?;
    Ok(PyMemoryStore {
        inner: Arc::new(PyMemoryStoreAdapter { target }),
    })
}

#[pyfunction]
fn in_memory_long_store() -> PyLongStore {
    PyLongStore {
        inner: Arc::new(InMemoryLongStore::new()),
    }
}

#[pyfunction]
fn long_store_from_factory(key: String) -> PyResult<PyLongStore> {
    let target = crate::guest::must_lookup("long_store", &key)?;
    Ok(PyLongStore {
        inner: Arc::new(PyLongStoreAdapter { target }),
    })
}

// ----- Strategy factories -------------------------------------------------

#[pyfunction]
#[pyo3(signature = (store, limit=8, tokens_per_item=32))]
fn recency_memory_strategy(
    store: PyMemoryStore,
    limit: usize,
    tokens_per_item: u32,
) -> crate::strategy::PyMemoryStrategy {
    crate::strategy::PyMemoryStrategy {
        inner: Arc::new(RecencyMemoryStrategy::new(
            store.inner,
            limit,
            tokens_per_item,
        )),
    }
}

#[pyfunction]
#[pyo3(signature = (inner, max_summary_tokens=512))]
fn summarizing_memory_strategy(
    inner: crate::strategy::PyMemoryStrategy,
    max_summary_tokens: u32,
) -> crate::strategy::PyMemoryStrategy {
    // SummarizingMemoryStrategy is generic over its inner strategy.
    // Wrap the Arc<dyn> in a small concrete adapter to satisfy `I:
    // MemoryStrategy`.
    struct ArcWrap(Arc<dyn MemoryStrategy>);
    #[async_trait]
    impl MemoryStrategy for ArcWrap {
        async fn retrieve(
            &self,
            ctx: &atomr_agents_core::AgentContext,
            budget: &mut atomr_agents_core::TokenBudget,
        ) -> AgentResult<Vec<atomr_agents_core::MemoryChunk>> {
            self.0.retrieve(ctx, budget).await
        }
        async fn store(&self, item: MemoryItem) -> AgentResult<()> {
            self.0.store(item).await
        }
    }
    crate::strategy::PyMemoryStrategy {
        inner: Arc::new(SummarizingMemoryStrategy::new(
            ArcWrap(inner.inner),
            max_summary_tokens,
        )),
    }
}

#[pyfunction]
fn chained_memory_strategy(
    strategies: Vec<crate::strategy::PyMemoryStrategy>,
) -> crate::strategy::PyMemoryStrategy {
    // ChainedMemoryStrategy expects `Vec<Box<dyn MemoryStrategy>>`.
    // Convert from Arc by wrapping each via a small adapter.
    struct ArcWrap(Arc<dyn MemoryStrategy>);
    #[async_trait]
    impl MemoryStrategy for ArcWrap {
        async fn retrieve(
            &self,
            ctx: &atomr_agents_core::AgentContext,
            budget: &mut atomr_agents_core::TokenBudget,
        ) -> AgentResult<Vec<atomr_agents_core::MemoryChunk>> {
            self.0.retrieve(ctx, budget).await
        }

        async fn store(&self, item: MemoryItem) -> AgentResult<()> {
            self.0.store(item).await
        }
    }
    let members: Vec<Box<dyn MemoryStrategy>> = strategies
        .into_iter()
        .map(|s| Box::new(ArcWrap(s.inner)) as Box<dyn MemoryStrategy>)
        .collect();
    crate::strategy::PyMemoryStrategy {
        inner: Arc::new(ChainedMemoryStrategy::new(members)),
    }
}

// Use `_ = ...` to keep helper imports referenced.
#[allow(dead_code)]
fn _use_pydict(_d: &PyDict) {}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "memory")?;
    m.add_class::<PyMemoryStore>()?;
    m.add_class::<PyLongStore>()?;
    m.add_class::<PyNamespace>()?;
    m.add_class::<PyStoreItem>()?;
    m.add_function(wrap_pyfunction!(in_memory_store, &m)?)?;
    m.add_function(wrap_pyfunction!(memory_store_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(in_memory_long_store, &m)?)?;
    m.add_function(wrap_pyfunction!(long_store_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(recency_memory_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(summarizing_memory_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(chained_memory_strategy, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
