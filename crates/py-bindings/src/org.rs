//! `atomr-agents-org` — organizational hierarchy bindings.
//!
//! Wraps:
//! - `Org` / `Department` / `Team` builders that compose `OrgUnit`s
//!   over a children list, an `OrgRoutingStrategy`, a `Policy`, and a
//!   set of granted tool-sets. Each level exposes itself as a
//!   `PyCallable` via `build()`.
//! - The three reference routers — `RoundRobinRouter`,
//!   `LoadAwareRouter`, `CapabilityMatchRouter` — wrapped in a
//!   dedicated `OrgRoutingStrategyHandle` (distinct from the agent
//!   strategy crate's `RoutingStrategy`; org routers are pre-agent and
//!   keyed on the child *label* rather than on `AgentContext`).
//! - `NamespacedMemory` exposed as a `MemoryStore` factory.
//! - `swarm_loop` exposed as a `PyCallable` factory that drives the
//!   active-agent slot until `{"done": true}`.
//! - `ActiveAgent` exposed as a small shared-state data class.

use std::collections::HashMap;
use std::sync::Arc;

use atomr_agents_callable::{CallableHandle, FnCallable};
use atomr_agents_core::{AgentError, AgentId, CallCtx, DepartmentId, OrgId, TeamId, ToolSetId, Value};
use atomr_agents_memory::InMemoryStore;
use atomr_agents_org as org_crate;
use atomr_agents_org::{
    ActiveAgent, CapabilityMatchRouter, LoadAwareRouter, NamespacedMemory, OrgRoutingStrategy,
    RoundRobinRouter,
};
use atomr_agents_strategy::Policy;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::callable::PyCallable;
use crate::core::PyMemoryNamespace;
use crate::memory::PyMemoryStore;
use crate::strategy::PyPolicy;

// Limitation note: the upstream `OrgUnit` is `pub` within
// `atomr_agents_org::team` but the `team` module is private, so the
// struct is not externally constructible. We therefore expose
// builders that record the same configuration shape (children +
// routing + policy + granted toolsets) and produce a `PyCallable`
// via a small in-process adapter that mirrors `OrgUnit::route_and_call`.
// Once the org crate exposes public constructors, the builders can
// swap to the upstream types without changing the Python API.

// ----- Routing strategy handle --------------------------------------------

/// Dyn handle for `atomr_agents_org::OrgRoutingStrategy`. Distinct
/// from the agent-strategy crate's `RoutingStrategy` (different trait
/// shape — picks among children rather than over `AgentContext`).
#[pyclass(name = "OrgRoutingStrategyHandle", module = "atomr_agents._native.org")]
#[derive(Clone)]
pub struct PyOrgRoutingStrategy {
    pub(crate) inner: Arc<dyn OrgRoutingStrategy>,
}

#[pymethods]
impl PyOrgRoutingStrategy {
    fn __repr__(&self) -> String {
        "OrgRoutingStrategyHandle".into()
    }
}

// ----- Router factories ----------------------------------------------------

#[pyfunction]
fn round_robin_router() -> PyOrgRoutingStrategy {
    PyOrgRoutingStrategy {
        inner: Arc::new(RoundRobinRouter::new()),
    }
}

/// Load-aware router. `size` is the expected number of children — it
/// is used to pre-size the in-flight counter. The router resizes the
/// counter on first use if `children.len() != size`.
///
/// Note: the upstream constructor is `LoadAwareRouter::for_size(n)` —
/// the `metric` parameter is reserved for future variants and is
/// ignored in v0.
#[pyfunction]
#[pyo3(signature = (size=0, metric=None))]
fn load_aware_router(size: usize, metric: Option<String>) -> PyOrgRoutingStrategy {
    let _ = metric;
    PyOrgRoutingStrategy {
        inner: Arc::new(LoadAwareRouter::for_size(size)),
    }
}

/// Capability-match router. The router picks the child whose label
/// contains the request's `route` field (case-insensitive substring).
///
/// The `capability_map` parameter is accepted for parity with the
/// task spec (agent-id → capability strings) but is *not* currently
/// consulted by the upstream `CapabilityMatchRouter`, which works
/// solely off the children's labels. The parameter is preserved here
/// as a forward-compatible hook.
#[pyfunction]
#[pyo3(signature = (capability_map=None))]
fn capability_match_router(capability_map: Option<&Bound<'_, PyDict>>) -> PyResult<PyOrgRoutingStrategy> {
    // Validate the shape if provided so callers fail fast.
    if let Some(d) = capability_map {
        for (k, v) in d.iter() {
            let _: String = k.extract()?;
            let _: Vec<String> = v.extract()?;
        }
    }
    Ok(PyOrgRoutingStrategy {
        inner: Arc::new(CapabilityMatchRouter),
    })
}

