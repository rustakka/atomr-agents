//! Python class → Rust `PersonaStrategy` adapter.
//!
//! `resolve(ctx, budget)` should return a dict shaped like
//! `RenderedPersona`:
//!
//! ```python
//! {"identity": "...", "salient_traits": [], "style": {}, "metadata": {}, "estimated_tokens": 4}
//! ```
//!
//! The deserialization round-trips via JSON to reuse the existing
//! `Serialize`/`Deserialize` impls on the persona structs.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, Result as AgentResult, TokenBudget};
use atomr_agents_persona::{PersonaStrategy, RenderedPersona};
use pyo3::prelude::*;

use super::conv_helpers::{
    await_and_jsonify, build_agent_ctx_dict, build_budget_dict, resolve_instance,
};
use super::registry::GUESTS;

pub struct PyPersonaAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyPersonaAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl PersonaStrategy for PyPersonaAdapter {
    async fn resolve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<RenderedPersona> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "resolve")?;
            let ctx_dict = build_agent_ctx_dict(py, ctx)?;
            let budget_dict = build_budget_dict(py, budget)?;
            let m = instance.getattr("resolve")?;
            let result = m.call1((ctx_dict, budget_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Strategy(format!("guest persona {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Strategy(format!("guest persona {label}: {e}")))?;

        // RenderedPersona derives Serialize/Deserialize — the dict
        // shape can deserialize directly.
        serde_json::from_value::<RenderedPersona>(value).map_err(|e| {
            AgentError::Strategy(format!(
                "guest persona {label}: invalid RenderedPersona shape: {e}"
            ))
        })
    }
}

#[pyclass(name = "GuestPersona", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyPersona {
    /// Held for `Agent.from_spec` (W3b) which casts the handle into a
    /// `Box<dyn PersonaStrategy>` for `AgentSpec::into_agent`.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn PersonaStrategy>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyPersona {
    fn __repr__(&self) -> String {
        format!("GuestPersona(key={:?})", self.key)
    }
}

#[pyfunction]
pub(crate) fn build_guest_persona(key: String) -> PyResult<PyPersona> {
    let entry = GUESTS
        .get(&("persona".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no persona registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PyPersonaAdapter::new(target, key.clone());
    Ok(PyPersona {
        inner: Arc::new(adapter),
        key,
    })
}
