//! Core data types: ID newtypes, message / role / turn-input shapes,
//! resource budgets (Token/Time/Money/Iteration), memory primitives,
//! and a re-export of inference token types from `atomr_infer_core`.
//!
//! Mirrors `atomr-infer/inference-py-bindings/src/core.rs`'s shape:
//! data classes wrapping their Rust counterparts with field
//! getters/setters and value semantics.

use std::time::Duration;

use atomr_agents_core::{
    AgentId, DepartmentId, HarnessId, IterationBudget, MemoryChunk, MemoryItem, MemoryKind,
    MemoryNamespace, MoneyBudget, OrgId, PersonaId, RunId, SkillId, TeamId, TimeBudget, TokenBudget,
    ToolId, ToolSetId, WorkflowId,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::conv::{json_to_py, py_to_json};
use crate::errors;

// ----- ID newtype wrappers --------------------------------------------------

/// One macro produces the PyO3 wrapper for each id-newtype.
/// Each ID is hashable + comparable + has a `.value` getter.
macro_rules! id_pyclass {
    ($py:ident, $rs:ty, $name:literal) => {
        #[pyclass(name = $name, module = "atomr_agents._native.core", eq, hash, frozen)]
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $py {
            pub(crate) inner: $rs,
        }

        #[pymethods]
        impl $py {
            #[new]
            fn new(value: String) -> Self {
                Self {
                    inner: <$rs>::from(value),
                }
            }

            #[staticmethod]
            fn generate() -> Self {
                Self {
                    inner: <$rs>::new(),
                }
            }

            #[getter]
            fn value(&self) -> &str {
                self.inner.as_str()
            }

            fn __repr__(&self) -> String {
                format!("{}({:?})", $name, self.inner.as_str())
            }

            fn __str__(&self) -> &str {
                self.inner.as_str()
            }
        }

        impl From<$rs> for $py {
            fn from(inner: $rs) -> Self {
                Self { inner }
            }
        }

        impl From<$py> for $rs {
            fn from(p: $py) -> Self {
                p.inner
            }
        }
    };
}

id_pyclass!(PyAgentId, AgentId, "AgentId");
id_pyclass!(PyTeamId, TeamId, "TeamId");
id_pyclass!(PyDepartmentId, DepartmentId, "DepartmentId");
id_pyclass!(PyOrgId, OrgId, "OrgId");
id_pyclass!(PyWorkflowId, WorkflowId, "WorkflowId");
id_pyclass!(PyHarnessId, HarnessId, "HarnessId");
id_pyclass!(PyToolId, ToolId, "ToolId");
id_pyclass!(PyToolSetId, ToolSetId, "ToolSetId");
id_pyclass!(PySkillId, SkillId, "SkillId");
id_pyclass!(PyPersonaId, PersonaId, "PersonaId");
id_pyclass!(PyRunId, RunId, "RunId");

// ----- Budgets --------------------------------------------------------------

#[pyclass(name = "TokenBudget", module = "atomr_agents._native.core")]
#[derive(Clone, Copy)]
pub struct PyTokenBudget {
    pub(crate) inner: TokenBudget,
}

#[pymethods]
impl PyTokenBudget {
    #[new]
    fn new(total: u32) -> Self {
        Self {
            inner: TokenBudget::new(total),
        }
    }

    #[getter]
    fn remaining(&self) -> u32 {
        self.inner.remaining
    }

    #[getter]
    fn reserved(&self) -> u32 {
        self.inner.reserved
    }

    fn consume(&mut self, n: u32) -> PyResult<()> {
        self.inner
            .consume(n)
            .map_err(|_| PyErr::new::<errors::BudgetExhausted, _>("token budget exceeded"))
    }

    fn reserve(&mut self, n: u32) -> PyResult<()> {
        self.inner
            .reserve(n)
            .map_err(|_| PyErr::new::<errors::BudgetExhausted, _>("token budget exceeded"))
    }

    fn release(&mut self, n: u32) {
        self.inner.release(n);
    }

    fn split(&self, n: u32) -> Vec<Self> {
        self.inner
            .split(n)
            .into_iter()
            .map(|inner| Self { inner })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "TokenBudget(remaining={}, reserved={})",
            self.inner.remaining, self.inner.reserved
        )
    }
}

