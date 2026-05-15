//! Python class → Rust `InstructionStrategy` adapter.
//!
//! The Python target's `render(ctx, budget)` is called once per turn
//! (sync or async). It must return a dict shaped like
//! `RenderedInstructions`:
//!
//! ```python
//! {"system_prompt": "...", "estimated_tokens": 42}
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, Result as AgentResult, TokenBudget};
use atomr_agents_instruction::{InstructionStrategy, RenderedInstructions};
use pyo3::prelude::*;

use super::conv_helpers::{await_and_jsonify, build_agent_ctx_dict, build_budget_dict, resolve_instance};
use super::registry::GUESTS;

pub struct PyInstructionAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyInstructionAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl InstructionStrategy for PyInstructionAdapter {
    async fn render(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<RenderedInstructions> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "render")?;
            let ctx_dict = build_agent_ctx_dict(py, ctx)?;
            let budget_dict = build_budget_dict(py, budget)?;
            let render = instance.getattr("render")?;
            let result = render.call1((ctx_dict, budget_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Strategy(format!("guest instruction {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Strategy(format!("guest instruction {label}: {e}")))?;

        let map = value.as_object().ok_or_else(|| {
            AgentError::Strategy(format!("guest instruction {label}: expected object, got {value}"))
        })?;
        let system_prompt = map
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let estimated_tokens = map.get("estimated_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok(RenderedInstructions {
            system_prompt,
            estimated_tokens,
        })
    }
}

/// Opaque Python handle holding `Arc<dyn InstructionStrategy>`. Lets
/// downstream Python APIs (eventually `Agent.from_spec`) accept a
/// strategy by handle without exposing the trait directly.
#[pyclass(name = "GuestInstruction", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyInstruction {
    /// Held for `Agent.from_spec` (W3b) which casts the handle into a
    /// `Box<dyn InstructionStrategy>` for `AgentSpec::into_agent`.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn InstructionStrategy>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyInstruction {
    fn __repr__(&self) -> String {
        format!("GuestInstruction(key={:?})", self.key)
    }
}

#[pyfunction]
pub(crate) fn build_guest_instruction_strategy(key: String) -> PyResult<PyInstruction> {
    let entry = GUESTS
        .get(&("strategy:instruction".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no instruction strategy registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PyInstructionAdapter::new(target, key.clone());
    Ok(PyInstruction {
        inner: Arc::new(adapter),
        key,
    })
}
