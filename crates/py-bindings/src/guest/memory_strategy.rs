//! Python class → Rust `MemoryStrategy` adapter.
//!
//! `retrieve(ctx, budget)` should return a list of `MemoryChunk`-shaped
//! dicts; `store(item)` accepts a dict that round-trips through
//! `MemoryItem` and returns nothing.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, MemoryChunk, MemoryItem, Result as AgentResult, TokenBudget};
use atomr_agents_strategy::MemoryStrategy;
use pyo3::prelude::*;

use super::conv_helpers::{
    await_and_jsonify, build_agent_ctx_dict, build_budget_dict, build_memory_item_dict,
    resolve_instance,
};
use super::registry::GUESTS;

pub struct PyMemoryStrategyAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyMemoryStrategyAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl MemoryStrategy for PyMemoryStrategyAdapter {
    async fn retrieve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<Vec<MemoryChunk>> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "retrieve")?;
            let ctx_dict = build_agent_ctx_dict(py, ctx)?;
            let budget_dict = build_budget_dict(py, budget)?;
            let m = instance.getattr("retrieve")?;
            let result = m.call1((ctx_dict, budget_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Strategy(format!("guest memory {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Strategy(format!("guest memory {label}: {e}")))?;

        let arr = value.as_array().ok_or_else(|| {
            AgentError::Strategy(format!(
                "guest memory {label}: expected array, got {value}"
            ))
        })?;

        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let map = v.as_object().ok_or_else(|| {
                AgentError::Strategy(format!(
                    "guest memory {label}: expected chunk object, got {v}"
                ))
            })?;
            out.push(MemoryChunk {
                source_id: map
                    .get("source_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                text: map
                    .get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                score: map.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                estimated_tokens: map
                    .get("estimated_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as u32,
            });
        }
        Ok(out)
    }

    async fn store(&self, item: MemoryItem) -> AgentResult<()> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "store")?;
            let item_dict = build_memory_item_dict(py, &item)?;
            let m = instance.getattr("store")?;
            let result = m.call1((item_dict,))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Strategy(format!("guest memory {label}: {e}")))?;

        // Discard the return value but await if it's a coroutine.
        let _ = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Strategy(format!("guest memory {label}: {e}")))?;
        Ok(())
    }
}

#[pyclass(name = "GuestMemoryStrategy", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyMemoryStrategyHandle {
    /// Held for `Agent.from_spec` (W3b) which casts the handle into a
    /// `Box<dyn MemoryStrategy>` for `AgentSpec::into_agent`.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn MemoryStrategy>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyMemoryStrategyHandle {
    fn __repr__(&self) -> String {
        format!("GuestMemoryStrategy(key={:?})", self.key)
    }
}

#[pyfunction]
pub(crate) fn build_guest_memory_strategy(key: String) -> PyResult<PyMemoryStrategyHandle> {
    let entry = GUESTS
        .get(&("strategy:memory".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no memory strategy registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PyMemoryStrategyAdapter::new(target, key.clone());
    Ok(PyMemoryStrategyHandle {
        inner: Arc::new(adapter),
        key,
    })
}
