//! Python class → Rust `SkillStrategy` adapter.
//!
//! `applicable(ctx, budget)` should return a list of skill-ref dicts:
//!
//! ```python
//! [{"id": "skill-x", "name": "...", "priority": 5}, ...]
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, Result as AgentResult, SkillId, TokenBudget};
use atomr_agents_strategy::{SkillRef, SkillStrategy};
use pyo3::prelude::*;

use super::conv_helpers::{
    await_and_jsonify, build_agent_ctx_dict, build_budget_dict, resolve_instance,
};
use super::registry::GUESTS;

pub struct PySkillStrategyAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PySkillStrategyAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl SkillStrategy for PySkillStrategyAdapter {
    async fn applicable(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> AgentResult<Vec<SkillRef>> {
        let target = self.target.clone();
        let label = self.label.clone();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "applicable")?;
            let ctx_dict = build_agent_ctx_dict(py, ctx)?;
            let budget_dict = build_budget_dict(py, budget)?;
            let m = instance.getattr("applicable")?;
            let result = m.call1((ctx_dict, budget_dict))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Strategy(format!("guest skill {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Strategy(format!("guest skill {label}: {e}")))?;

        let arr = value.as_array().ok_or_else(|| {
            AgentError::Strategy(format!("guest skill {label}: expected array, got {value}"))
        })?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let map = v.as_object().ok_or_else(|| {
                AgentError::Strategy(format!(
                    "guest skill {label}: expected skill ref object, got {v}"
                ))
            })?;
            let id = map
                .get("id")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    AgentError::Strategy(format!("guest skill {label}: skill ref missing id"))
                })?
                .to_string();
            let name = map
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let priority = map
                .get("priority")
                .and_then(|x| x.as_u64())
                .unwrap_or(5) as u8;
            out.push(SkillRef {
                id: SkillId::from(id),
                name,
                priority,
            });
        }
        Ok(out)
    }
}

#[pyclass(name = "GuestSkillStrategy", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PySkillStrategyHandle {
    /// Held for `Agent.from_spec` (W3b) which casts the handle into a
    /// `Box<dyn SkillStrategy>` for `AgentSpec::into_agent`.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn SkillStrategy>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PySkillStrategyHandle {
    fn __repr__(&self) -> String {
        format!("GuestSkillStrategy(key={:?})", self.key)
    }
}

#[pyfunction]
pub(super) fn build_guest_skill_strategy(key: String) -> PyResult<PySkillStrategyHandle> {
    let entry = GUESTS
        .get(&("strategy:skill".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no skill strategy registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PySkillStrategyAdapter::new(target, key.clone());
    Ok(PySkillStrategyHandle {
        inner: Arc::new(adapter),
        key,
    })
}
