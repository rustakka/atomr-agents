//! LLM cache surface — `CacheKey` and `CachedTurn` data classes plus
//! `InMemoryLlmCache` and a dyn `LlmCache` handle (`PyLlmCache`) with
//! factories for semantic / sqlite / redis backends.

use std::sync::Arc;

use atomr_agents_cache::{CacheKey, CachedTurn, InMemoryLlmCache, LlmCache, SemanticLlmCache};
use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;

use crate::core::{PyFinishReason, PyTokenUsage};
use crate::embed::PyEmbedder;

#[pyclass(name = "CacheKey", module = "atomr_agents._native.cache")]
#[derive(Clone)]
pub struct PyCacheKey {
    pub(crate) inner: CacheKey,
}

#[pymethods]
impl PyCacheKey {
    #[new]
    fn new(model: String, messages_hash: u64, sampling_hash: u64) -> Self {
        Self {
            inner: CacheKey {
                model,
                messages_hash,
                sampling_hash,
            },
        }
    }

    #[getter]
    fn model(&self) -> &str {
        &self.inner.model
    }

    #[getter]
    fn messages_hash(&self) -> u64 {
        self.inner.messages_hash
    }

    #[getter]
    fn sampling_hash(&self) -> u64 {
        self.inner.sampling_hash
    }

    fn __repr__(&self) -> String {
        format!(
            "CacheKey(model={:?}, messages_hash={}, sampling_hash={})",
            self.inner.model, self.inner.messages_hash, self.inner.sampling_hash
        )
    }
}

#[pyclass(name = "CachedTurn", module = "atomr_agents._native.cache")]
#[derive(Clone)]
pub struct PyCachedTurn {
    pub(crate) inner: CachedTurn,
}

#[pymethods]
impl PyCachedTurn {
    #[new]
    #[pyo3(signature = (text, usage=None, finish_reason=None))]
    fn new(text: String, usage: Option<PyTokenUsage>, finish_reason: Option<PyFinishReason>) -> Self {
        let usage = usage
            .map(|u| atomr_infer_core::tokens::TokenUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                reasoning_tokens: u.reasoning_tokens,
                cached_tokens: u.cached_tokens,
                ..Default::default()
            })
            .unwrap_or_default();
        Self {
            inner: CachedTurn {
                text,
                usage,
                finish_reason: finish_reason.map(|_| atomr_infer_core::tokens::FinishReason::Stop),
            },
        }
    }

    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    #[getter]
    fn usage(&self) -> PyTokenUsage {
        PyTokenUsage::from(self.inner.usage)
    }

    fn __repr__(&self) -> String {
        format!(
            "CachedTurn(text={:?}, usage={:?})",
            self.inner.text, self.inner.usage
        )
    }
}

// ----- PyLlmCache dyn handle ----------------------------------------------

#[pyclass(name = "LlmCache", module = "atomr_agents._native.cache")]
#[derive(Clone)]
pub struct PyLlmCache {
    pub(crate) inner: Arc<dyn LlmCache>,
}

#[pymethods]
impl PyLlmCache {
    /// Async cache lookup. Returns `None` on miss, the cached turn on
    /// hit.
    fn get<'py>(&self, py: Python<'py>, key: PyCacheKey) -> PyResult<Bound<'py, PyAny>> {
        let cache = self.inner.clone();
        let k = key.inner;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = cache.get(&k).await.map_err(crate::errors::map)?;
            Ok(v.map(|inner| PyCachedTurn { inner }))
        })
    }

    /// Async cache store. No return value.
    fn put<'py>(&self, py: Python<'py>, key: PyCacheKey, value: PyCachedTurn) -> PyResult<Bound<'py, PyAny>> {
        let cache = self.inner.clone();
        let k = key.inner;
        let v = value.inner;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            cache.put(k, v).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        "LlmCache(handle)".into()
    }
}

#[pyclass(name = "InMemoryLlmCache", module = "atomr_agents._native.cache")]
pub struct PyInMemoryLlmCache {
    inner: Arc<InMemoryLlmCache>,
}

#[pymethods]
impl PyInMemoryLlmCache {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(InMemoryLlmCache::new()),
        }
    }

    /// Async cache lookup. Returns `None` on miss, the cached turn on
    /// hit.
    fn get<'py>(&self, py: Python<'py>, key: PyCacheKey) -> PyResult<Bound<'py, PyAny>> {
        let cache = self.inner.clone();
        let k = key.inner;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = cache.get(&k).await.map_err(crate::errors::map)?;
            Ok(v.map(|inner| PyCachedTurn { inner }))
        })
    }

    /// Async cache store. No return value.
    fn put<'py>(&self, py: Python<'py>, key: PyCacheKey, value: PyCachedTurn) -> PyResult<Bound<'py, PyAny>> {
        let cache = self.inner.clone();
        let k = key.inner;
        let v = value.inner;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            cache.put(k, v).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        "InMemoryLlmCache(...)".to_string()
    }
}

// ----- Factory functions --------------------------------------------------

/// Build a `SemanticLlmCache` backed by `embedder` with cosine
/// similarity `threshold`. Returns a dyn `LlmCache` handle.
#[pyfunction]
fn semantic_llm_cache(embedder: PyEmbedder, threshold: f32) -> PyLlmCache {
    PyLlmCache {
        inner: Arc::new(SemanticLlmCache::new(embedder.inner, threshold)),
    }
}

#[cfg(feature = "cache-sqlite")]
#[pyfunction]
fn sqlite_llm_cache(py: Python<'_>, path: String) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let inner = atomr_agents_cache::SqliteLlmCache::connect(path)
            .await
            .map_err(crate::errors::map)?;
        Ok(PyLlmCache {
            inner: Arc::new(inner),
        })
    })
}

#[cfg(not(feature = "cache-sqlite"))]
#[pyfunction]
fn sqlite_llm_cache(_path: String) -> PyResult<PyLlmCache> {
    Err(PyNotImplementedError::new_err(
        "sqlite_llm_cache: build atomr-agents-py-bindings with the \
         `cache-sqlite` feature to enable the SQLite LLM cache backend.",
    ))
}

#[cfg(feature = "cache-redis")]
#[pyfunction]
fn redis_llm_cache(py: Python<'_>, url: String) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let inner = atomr_agents_cache::RedisLlmCache::connect(url)
            .await
            .map_err(crate::errors::map)?;
        Ok(PyLlmCache {
            inner: Arc::new(inner),
        })
    })
}

#[cfg(not(feature = "cache-redis"))]
#[pyfunction]
fn redis_llm_cache(_url: String) -> PyResult<PyLlmCache> {
    Err(PyNotImplementedError::new_err(
        "redis_llm_cache: build atomr-agents-py-bindings with the \
         `cache-redis` feature to enable the Redis LLM cache backend.",
    ))
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "cache")?;
    m.add_class::<PyCacheKey>()?;
    m.add_class::<PyCachedTurn>()?;
    m.add_class::<PyLlmCache>()?;
    m.add_class::<PyInMemoryLlmCache>()?;
    m.add_function(wrap_pyfunction!(semantic_llm_cache, &m)?)?;
    m.add_function(wrap_pyfunction!(sqlite_llm_cache, &m)?)?;
    m.add_function(wrap_pyfunction!(redis_llm_cache, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