#[pyclass(name = "TimeBudget", module = "atomr_agents._native.core")]
#[derive(Clone, Copy)]
pub struct PyTimeBudget {
    pub(crate) inner: TimeBudget,
}

#[pymethods]
impl PyTimeBudget {
    #[new]
    fn new(milliseconds: u64) -> Self {
        Self {
            inner: TimeBudget::new(Duration::from_millis(milliseconds)),
        }
    }

    #[getter]
    fn remaining_ms(&self) -> u64 {
        self.inner.remaining_ms
    }

    fn consume_ms(&mut self, ms: u64) -> PyResult<()> {
        self.inner
            .consume(Duration::from_millis(ms))
            .map_err(|_| PyErr::new::<errors::BudgetExhausted, _>("time budget exceeded"))
    }

    fn __repr__(&self) -> String {
        format!("TimeBudget(remaining_ms={})", self.inner.remaining_ms)
    }
}

#[pyclass(name = "MoneyBudget", module = "atomr_agents._native.core")]
#[derive(Clone, Copy)]
pub struct PyMoneyBudget {
    pub(crate) inner: MoneyBudget,
}

#[pymethods]
impl PyMoneyBudget {
    #[new]
    #[pyo3(signature = (usd))]
    fn new(usd: f64) -> Self {
        Self {
            inner: MoneyBudget::from_usd(usd),
        }
    }

    #[getter]
    fn remaining_micro_usd(&self) -> u64 {
        self.inner.remaining_micro_usd
    }

    #[getter]
    fn remaining_usd(&self) -> f64 {
        (self.inner.remaining_micro_usd as f64) / 1_000_000.0
    }

    fn consume_micro(&mut self, micro: u64) -> PyResult<()> {
        self.inner
            .consume_micro(micro)
            .map_err(|_| PyErr::new::<errors::BudgetExhausted, _>("money budget exceeded"))
    }

    fn __repr__(&self) -> String {
        format!("MoneyBudget(remaining_usd={:.6})", self.remaining_usd())
    }
}

#[pyclass(name = "IterationBudget", module = "atomr_agents._native.core")]
#[derive(Clone, Copy)]
pub struct PyIterationBudget {
    pub(crate) inner: IterationBudget,
}

#[pymethods]
impl PyIterationBudget {
    #[new]
    fn new(n: u32) -> Self {
        Self {
            inner: IterationBudget::new(n),
        }
    }

    #[getter]
    fn remaining(&self) -> u32 {
        self.inner.remaining
    }

    fn consume_one(&mut self) -> PyResult<()> {
        self.inner
            .consume_one()
            .map_err(|_| PyErr::new::<errors::BudgetExhausted, _>("iteration budget exceeded"))
    }

    fn __repr__(&self) -> String {
        format!("IterationBudget(remaining={})", self.inner.remaining)
    }
}

// ----- Memory primitives ----------------------------------------------------
//
// Mirrors atomr_agents_core::memory: MemoryItem (id/kind/namespace/
// payload/timestamp_ms/tags), MemoryChunk (retrieval result),
// MemoryKind (Episodic / Semantic / Working / Scratchpad), and
// MemoryNamespace (Agent / Team / Org tagged variants).

#[pyclass(name = "MemoryNamespace", module = "atomr_agents._native.core")]
#[derive(Clone)]
pub struct PyMemoryNamespace {
    pub(crate) inner: MemoryNamespace,
}

#[pymethods]
impl PyMemoryNamespace {
    #[staticmethod]
    fn agent(id: String) -> Self {
        Self {
            inner: MemoryNamespace::Agent(AgentId::from(id)),
        }
    }

    #[staticmethod]
    fn team(id: String) -> Self {
        Self {
            inner: MemoryNamespace::Team(TeamId::from(id)),
        }
    }

    #[staticmethod]
    fn org(id: String) -> Self {
        Self {
            inner: MemoryNamespace::Org(OrgId::from(id)),
        }
    }

    /// `"agent" | "team" | "org"` — discriminator string.
    #[getter]
    fn scope(&self) -> &'static str {
        match self.inner {
            MemoryNamespace::Agent(_) => "agent",
            MemoryNamespace::Team(_) => "team",
            MemoryNamespace::Org(_) => "org",
        }
    }

    #[getter]
    fn id(&self) -> String {
        match &self.inner {
            MemoryNamespace::Agent(i) => i.as_str().to_string(),
            MemoryNamespace::Team(i) => i.as_str().to_string(),
            MemoryNamespace::Org(i) => i.as_str().to_string(),
        }
    }

    fn __repr__(&self) -> String {
        format!("MemoryNamespace({}={:?})", self.scope(), self.id())
    }
}

