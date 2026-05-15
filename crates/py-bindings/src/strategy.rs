//! Strategy value types + dyn-handle adapters for Python guests.
//!
//! Each trait in `atomr-agents-strategy` (`LoopStrategy`,
//! `TerminationStrategy`, `MemoryStrategy`, `SkillStrategy`,
//! `ToolStrategy`, `RoutingStrategy`, `PolicyStrategy`) has a Python
//! adapter that wraps a `PyObject` and dispatches the trait methods
//! to the Python target.
//!
//! Enum/value types (`Termination`, `RoutingTarget`, `SkillRef`,
//! `ToolRef`, `Policy`, `PolicyDecision`) are concrete `#[pyclass]`es.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::Callable;
use atomr_agents_core::{
    AgentContext, AgentError, CallCtx, MemoryChunk, MemoryItem, MemoryNamespace, Result as AgentResult,
    SkillId, TokenBudget, ToolId, ToolSetId, Value,
};
use atomr_agents_strategy::{
    MemoryStrategy, Policy, PolicyDecision, PolicyStrategy, RoutingStrategy, RoutingTarget, SkillRef,
    SkillStrategy, Termination, ToolRef, ToolStrategy,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::callable::PyCallable;
use crate::conv::{json_to_py, py_to_json};

// ----- Termination ---------------------------------------------------------

/// `Termination` — `Continue` or `Done(reason)`.
#[pyclass(name = "Termination", module = "atomr_agents._native.strategy", eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyTermination {
    pub(crate) kind: String,
    pub(crate) reason: Option<String>,
}

#[pymethods]
impl PyTermination {
    #[staticmethod]
    fn continue_() -> Self {
        Self {
            kind: "continue".into(),
            reason: None,
        }
    }

    #[staticmethod]
    fn done(reason: String) -> Self {
        Self {
            kind: "done".into(),
            reason: Some(reason),
        }
    }

    #[getter]
    fn kind(&self) -> &str {
        &self.kind
    }

    #[getter]
    fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    fn is_done(&self) -> bool {
        self.kind == "done"
    }

    fn __repr__(&self) -> String {
        match &self.reason {
            Some(r) => format!("Termination(done={r:?})"),
            None => "Termination(continue)".into(),
        }
    }
}

impl PyTermination {
    pub(crate) fn into_inner(self) -> Termination {
        match self.kind.as_str() {
            "done" => {
                let r = self.reason.unwrap_or_else(|| "done".into());
                Termination::Done(Box::leak(r.into_boxed_str()))
            }
            _ => Termination::Continue,
        }
    }
}

impl From<Termination> for PyTermination {
    fn from(t: Termination) -> Self {
        match t {
            Termination::Continue => Self {
                kind: "continue".into(),
                reason: None,
            },
            Termination::Done(r) => Self {
                kind: "done".into(),
                reason: Some(r.to_string()),
            },
        }
    }
}

// ----- RoutingTarget -------------------------------------------------------

#[pyclass(name = "RoutingTarget", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyRoutingTarget {
    pub(crate) inner: RoutingTarget,
}

#[pymethods]
impl PyRoutingTarget {
    #[new]
    fn new(label: String, handle: PyCallable) -> Self {
        Self {
            inner: RoutingTarget {
                label,
                handle: handle.inner,
            },
        }
    }

    #[getter]
    fn label(&self) -> &str {
        &self.inner.label
    }

    #[getter]
    fn handle(&self) -> PyCallable {
        PyCallable::from_handle(self.inner.handle.clone())
    }

    fn __repr__(&self) -> String {
        format!("RoutingTarget(label={:?})", self.inner.label)
    }
}

// ----- SkillRef ------------------------------------------------------------

#[pyclass(name = "SkillRef", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PySkillRef {
    pub(crate) inner: SkillRef,
}