// ----- Team / Department / Org builders -----------------------------------

/// Builder for a `Team` (mid-level org unit). Configure via
/// `.add_agent(label, callable)`, `.routing(handle)`,
/// `.policy(policy)`, `.grant_toolset(id)`, then `.build()` to obtain
/// a dispatch-ready `Callable`.
#[pyclass(name = "Team", module = "atomr_agents._native.org")]
pub struct PyTeam {
    id: TeamId,
    label: String,
    children: Vec<(String, CallableHandle)>,
    routing: Option<Arc<dyn OrgRoutingStrategy>>,
    policy: Policy,
    granted_toolsets: Vec<ToolSetId>,
}

#[pymethods]
impl PyTeam {
    #[new]
    #[pyo3(signature = (id, label=None))]
    fn new(id: String, label: Option<String>) -> Self {
        let lbl = label.unwrap_or_else(|| id.clone());
        Self {
            id: TeamId::from(id),
            label: lbl,
            children: Vec::new(),
            routing: None,
            policy: Policy::default(),
            granted_toolsets: Vec::new(),
        }
    }

    /// Add a child agent (or any `Callable`) under `label`.
    fn add_agent(&mut self, label: String, agent: PyCallable) {
        self.children.push((label, agent.inner));
    }

    /// Set the routing strategy. Defaults to round-robin if omitted.
    fn routing(&mut self, handle: PyOrgRoutingStrategy) {
        self.routing = Some(handle.inner);
    }

    /// Set the team-level policy. Defaults to `Policy::default()`.
    fn policy(&mut self, policy: PyPolicy) {
        self.policy = policy.inner;
    }

    fn grant_toolset(&mut self, id: String) {
        self.granted_toolsets.push(ToolSetId::from(id));
    }

    #[getter]
    fn id(&self) -> String {
        self.id.as_str().to_string()
    }

    /// Freeze the builder into a `Callable` dispatch handle.
    fn build(&self) -> PyCallable {
        let routing = self
            .routing
            .clone()
            .unwrap_or_else(|| Arc::new(RoundRobinRouter::new()));
        let children = self.children.clone();
        let unit_label = self.label.clone();
        let _policy = self.policy.clone();
        let _toolsets = self.granted_toolsets.clone();
        // Tag the label with the team id so traces match the upstream
        // `Team::label() -> id` behaviour.
        let id_label = self.id.as_str().to_string();
        let h: CallableHandle = Arc::new(FnCallable::labeled(
            Box::leak(id_label.into_boxed_str()),
            move |v: Value, c: CallCtx| {
                let routing = routing.clone();
                let children = children.clone();
                let unit_label = unit_label.clone();
                async move {
                    let label = v.get("route").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if children.is_empty() {
                        return Err(AgentError::Internal(format!("{unit_label}: no children")));
                    }
                    let child = routing.pick(&children, &label).await?;
                    child.call(v, c).await
                }
            },
        ));
        PyCallable::from_handle(h)
    }

    fn __repr__(&self) -> String {
        format!(
            "Team(id={:?}, children={}, label={:?})",
            self.id.as_str(),
            self.children.len(),
            self.label
        )
    }
}

/// Builder for a `Department`. Same shape as `Team` but holds teams
/// (or any `Callable`s) as children.
#[pyclass(name = "Department", module = "atomr_agents._native.org")]
pub struct PyDepartment {
    id: DepartmentId,
    label: String,
    children: Vec<(String, CallableHandle)>,
    routing: Option<Arc<dyn OrgRoutingStrategy>>,
    policy: Policy,
    granted_toolsets: Vec<ToolSetId>,
}

#[pymethods]
impl PyDepartment {
    #[new]
    #[pyo3(signature = (id, label=None))]
    fn new(id: String, label: Option<String>) -> Self {
        let lbl = label.unwrap_or_else(|| id.clone());
        Self {
            id: DepartmentId::from(id),
            label: lbl,
            children: Vec::new(),
            routing: None,
            policy: Policy::default(),
            granted_toolsets: Vec::new(),
        }
    }

    /// Add a child team (or any `Callable`) under `label`.
    fn add_team(&mut self, label: String, team: PyCallable) {
        self.children.push((label, team.inner));
    }

    /// Alias for `add_team` — accepts any `Callable`.
    fn add_child(&mut self, label: String, child: PyCallable) {
        self.children.push((label, child.inner));
    }

    fn routing(&mut self, handle: PyOrgRoutingStrategy) {
        self.routing = Some(handle.inner);
    }

    fn policy(&mut self, policy: PyPolicy) {
        self.policy = policy.inner;
    }

    fn grant_toolset(&mut self, id: String) {
        self.granted_toolsets.push(ToolSetId::from(id));
    }