/// String-tagged `MemoryKind`. Variants: episodic, semantic, working,
/// scratchpad.
#[pyclass(name = "MemoryKind", module = "atomr_agents._native.core", eq, frozen)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PyMemoryKind {
    pub(crate) inner: MemoryKind,
}

#[pymethods]
impl PyMemoryKind {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let inner = match name {
            "episodic" => MemoryKind::Episodic,
            "semantic" => MemoryKind::Semantic,
            "working" => MemoryKind::Working,
            "scratchpad" => MemoryKind::Scratchpad,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown memory kind: {other:?}"
                )));
            }
        };
        Ok(Self { inner })
    }

    #[getter]
    fn name(&self) -> &'static str {
        match self.inner {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Working => "working",
            MemoryKind::Scratchpad => "scratchpad",
        }
    }

    #[staticmethod]
    fn episodic() -> Self {
        Self {
            inner: MemoryKind::Episodic,
        }
    }
    #[staticmethod]
    fn semantic() -> Self {
        Self {
            inner: MemoryKind::Semantic,
        }
    }
    #[staticmethod]
    fn working() -> Self {
        Self {
            inner: MemoryKind::Working,
        }
    }
    #[staticmethod]
    fn scratchpad() -> Self {
        Self {
            inner: MemoryKind::Scratchpad,
        }
    }

    fn __repr__(&self) -> String {
        format!("MemoryKind({:?})", self.name())
    }
}

#[pyclass(name = "MemoryItem", module = "atomr_agents._native.core")]
#[derive(Clone)]
pub struct PyMemoryItem {
    pub(crate) inner: MemoryItem,
}

#[pymethods]
impl PyMemoryItem {
    #[new]
    #[pyo3(signature = (id, kind, namespace, payload, timestamp_ms=None, tags=None))]
    fn new(
        id: String,
        kind: PyMemoryKind,
        namespace: PyMemoryNamespace,
        payload: &Bound<'_, PyAny>,
        timestamp_ms: Option<i64>,
        tags: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let payload_v = py_to_json(payload.py(), payload)?;
        Ok(Self {
            inner: MemoryItem {
                id,
                kind: kind.inner,
                namespace: namespace.inner,
                payload: payload_v,
                timestamp_ms: timestamp_ms.unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                tags: tags.unwrap_or_default(),
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn kind(&self) -> PyMemoryKind {
        PyMemoryKind {
            inner: self.inner.kind,
        }
    }

    #[getter]
    fn namespace(&self) -> PyMemoryNamespace {
        PyMemoryNamespace {
            inner: self.inner.namespace.clone(),
        }
    }

    #[getter]
    fn payload(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.payload)
    }

    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    #[getter]
    fn tags(&self) -> Vec<String> {
        self.inner.tags.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "MemoryItem(id={:?}, kind={:?}, ns_scope={:?}, ts={})",
            self.inner.id,
            self.kind().name(),
            self.namespace().scope(),
            self.inner.timestamp_ms,
        )
    }
}

/// Retrieval-side memory chunk emitted by memory strategies.
#[pyclass(name = "MemoryChunk", module = "atomr_agents._native.core")]
#[derive(Clone)]
pub struct PyMemoryChunk {
    pub(crate) inner: MemoryChunk,
}

#[pymethods]
impl PyMemoryChunk {
    #[new]
    fn new(source_id: String, text: String, score: f32, estimated_tokens: u32) -> Self {
        Self {
            inner: MemoryChunk {
                source_id,
                text,
                score,
                estimated_tokens,
            },
        }
    }

    #[getter]
    fn source_id(&self) -> &str {
        &self.inner.source_id
    }

    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    #[getter]
    fn score(&self) -> f32 {
        self.inner.score
    }

    #[getter]
    fn estimated_tokens(&self) -> u32 {
        self.inner.estimated_tokens
    }

    fn __repr__(&self) -> String {
        format!(
            "MemoryChunk(source_id={:?}, score={:.3}, est_tokens={})",
            self.inner.source_id, self.inner.score, self.inner.estimated_tokens
        )
    }
}

// ----- Inference re-exports (lightweight) ----------------------------------
//
// FinishReason / TokenUsage / Tokens are re-exported from
// atomr_infer_core via atomr_agents_core::inference. We expose them
// here as plain dict-like wrappers so consumers don't need to install
// atomr-infer's Python wheel.

#[pyclass(name = "TokenUsage", module = "atomr_agents._native.core")]
#[derive(Clone, Copy)]
pub struct PyTokenUsage {
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) reasoning_tokens: u32,
    pub(crate) cached_tokens: u32,
}

#[pymethods]
impl PyTokenUsage {
    #[new]
    #[pyo3(signature = (input_tokens=0, output_tokens=0, reasoning_tokens=0, cached_tokens=0))]
    fn new(
        input_tokens: u32,
        output_tokens: u32,
        reasoning_tokens: u32,
        cached_tokens: u32,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            reasoning_tokens,
            cached_tokens,
        }
    }