#[pymethods]
impl PySkillRef {
    #[new]
    #[pyo3(signature = (id, name, priority=0))]
    fn new(id: String, name: String, priority: u8) -> Self {
        Self {
            inner: SkillRef {
                id: SkillId::from(id),
                name,
                priority,
            },
        }
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    fn priority(&self) -> u8 {
        self.inner.priority
    }

    fn __repr__(&self) -> String {
        format!(
            "SkillRef(id={:?}, name={:?}, priority={})",
            self.inner.id.as_str(),
            self.inner.name,
            self.inner.priority
        )
    }
}

// ----- ToolRef -------------------------------------------------------------

#[pyclass(name = "ToolRef", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyToolRef {
    pub(crate) inner: ToolRef,
}

#[pymethods]
impl PyToolRef {
    #[new]
    fn new(id: String, name: String, handle: PyCallable) -> Self {
        Self {
            inner: ToolRef {
                id: ToolId::from(id),
                name,
                handle: handle.inner,
            },
        }
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    fn handle(&self) -> PyCallable {
        PyCallable::from_handle(self.inner.handle.clone())
    }

    fn __repr__(&self) -> String {
        format!(
            "ToolRef(id={:?}, name={:?})",
            self.inner.id.as_str(),
            self.inner.name
        )
    }
}

// ----- Policy / PolicyDecision --------------------------------------------

#[pyclass(name = "Policy", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyPolicy {
    pub(crate) inner: Policy,
}

#[pymethods]
impl PyPolicy {
    #[new]
    #[pyo3(signature = (allowed_toolsets=Vec::new(), max_tokens_per_call=None, max_money_micro_usd_per_call=None, allowed_models=Vec::new()))]
    fn new(
        allowed_toolsets: Vec<String>,
        max_tokens_per_call: Option<u32>,
        max_money_micro_usd_per_call: Option<u64>,
        allowed_models: Vec<String>,
    ) -> Self {
        Self {
            inner: Policy {
                allowed_toolsets: allowed_toolsets.into_iter().map(ToolSetId::from).collect(),
                max_tokens_per_call,
                max_money_micro_usd_per_call,
                allowed_models,
            },
        }
    }

    #[getter]
    fn allowed_toolsets(&self) -> Vec<String> {
        self.inner
            .allowed_toolsets
            .iter()
            .map(|t| t.as_str().to_string())
            .collect()
    }

    #[getter]
    fn allowed_models(&self) -> Vec<String> {
        self.inner.allowed_models.clone()
    }

    #[getter]
    fn max_tokens_per_call(&self) -> Option<u32> {
        self.inner.max_tokens_per_call
    }

    #[getter]
    fn max_money_micro_usd_per_call(&self) -> Option<u64> {
        self.inner.max_money_micro_usd_per_call
    }

    /// Narrow `self` against a child policy (intersection of grants).
    fn narrow(&self, child: &PyPolicy) -> PyPolicy {
        PyPolicy {
            inner: Policy::narrow(&self.inner, &child.inner),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Policy(toolsets={}, models={}, tokens={:?}, micro_usd={:?})",
            self.inner.allowed_toolsets.len(),
            self.inner.allowed_models.len(),
            self.inner.max_tokens_per_call,
            self.inner.max_money_micro_usd_per_call,
        )
    }
}

#[pyclass(name = "PolicyDecision", module = "atomr_agents._native.strategy", eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyPolicyDecision {
    pub(crate) kind: String,
    pub(crate) reason: Option<String>,
}

#[pymethods]
impl PyPolicyDecision {
    #[staticmethod]
    fn allow() -> Self {
        Self {
            kind: "allow".into(),
            reason: None,
        }
    }
    #[staticmethod]
    fn deny(reason: String) -> Self {
        Self {
            kind: "deny".into(),
            reason: Some(reason),
        }
    }
    #[getter]
    fn kind(&self) -> &str {
        &self.kind
    }
    #[getter]
    fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    fn __repr__(&self) -> String {
        match &self.reason {
            Some(r) => format!("PolicyDecision(deny={r:?})"),
            None => "PolicyDecision(allow)".into(),
        }
    }
}

impl From<PolicyDecision> for PyPolicyDecision {
    fn from(d: PolicyDecision) -> Self {
        match d {
            PolicyDecision::Allow => PyPolicyDecision::allow(),
            PolicyDecision::Deny(r) => PyPolicyDecision::deny(r),
        }
    }
}