    #[getter]
    fn id(&self) -> String {
        self.id.as_str().to_string()
    }

    fn build(&self) -> PyCallable {
        let routing = self
            .routing
            .clone()
            .unwrap_or_else(|| Arc::new(CapabilityMatchRouter));
        let children = self.children.clone();
        let unit_label = self.label.clone();
        let id_label = self.id.as_str().to_string();
        let _policy = self.policy.clone();
        let _toolsets = self.granted_toolsets.clone();
        let h: CallableHandle = Arc::new(FnCallable::labeled(
            Box::leak(id_label.into_boxed_str()),
            move |v: Value, c: CallCtx| {
                let routing = routing.clone();
                let children = children.clone();
                let unit_label = unit_label.clone();
                async move {
                    let label = v.get("route").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if children.is_empty() {
                        return Err(AgentError::Internal(format!("{unit_label}: no children")));
                    }
                    let child = routing.pick(&children, &label).await?;
                    child.call(v, c).await
                }
            },
        ));
        PyCallable::from_handle(h)
    }

    fn __repr__(&self) -> String {
        format!(
            "Department(id={:?}, children={}, label={:?})",
            self.id.as_str(),
            self.children.len(),
            self.label
        )
    }
}

/// Builder for an `Org` (top-level unit). Adds an optional
/// `parent_policy` for policy narrowing.
#[pyclass(name = "Org", module = "atomr_agents._native.org")]
pub struct PyOrg {
    id: OrgId,
    label: String,
    children: Vec<(String, CallableHandle)>,
    routing: Option<Arc<dyn OrgRoutingStrategy>>,
    policy: Policy,
    granted_toolsets: Vec<ToolSetId>,
    parent_policy: Option<Policy>,
}

#[pymethods]
impl PyOrg {
    #[new]
    #[pyo3(signature = (id, label=None))]
    fn new(id: String, label: Option<String>) -> Self {
        let lbl = label.unwrap_or_else(|| id.clone());
        Self {
            id: OrgId::from(id),
            label: lbl,
            children: Vec::new(),
            routing: None,
            policy: Policy::default(),
            granted_toolsets: Vec::new(),
            parent_policy: None,
        }
    }

    fn add_department(&mut self, label: String, dept: PyCallable) {
        self.children.push((label, dept.inner));
    }

    fn add_child(&mut self, label: String, child: PyCallable) {
        self.children.push((label, child.inner));
    }

    fn routing(&mut self, handle: PyOrgRoutingStrategy) {
        self.routing = Some(handle.inner);
    }

    fn policy(&mut self, policy: PyPolicy) {
        self.policy = policy.inner;
    }

    fn parent_policy(&mut self, policy: PyPolicy) {
        self.parent_policy = Some(policy.inner);
    }

    fn grant_toolset(&mut self, id: String) {
        self.granted_toolsets.push(ToolSetId::from(id));
    }

    /// Returns the policy resolved against the parent policy (if any).
    /// Mirrors `Org::resolved_policy()`.
    fn resolved_policy(&self) -> PyPolicy {
        let resolved = match &self.parent_policy {
            Some(p) => Policy::narrow(p, &self.policy),
            None => self.policy.clone(),
        };
        PyPolicy { inner: resolved }
    }

    #[getter]
    fn id(&self) -> String {
        self.id.as_str().to_string()
    }

    fn build(&self) -> PyCallable {
        let routing = self
            .routing
            .clone()
            .unwrap_or_else(|| Arc::new(CapabilityMatchRouter));
        let children = self.children.clone();
        let unit_label = self.label.clone();
        let id_label = self.id.as_str().to_string();
        let _policy = self.policy.clone();
        let _toolsets = self.granted_toolsets.clone();
        let h: CallableHandle = Arc::new(FnCallable::labeled(
            Box::leak(id_label.into_boxed_str()),
            move |v: Value, c: CallCtx| {
                let routing = routing.clone();
                let children = children.clone();
                let unit_label = unit_label.clone();
                async move {
                    let label = v.get("route").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if children.is_empty() {
                        return Err(AgentError::Internal(format!("{unit_label}: no children")));
                    }
                    let child = routing.pick(&children, &label).await?;
                    child.call(v, c).await
                }
            },
        ));
        PyCallable::from_handle(h)
    }

    fn __repr__(&self) -> String {
        format!(
            "Org(id={:?}, children={}, label={:?})",
            self.id.as_str(),
            self.children.len(),
            self.label
        )
    }
}

// ----- NamespacedMemory ---------------------------------------------------