    #[getter]
    fn input_tokens(&self) -> u32 {
        self.input_tokens
    }
    #[getter]
    fn output_tokens(&self) -> u32 {
        self.output_tokens
    }
    #[getter]
    fn reasoning_tokens(&self) -> u32 {
        self.reasoning_tokens
    }
    #[getter]
    fn cached_tokens(&self) -> u32 {
        self.cached_tokens
    }
    #[getter]
    fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }

    fn __repr__(&self) -> String {
        format!(
            "TokenUsage(input={}, output={}, reasoning={}, cached={})",
            self.input_tokens, self.output_tokens, self.reasoning_tokens, self.cached_tokens
        )
    }
}

impl From<atomr_infer_core::tokens::TokenUsage> for PyTokenUsage {
    fn from(u: atomr_infer_core::tokens::TokenUsage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            reasoning_tokens: u.reasoning_tokens,
            cached_tokens: u.cached_tokens,
        }
    }
}

/// String-tagged `FinishReason`: stop, length, tool_calls, content_filter, error.
#[pyclass(name = "FinishReason", module = "atomr_agents._native.core", eq, hash, frozen)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyFinishReason {
    inner: String,
}

#[pymethods]
impl PyFinishReason {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let valid = ["stop", "length", "tool_calls", "content_filter", "error"];
        if !valid.contains(&name) {
            return Err(PyValueError::new_err(format!(
                "unknown finish reason: {name:?}"
            )));
        }
        Ok(Self {
            inner: name.to_string(),
        })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner
    }

    fn __repr__(&self) -> String {
        format!("FinishReason({:?})", self.inner)
    }
}

impl From<atomr_infer_core::tokens::FinishReason> for PyFinishReason {
    fn from(r: atomr_infer_core::tokens::FinishReason) -> Self {
        // FinishReason is `#[non_exhaustive]` upstream so we keep a
        // wildcard arm. Update mappings as new variants land.
        let s = match r {
            atomr_infer_core::tokens::FinishReason::Stop => "stop",
            atomr_infer_core::tokens::FinishReason::Length => "length",
            atomr_infer_core::tokens::FinishReason::ToolCalls => "tool_calls",
            atomr_infer_core::tokens::FinishReason::ContentFilter => "content_filter",
            atomr_infer_core::tokens::FinishReason::Error => "error",
            _ => "unknown",
        };
        Self {
            inner: s.to_string(),
        }
    }
}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "core")?;
    m.add_class::<PyAgentId>()?;
    m.add_class::<PyTeamId>()?;
    m.add_class::<PyDepartmentId>()?;
    m.add_class::<PyOrgId>()?;
    m.add_class::<PyWorkflowId>()?;
    m.add_class::<PyHarnessId>()?;
    m.add_class::<PyToolId>()?;
    m.add_class::<PyToolSetId>()?;
    m.add_class::<PySkillId>()?;
    m.add_class::<PyPersonaId>()?;
    m.add_class::<PyRunId>()?;
    m.add_class::<PyTokenBudget>()?;
    m.add_class::<PyTimeBudget>()?;
    m.add_class::<PyMoneyBudget>()?;
    m.add_class::<PyIterationBudget>()?;
    m.add_class::<PyMemoryNamespace>()?;
    m.add_class::<PyMemoryKind>()?;
    m.add_class::<PyMemoryItem>()?;
    m.add_class::<PyMemoryChunk>()?;
    m.add_class::<PyTokenUsage>()?;
    m.add_class::<PyFinishReason>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