impl PyPolicyDecision {
    pub(crate) fn into_inner(self) -> PolicyDecision {
        match self.kind.as_str() {
            "deny" => PolicyDecision::Deny(self.reason.unwrap_or_default()),
            _ => PolicyDecision::Allow,
        }
    }
}

// ----- Dyn strategy handles (target-erased) --------------------------------

#[pyclass(name = "MemoryStrategyHandle", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyMemoryStrategy {
    pub(crate) inner: Arc<dyn MemoryStrategy>,
}

#[pyclass(name = "SkillStrategyHandle", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PySkillStrategy {
    pub(crate) inner: Arc<dyn SkillStrategy>,
}

#[pyclass(name = "ToolStrategyHandle", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyToolStrategy {
    pub(crate) inner: Arc<dyn ToolStrategy>,
}

#[pyclass(name = "RoutingStrategyHandle", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyRoutingStrategy {
    pub(crate) inner: Arc<dyn RoutingStrategy>,
}

#[pyclass(name = "PolicyStrategyHandle", module = "atomr_agents._native.strategy")]
#[derive(Clone)]
pub struct PyPolicyStrategy {
    pub(crate) inner: Arc<dyn PolicyStrategy>,
}

#[pymethods]
impl PyMemoryStrategy {
    fn __repr__(&self) -> String {
        "MemoryStrategyHandle".into()
    }
}
#[pymethods]
impl PySkillStrategy {
    fn __repr__(&self) -> String {
        "SkillStrategyHandle".into()
    }
}
#[pymethods]
impl PyToolStrategy {
    fn __repr__(&self) -> String {
        "ToolStrategyHandle".into()
    }
}
#[pymethods]
impl PyRoutingStrategy {
    fn __repr__(&self) -> String {
        "RoutingStrategyHandle".into()
    }
}
#[pymethods]
impl PyPolicyStrategy {
    fn __repr__(&self) -> String {
        "PolicyStrategyHandle".into()
    }
}

// ----- Common helper: build AgentContext dict for python callbacks --------

pub(crate) fn agent_context_to_pydict<'py>(
    py: Python<'py>,
    ctx: &AgentContext,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    d.set_item("agent_id", ctx.agent_id.as_str())?;
    if let Some(t) = &ctx.team_id {
        d.set_item("team_id", t.as_str())?;
    }
    if let Some(o) = &ctx.org_id {
        d.set_item("org_id", o.as_str())?;
    }
    let turn = PyDict::new_bound(py);
    turn.set_item("user", &ctx.turn.user)?;
    let history = PyList::empty_bound(py);
    for m in &ctx.turn.history {
        let row = PyDict::new_bound(py);
        let role = match m.role {
            atomr_agents_core::MessageRole::System => "system",
            atomr_agents_core::MessageRole::User => "user",
            atomr_agents_core::MessageRole::Assistant => "assistant",
            atomr_agents_core::MessageRole::Tool => "tool",
        };
        row.set_item("role", role)?;
        row.set_item("content", &m.content)?;
        history.append(row)?;
    }
    turn.set_item("history", history)?;
    d.set_item("turn", turn)?;
    Ok(d)
}

// ----- Adapters ------------------------------------------------------------