/// Build a `MemoryStore` that namespaces reads/writes by the
/// org/team/agent triple. Writes to other namespaces are rejected
/// per upstream `NamespacedMemory` rules.
///
/// `namespace` must be an `Agent` namespace — its id is taken as the
/// agent id. `team` and `org` are optional override ids; if omitted,
/// the parent store is wrapped under a default org id (`"default"`)
/// and no team binding.
///
/// Limitations: `NamespacedMemory` writes go to a fresh
/// `InMemoryStore`; the `parent` `MemoryStore` is ignored at the
/// backing layer in this v0 (matching the upstream constructor,
/// which always allocates a fresh `InMemoryStore`). To share state
/// across agents, share the `MemoryStore` indirectly via the same
/// `OrgId` namespace once cascaded reads cover external stores.
#[pyfunction]
#[pyo3(signature = (parent, namespace, org_id=None, team_id=None, allow_team_write=false))]
fn namespaced_memory(
    parent: PyMemoryStore,
    namespace: PyMemoryNamespace,
    org_id: Option<String>,
    team_id: Option<String>,
    allow_team_write: bool,
) -> PyResult<PyMemoryStore> {
    let _ = parent; // upstream allocates its own backing store in v0
    let agent_id = match &namespace.inner {
        atomr_agents_core::MemoryNamespace::Agent(a) => AgentId::from(a.as_str().to_string()),
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "namespaced_memory requires an Agent namespace, got {:?}",
                other
            )));
        }
    };
    let org = OrgId::from(org_id.unwrap_or_else(|| "default".into()));
    let mut nm = NamespacedMemory::new(org, agent_id);
    if let Some(t) = team_id {
        nm = nm.with_team(TeamId::from(t));
    }
    nm = nm.with_team_write(allow_team_write);
    Ok(PyMemoryStore { inner: Arc::new(nm) })
}

// ----- ActiveAgent + swarm_loop -------------------------------------------

/// Shared mutable slot used by swarm/network patterns to indicate
/// which agent should run next.
#[pyclass(name = "ActiveAgent", module = "atomr_agents._native.org")]
#[derive(Clone)]
pub struct PyActiveAgent {
    pub(crate) inner: ActiveAgent,
}

#[pymethods]
impl PyActiveAgent {
    #[new]
    fn new(initial: String) -> Self {
        Self {
            inner: ActiveAgent::new(initial),
        }
    }

    fn get(&self) -> String {
        self.inner.get()
    }

    fn set(&self, value: String) {
        self.inner.set(value);
    }

    fn __repr__(&self) -> String {
        format!("ActiveAgent(current={:?})", self.inner.get())
    }
}

/// Build a `Callable` that drives `swarm_loop` over the supplied
/// agents. The returned callable takes the initial input and runs
/// until the active agent emits `{"done": true}` or `max_steps`
/// iterations elapse.
///
/// `initial_agent` is the first agent label (looked up by its
/// `Callable.label`); `agents` is the pool of candidate agents, each
/// identified by its label.
#[pyfunction]
#[pyo3(signature = (initial_agent, agents, max_steps=16))]
fn swarm_loop(initial_agent: String, agents: Vec<PyCallable>, max_steps: u32) -> PyResult<PyCallable> {
    let mut map: HashMap<String, CallableHandle> = HashMap::new();
    for a in agents.into_iter() {
        let label = a.inner.label().to_string();
        map.insert(label, a.inner);
    }
    if !map.contains_key(&initial_agent) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "swarm_loop: initial_agent {:?} not present in agents pool",
            initial_agent
        )));
    }
    let map = Arc::new(map);
    let initial = initial_agent.clone();
    let h: CallableHandle = Arc::new(FnCallable::labeled(
        Box::leak(format!("swarm:{initial}").into_boxed_str()),
        move |input: Value, _ctx: CallCtx| {
            let map = map.clone();
            let initial = initial.clone();
            async move {
                let active = ActiveAgent::new(initial);
                org_crate::swarm_loop(&map, &active, input, max_steps).await
            }
        },
    ));
    Ok(PyCallable::from_handle(h))
}

// Keep helper imports referenced in case of future use.
#[allow(dead_code)]
fn _force_used(_s: &InMemoryStore) {}

// ----- Module registration ------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "org")?;
    m.add_class::<PyTeam>()?;
    m.add_class::<PyDepartment>()?;
    m.add_class::<PyOrg>()?;
    m.add_class::<PyOrgRoutingStrategy>()?;
    m.add_class::<PyActiveAgent>()?;
    m.add_function(wrap_pyfunction!(round_robin_router, &m)?)?;
    m.add_function(wrap_pyfunction!(load_aware_router, &m)?)?;
    m.add_function(wrap_pyfunction!(capability_match_router, &m)?)?;
    m.add_function(wrap_pyfunction!(namespaced_memory, &m)?)?;
    m.add_function(wrap_pyfunction!(swarm_loop, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
