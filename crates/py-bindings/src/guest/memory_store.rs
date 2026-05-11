//! Python class → Rust `MemoryStore` adapter.
//!
//! - `put(item)` accepts a `MemoryItem`-shaped dict and returns nothing.
//! - `list(namespace, limit)` accepts a namespace dict + int and
//!   returns a list of `MemoryItem`-shaped dicts.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, MemoryItem, MemoryNamespace, Result as AgentResult};
use atomr_agents_memory::MemoryStore;
use pyo3::prelude::*;

use super::conv_helpers::{
    await_and_jsonify, build_memory_item_dict, build_namespace_dict_pub, parse_memory_item_value,
    resolve_instance,
};
use super::registry::GUESTS;

pub struct PyMemoryStoreAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyMemoryStoreAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl MemoryStore for PyMemoryStoreAdapter {
    async fn put(&self, item: MemoryItem) -> AgentResult<()> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "put")?;
            let item_dict = build_memory_item_dict(py, &item)?;
            let m = instance.getattr("put")?;
            let result = m.call1((item_dict,))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Memory(format!("guest memory_store {label}: {e}")))?;

        let _ = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Memory(format!("guest memory_store {label}: {e}")))?;
        Ok(())
    }

    async fn list(
        &self,
        namespace: &MemoryNamespace,
        limit: usize,
    ) -> AgentResult<Vec<MemoryItem>> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "list")?;
            let ns_dict = build_namespace_dict_pub(py, namespace)?;
            let m = instance.getattr("list")?;
            let result = m.call1((ns_dict, limit))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Memory(format!("guest memory_store {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Memory(format!("guest memory_store {label}: {e}")))?;

        let arr = value.as_array().ok_or_else(|| {
            AgentError::Memory(format!(
                "guest memory_store {label}: expected array, got {value}"
            ))
        })?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            out.push(parse_memory_item_value(v).map_err(|e| {
                AgentError::Memory(format!("guest memory_store {label}: {e}"))
            })?);
        }
        Ok(out)
    }
}

#[pyclass(name = "GuestMemoryStore", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyMemoryStoreHandle {
    /// Held for downstream APIs (e.g. memory-strategy wiring) that
    /// take a `Box<dyn MemoryStore>` from a Python registration.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn MemoryStore>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyMemoryStoreHandle {
    fn __repr__(&self) -> String {
        format!("GuestMemoryStore(key={:?})", self.key)
    }
}

#[pyfunction]
pub(super) fn build_guest_memory_store(key: String) -> PyResult<PyMemoryStoreHandle> {
    let entry = GUESTS
        .get(&("memory".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no memory store registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PyMemoryStoreAdapter::new(target, key.clone());
    Ok(PyMemoryStoreHandle {
        inner: Arc::new(adapter),
        key,
    })
}
