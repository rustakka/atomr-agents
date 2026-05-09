//! LLM cache surface — `CacheKey` and `CachedTurn` data classes plus
//! `InMemoryLlmCache` async get/put.

use std::sync::Arc;

use atomr_agents_cache::{CacheKey, CachedTurn, InMemoryLlmCache, LlmCache};
use pyo3::prelude::*;

use crate::core::{PyFinishReason, PyTokenUsage};

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
    fn new(
        text: String,
        usage: Option<PyTokenUsage>,
        finish_reason: Option<PyFinishReason>,
    ) -> Self {
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
    fn put<'py>(
        &self,
        py: Python<'py>,
        key: PyCacheKey,
        value: PyCachedTurn,
    ) -> PyResult<Bound<'py, PyAny>> {
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

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "cache")?;
    m.add_class::<PyCacheKey>()?;
    m.add_class::<PyCachedTurn>()?;
    m.add_class::<PyInMemoryLlmCache>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