pub(crate) struct PyMemoryStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl MemoryStrategy for PyMemoryStrategyAdapter {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<Vec<MemoryChunk>> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("retrieve")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("retrieve")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("memory strategy retrieve: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        let v = Python::with_gil(|py| py_to_json(py, final_val.bind(py)))
            .map_err(|e| AgentError::Internal(format!("memory strategy result: {e}")))?;
        let chunks: Vec<MemoryChunk> = match v {
            Value::Array(arr) => arr
                .into_iter()
                .filter_map(|item| {
                    let m = item.as_object()?;
                    Some(MemoryChunk {
                        source_id: m.get("source_id")?.as_str()?.to_string(),
                        text: m.get("text")?.as_str().unwrap_or_default().to_string(),
                        score: m.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                        estimated_tokens: m.get("estimated_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                            as u32,
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(chunks)
    }

    async fn store(&self, item: MemoryItem) -> AgentResult<()> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let dict = PyDict::new_bound(py);
            dict.set_item("id", &item.id)?;
            dict.set_item(
                "kind",
                match item.kind {
                    atomr_agents_core::MemoryKind::Episodic => "episodic",
                    atomr_agents_core::MemoryKind::Semantic => "semantic",
                    atomr_agents_core::MemoryKind::Working => "working",
                    atomr_agents_core::MemoryKind::Scratchpad => "scratchpad",
                },
            )?;
            let ns_dict = PyDict::new_bound(py);
            let (scope, ns_id) = match &item.namespace {
                MemoryNamespace::Agent(i) => ("agent", i.as_str()),
                MemoryNamespace::Team(i) => ("team", i.as_str()),
                MemoryNamespace::Org(i) => ("org", i.as_str()),
            };
            ns_dict.set_item("scope", scope)?;
            ns_dict.set_item("id", ns_id)?;
            dict.set_item("namespace", ns_dict)?;
            dict.set_item("payload", json_to_py(py, &item.payload)?)?;
            dict.set_item("timestamp_ms", item.timestamp_ms)?;
            dict.set_item("tags", item.tags.clone())?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("store")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("store")?.call1((dict,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("memory strategy store: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }
}

pub(crate) struct PySkillStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl SkillStrategy for PySkillStrategyAdapter {
    async fn applicable(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<Vec<SkillRef>> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("applicable")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("applicable")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("skill strategy applicable: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        let v = Python::with_gil(|py| py_to_json(py, final_val.bind(py)))
            .map_err(|e| AgentError::Internal(format!("skill strategy result: {e}")))?;
        let refs: Vec<SkillRef> = match v {
            Value::Array(arr) => arr
                .into_iter()
                .filter_map(|item| {
                    let m = item.as_object()?;
                    Some(SkillRef {
                        id: SkillId::from(m.get("id")?.as_str()?.to_string()),
                        name: m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        priority: m.get("priority").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(refs)
    }
}

pub(crate) struct PyToolStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl ToolStrategy for PyToolStrategyAdapter {
    async fn select(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<Vec<ToolRef>> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("select")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("select")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("tool strategy select: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        // The Python target is expected to return a list of ToolRef.
        // We don't try to round-trip CallableHandle through JSON; the
        // Python side returns PyToolRef instances that we extract.
        let refs: Vec<ToolRef> = Python::with_gil(|py| -> PyResult<Vec<ToolRef>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let item = item?;
                let r: PyToolRef = item.extract()?;
                out.push(r.inner);
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("tool strategy refs: {e}")))?;
        Ok(refs)
    }
}

pub(crate) struct PyRoutingStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl RoutingStrategy for PyRoutingStrategyAdapter {
    async fn route(&self, ctx: &AgentContext) -> AgentResult<RoutingTarget> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("route")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("route")?.call1((ctx_dict,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("routing strategy: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        let target = Python::with_gil(|py| -> PyResult<RoutingTarget> {
            let r: PyRoutingTarget = final_val.bind(py).extract()?;
            Ok(r.inner)
        })
        .map_err(|e| AgentError::Internal(format!("routing strategy result: {e}")))?;
        Ok(target)
    }
}

pub(crate) struct PyPolicyStrategyAdapter {
    target: Arc<PyObject>,
}

impl PolicyStrategy for PyPolicyStrategyAdapter {
    fn evaluate(
        &self,
        policy: &Policy,
        requested_toolset: Option<&ToolSetId>,
    ) -> AgentResult<PolicyDecision> {
        let target = self.target.clone();
        let policy_clone = policy.clone();
        let requested = requested_toolset.map(|t| t.as_str().to_string());
        Python::with_gil(|py| -> PyResult<PolicyDecision> {
            let bound = target.bind(py);
            let py_policy = PyPolicy { inner: policy_clone };
            let arg_policy = Py::new(py, py_policy)?;
            let arg_ts = match &requested {
                Some(s) => s.clone().into_py(py),
                None => py.None(),
            };
            let instance: Bound<'_, PyAny> = if bound.hasattr("evaluate")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("evaluate")?.call1((arg_policy, arg_ts))?;
            let pd: PyPolicyDecision = r.extract()?;
            Ok(pd.into_inner())
        })
        .map_err(|e| AgentError::Internal(format!("policy strategy: {e}")))
    }
}

// ----- Await-if-coroutine helper (shared across adapters) ------------------

pub(crate) async fn await_if_coro(value: PyObject) -> AgentResult<PyObject> {
    let maybe_future = Python::with_gil(|py| -> PyResult<Option<_>> {
        let bound = value.bind(py);
        let inspect = py.import_bound("inspect")?;
        let iscoroutine = inspect.getattr("iscoroutine")?;
        let is_coro: bool = iscoroutine.call1((bound,))?.extract()?;
        if is_coro {
            let fut = pyo3_async_runtimes::tokio::into_future(bound.clone())?;
            Ok(Some(fut))
        } else {
            Ok(None)
        }
    })
    .map_err(|e| AgentError::Internal(format!("inspect coroutine: {e}")))?;
    match maybe_future {
        Some(fut) => fut.await.map_err(|e| AgentError::Internal(format!("await: {e}"))),
        None => Ok(value),
    }
}

// ----- Factory functions ---------------------------------------------------

#[pyfunction]
fn memory_strategy_from_factory(key: String) -> PyResult<PyMemoryStrategy> {
    let target = crate::guest::must_lookup("strategy:memory", &key)
        .or_else(|_| crate::guest::must_lookup("memory", &key))?;
    Ok(PyMemoryStrategy {
        inner: Arc::new(PyMemoryStrategyAdapter { target }),
    })
}

#[pyfunction]
fn skill_strategy_from_factory(key: String) -> PyResult<PySkillStrategy> {
    let target = crate::guest::must_lookup("strategy:skill", &key)
        .or_else(|_| crate::guest::must_lookup("skill", &key))?;
    Ok(PySkillStrategy {
        inner: Arc::new(PySkillStrategyAdapter { target }),
    })
}

#[pyfunction]
fn tool_strategy_from_factory(key: String) -> PyResult<PyToolStrategy> {
    let target = crate::guest::must_lookup("strategy:tool", &key)
        .or_else(|_| crate::guest::must_lookup("tool", &key))?;
    Ok(PyToolStrategy {
        inner: Arc::new(PyToolStrategyAdapter { target }),
    })
}

#[pyfunction]
fn routing_strategy_from_factory(key: String) -> PyResult<PyRoutingStrategy> {
    let target = crate::guest::must_lookup("strategy:routing", &key)?;
    Ok(PyRoutingStrategy {
        inner: Arc::new(PyRoutingStrategyAdapter { target }),
    })
}

#[pyfunction]
fn policy_strategy_from_factory(key: String) -> PyResult<PyPolicyStrategy> {
    let target = crate::guest::must_lookup("strategy:policy", &key)?;
    Ok(PyPolicyStrategy {
        inner: Arc::new(PyPolicyStrategyAdapter { target }),
    })
}

// Keep symbols referenced (for clippy/dead-code).
#[allow(dead_code)]
fn _force_used(_c: &CallCtx, _v: &Value) {}

#[allow(dead_code)]
fn _is_callable_used(_c: &dyn Callable) {}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "strategy")?;
    m.add_class::<PyTermination>()?;
    m.add_class::<PyRoutingTarget>()?;
    m.add_class::<PySkillRef>()?;
    m.add_class::<PyToolRef>()?;
    m.add_class::<PyPolicy>()?;
    m.add_class::<PyPolicyDecision>()?;
    m.add_class::<PyMemoryStrategy>()?;
    m.add_class::<PySkillStrategy>()?;
    m.add_class::<PyToolStrategy>()?;
    m.add_class::<PyRoutingStrategy>()?;
    m.add_class::<PyPolicyStrategy>()?;
    m.add_function(wrap_pyfunction!(memory_strategy_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(skill_strategy_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(tool_strategy_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(routing_strategy_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(policy_strategy_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
