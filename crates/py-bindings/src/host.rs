//! PyO3 bindings for the `atomr-agents-host` runtime.
//!
//! Exposed as `atomr_agents._native.host`. Covers loader (M1),
//! chat/router (M2), memory-sync/rules (M3), skills (M4), hooks (M5),
//! scheduler (M6), gateway (M7), mcp (M8), curator+events (M9),
//! branching (M10), registry cache (M11), evals (M12).

#![allow(non_local_definitions)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList, PyModule};

use atomr_agents_host as host;
use host::{
    branching, chat as host_chat, config as host_config, curator, evals as host_evals, events,
    gateway as host_gateway, hooks as host_hooks, layout, loader, markdown as host_markdown, mcp,
    memory_sync, registry_cache, routes, runtime as host_runtime, scheduler as host_scheduler,
    skills_registry,
};

// =========================================================================
// JSON <-> Python helpers
// =========================================================================

fn py_to_json(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    crate::conv::py_to_json(py, v)
}

fn json_to_py<'py>(py: Python<'py>, v: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    let obj = crate::conv::json_to_py(py, v)?;
    Ok(obj.into_bound(py))
}

fn map_err<E: std::fmt::Display>(e: E) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn map_value_err<E: std::fmt::Display>(e: E) -> PyErr {
    PyValueError::new_err(e.to_string())
}

// =========================================================================
// Layout: HostPaths, AgentPaths
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "HostPaths", frozen)]
#[derive(Clone)]
pub struct PyHostPaths {
    inner: layout::HostPaths,
}

#[pymethods]
impl PyHostPaths {
    #[new]
    fn new(root: PathBuf) -> Self {
        Self { inner: layout::HostPaths::new(root) }
    }
    #[getter]
    fn root(&self) -> PathBuf { self.inner.root.clone() }
    #[getter]
    fn config_yaml(&self) -> PathBuf { self.inner.config_yaml() }
    #[getter]
    fn agents_md(&self) -> PathBuf { self.inner.agents_md() }
    #[getter]
    fn agents_dir(&self) -> PathBuf { self.inner.agents_dir() }
    #[getter]
    fn channels_dir(&self) -> PathBuf { self.inner.channels_dir() }
    #[getter]
    fn crons_dir(&self) -> PathBuf { self.inner.crons_dir() }
    #[getter]
    fn tools_dir(&self) -> PathBuf { self.inner.tools_dir() }
    #[getter]
    fn registry_dir(&self) -> PathBuf { self.inner.registry_dir() }
    #[getter]
    fn evals_dir(&self) -> PathBuf { self.inner.evals_dir() }
    #[getter]
    fn mcp_dir(&self) -> PathBuf { self.inner.mcp_dir() }
    #[getter]
    fn events_jsonl(&self) -> PathBuf { self.inner.events_jsonl() }
    fn agent(&self, agent_id: &str) -> PyAgentPaths {
        PyAgentPaths { inner: self.inner.agent(agent_id) }
    }
    fn list_agent_ids(&self) -> Vec<String> { self.inner.list_agent_ids() }
    fn ensure(&self) -> PyResult<()> { self.inner.ensure().map_err(map_err) }
    fn __repr__(&self) -> String { format!("HostPaths(root={})", self.inner.root.display()) }
}

#[pyclass(module = "atomr_agents._native.host", name = "AgentPaths", frozen)]
#[derive(Clone)]
pub struct PyAgentPaths {
    inner: layout::AgentPaths,
}

#[pymethods]
impl PyAgentPaths {
    #[new]
    fn new(root: PathBuf, agent_id: String) -> Self {
        Self { inner: layout::AgentPaths { root, agent_id } }
    }
    #[getter] fn root(&self) -> PathBuf { self.inner.root.clone() }
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id.clone() }
    #[getter] fn dir(&self) -> PathBuf { self.inner.dir() }
    #[getter] fn agent_yaml(&self) -> PathBuf { self.inner.agent_yaml() }
    #[getter] fn soul_md(&self) -> PathBuf { self.inner.soul_md() }
    #[getter] fn rules_md(&self) -> PathBuf { self.inner.rules_md() }
    #[getter] fn memory_md(&self) -> PathBuf { self.inner.memory_md() }
    #[getter] fn user_md(&self) -> PathBuf { self.inner.user_md() }
    #[getter] fn skills_dir(&self) -> PathBuf { self.inner.skills_dir() }
    #[getter] fn hooks_dir(&self) -> PathBuf { self.inner.hooks_dir() }
    #[getter] fn state_dir(&self) -> PathBuf { self.inner.state_dir() }
    #[getter] fn threads_dir(&self) -> PathBuf { self.inner.threads_dir() }
    #[getter] fn checkpoints_dir(&self) -> PathBuf { self.inner.checkpoints_dir() }
    #[getter] fn memory_db(&self) -> PathBuf { self.inner.memory_db() }
    fn ensure(&self) -> PyResult<()> { self.inner.ensure().map_err(map_err) }
    fn __repr__(&self) -> String {
        format!("AgentPaths(root={}, agent_id={})", self.inner.root.display(), self.inner.agent_id)
    }
}

#[pyfunction(name = "default_root")]
fn py_default_root() -> PathBuf { layout::default_root() }

// =========================================================================
// Config
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "ProviderConfig", frozen)]
#[derive(Clone)]
pub struct PyProviderConfig {
    inner: host_config::ProviderConfig,
}

#[pymethods]
impl PyProviderConfig {
    #[new]
    #[pyo3(signature = (name, kind, api_key_env=None, base_url=None, extra=None))]
    fn new(py: Python<'_>, name: String, kind: String, api_key_env: Option<String>, base_url: Option<String>, extra: Option<Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut x = std::collections::BTreeMap::new();
        if let Some(d) = extra {
            for (k, v) in d.iter() {
                let ks: String = k.extract()?;
                x.insert(ks, py_to_json(py, &v)?);
            }
        }
        Ok(Self { inner: host_config::ProviderConfig { name, kind, api_key_env, base_url, extra: x } })
    }
    #[getter] fn name(&self) -> String { self.inner.name.clone() }
    #[getter] fn kind(&self) -> String { self.inner.kind.clone() }
    #[getter] fn api_key_env(&self) -> Option<String> { self.inner.api_key_env.clone() }
    #[getter] fn base_url(&self) -> Option<String> { self.inner.base_url.clone() }
    #[getter]
    fn extra<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.extra {
            d.set_item(k, json_to_py(py, v)?)?;
        }
        Ok(d)
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "HostConfig", frozen)]
#[derive(Clone)]
pub struct PyHostConfig {
    inner: host_config::HostConfig,
}

#[pymethods]
impl PyHostConfig {
    #[staticmethod]
    fn load_default() -> PyResult<Self> {
        let inner = host_config::HostConfig::load_default().map_err(map_err)?;
        Ok(Self { inner })
    }
    #[staticmethod]
    fn load(root: PathBuf) -> PyResult<Self> {
        let inner = host_config::HostConfig::load(root).map_err(map_err)?;
        Ok(Self { inner })
    }
    #[getter] fn paths(&self) -> PyHostPaths { PyHostPaths { inner: self.inner.paths.clone() } }
    #[getter] fn version(&self) -> u32 { self.inner.version }
    #[getter] fn default_agent(&self) -> Option<String> { self.inner.default_agent.clone() }
    #[getter] fn default_model(&self) -> Option<String> { self.inner.default_model.clone() }
    #[getter]
    fn providers<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (name, p) in &self.inner.providers {
            let py_p = PyProviderConfig { inner: p.clone() };
            d.set_item(name, Py::new(py, py_p)?)?;
        }
        Ok(d)
    }
    #[getter]
    fn extra<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.extra {
            d.set_item(k, json_to_py(py, v)?)?;
        }
        Ok(d)
    }
    fn to_yaml_string(&self) -> PyResult<String> { self.inner.to_yaml_string().map_err(map_err) }
}

// =========================================================================
// Markdown
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "MarkdownDoc", frozen)]
#[derive(Clone)]
pub struct PyMarkdownDoc {
    inner: host_markdown::MarkdownDoc,
}

#[pymethods]
impl PyMarkdownDoc {
    #[staticmethod]
    fn read(path: PathBuf) -> PyResult<Self> {
        let inner = host_markdown::MarkdownDoc::read(&path).map_err(map_err)?;
        Ok(Self { inner })
    }
    #[staticmethod]
    fn parse_str(text: &str) -> PyResult<Self> {
        let inner = host_markdown::MarkdownDoc::parse_str(text, None).map_err(map_err)?;
        Ok(Self { inner })
    }
    #[getter] fn body(&self) -> String { self.inner.body.clone() }
    #[getter] fn source_path(&self) -> Option<PathBuf> { self.inner.source_path.clone() }
    #[getter]
    fn frontmatter<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.frontmatter {
            d.set_item(k, json_to_py(py, v)?)?;
        }
        Ok(d)
    }
    fn is_empty(&self) -> bool { self.inner.is_empty() }
}

#[pyfunction(name = "split_bullets")]
fn py_split_bullets(body: &str) -> Vec<String> { host_markdown::split_bullets(body) }

// =========================================================================
// Loader: SkillDefinition / HookDefinition / AgentDefinition / LoadedAgent
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "SkillDefinition", frozen)]
#[derive(Clone)]
pub struct PySkillDefinition { inner: loader::SkillDefinition }

#[pymethods]
impl PySkillDefinition {
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn name(&self) -> String { self.inner.name.clone() }
    #[getter] fn instruction_fragment(&self) -> String { self.inner.instruction_fragment.clone() }
    #[getter] fn priority(&self) -> u8 { self.inner.priority }
    #[getter] fn keywords(&self) -> Vec<String> { self.inner.keywords.clone() }
    #[getter] fn tool_overlay(&self) -> Vec<String> { self.inner.tool_overlay.clone() }
    #[getter] fn memory_namespace(&self) -> Vec<String> { self.inner.memory_namespace.clone() }
    #[getter] fn source_path(&self) -> Option<PathBuf> { self.inner.source_path.clone() }
}

#[pyclass(module = "atomr_agents._native.host", name = "HookDefinition", frozen)]
#[derive(Clone)]
pub struct PyHookDefinition { inner: loader::HookDefinition }

#[pymethods]
impl PyHookDefinition {
    #[getter] fn event(&self) -> String { self.inner.event.clone() }
    #[getter] fn when(&self) -> String { self.inner.when.clone() }
    #[getter] fn source_path(&self) -> Option<PathBuf> { self.inner.source_path.clone() }
    #[getter]
    fn match_<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.match_ { d.set_item(k, json_to_py(py, v)?)?; }
        Ok(d)
    }
    #[getter]
    fn call<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.call { d.set_item(k, json_to_py(py, v)?)?; }
        Ok(d)
    }
    #[getter]
    fn budget<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.budget { d.set_item(k, json_to_py(py, v)?)?; }
        Ok(d)
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "AgentDefinition", frozen)]
#[derive(Clone)]
pub struct PyAgentDefinition { inner: loader::AgentDefinition }

#[pymethods]
impl PyAgentDefinition {
    #[getter] fn paths(&self) -> PyAgentPaths { PyAgentPaths { inner: self.inner.paths.clone() } }
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id() }
    #[getter] fn model(&self) -> Option<String> { self.inner.model() }
    #[getter] fn max_iterations(&self) -> u32 { self.inner.max_iterations() }
    #[getter] fn token_budget(&self) -> u32 { self.inner.token_budget() }
    #[getter] fn time_budget_ms(&self) -> u64 { self.inner.time_budget_ms() }
    #[getter] fn money_budget_usd(&self) -> f64 { self.inner.money_budget_usd() }
    #[getter] fn skillset_id(&self) -> String { self.inner.skillset_id() }
    #[getter] fn skillset_version(&self) -> String { self.inner.skillset_version().to_string() }
    #[getter] fn soul(&self) -> PyMarkdownDoc { PyMarkdownDoc { inner: self.inner.soul.clone() } }
    #[getter] fn rules(&self) -> PyMarkdownDoc { PyMarkdownDoc { inner: self.inner.rules.clone() } }
    #[getter] fn memory(&self) -> PyMarkdownDoc { PyMarkdownDoc { inner: self.inner.memory.clone() } }
    #[getter] fn user(&self) -> PyMarkdownDoc { PyMarkdownDoc { inner: self.inner.user.clone() } }
    #[getter]
    fn skills<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let lst = PyList::empty_bound(py);
        for s in &self.inner.skills {
            lst.append(Py::new(py, PySkillDefinition { inner: s.clone() })?)?;
        }
        Ok(lst)
    }
    #[getter]
    fn hooks<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let lst = PyList::empty_bound(py);
        for h in &self.inner.hooks {
            lst.append(Py::new(py, PyHookDefinition { inner: h.clone() })?)?;
        }
        Ok(lst)
    }
    #[getter]
    fn spec_yaml<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.spec_yaml { d.set_item(k, json_to_py(py, v)?)?; }
        Ok(d)
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "LoadedAgent")]
#[derive(Clone)]
pub struct PyLoadedAgent { inner: loader::LoadedAgent }

#[pymethods]
impl PyLoadedAgent {
    #[getter] fn definition(&self) -> PyAgentDefinition { PyAgentDefinition { inner: self.inner.definition.clone() } }
    #[getter] fn agent_id(&self) -> String { self.inner.spec.id.to_string() }
    #[getter] fn model(&self) -> String { self.inner.spec.model.clone() }
    #[getter] fn max_iterations(&self) -> u32 { self.inner.spec.max_iterations }
    #[getter] fn token_budget(&self) -> u32 { self.inner.spec.token_budget }
    #[getter] fn time_budget_ms(&self) -> u64 { self.inner.spec.time_budget_ms }
    #[getter] fn money_budget_usd(&self) -> f64 { self.inner.spec.money_budget_usd }
    #[getter] fn rules(&self) -> Vec<String> { self.inner.rules.clone() }
    #[getter] fn memory_facts(&self) -> Vec<String> { self.inner.memory_facts.clone() }
    #[getter] fn user_profile(&self) -> String { self.inner.user_profile.clone() }
    #[getter] fn skill_count(&self) -> usize { self.inner.skill_set.skills.len() }
    #[getter] fn skill_ids(&self) -> Vec<String> {
        self.inner.skill_set.skills.iter().map(|s| s.id.to_string()).collect()
    }
    #[getter] fn persona_identity(&self) -> Option<String> {
        self.inner.persona.as_ref().map(|p| p.identity.clone())
    }
    #[getter] fn persona_tone(&self) -> Option<String> {
        self.inner.persona.as_ref().and_then(|p| p.style.tone.clone())
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "AgentLoader")]
pub struct PyAgentLoader { inner: loader::AgentLoader }

#[pymethods]
impl PyAgentLoader {
    #[new]
    fn new(config: &PyHostConfig) -> Self {
        Self { inner: loader::AgentLoader::new(config.inner.clone()) }
    }
    fn agent_ids(&self) -> Vec<String> { self.inner.agent_ids() }
    fn parse(&self, agent_id: &str) -> PyResult<PyAgentDefinition> {
        let d = self.inner.parse(agent_id).map_err(map_err)?;
        Ok(PyAgentDefinition { inner: d })
    }
    fn load(&self, agent_id: &str) -> PyResult<PyLoadedAgent> {
        let l = self.inner.load(agent_id).map_err(map_err)?;
        Ok(PyLoadedAgent { inner: l })
    }
}

// =========================================================================
// Runtime + AgentHandle
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "IdentitySnapshot", frozen)]
#[derive(Clone)]
pub struct PyIdentitySnapshot {
    #[pyo3(get)] pub agent_id: String,
    #[pyo3(get)] pub model: String,
    #[pyo3(get)] pub persona_identity: Option<String>,
}

#[pyclass(module = "atomr_agents._native.host", name = "StatusSnapshot", frozen)]
#[derive(Clone)]
pub struct PyStatusSnapshot {
    #[pyo3(get)] pub agent_id: String,
    #[pyo3(get)] pub model: String,
    #[pyo3(get)] pub persona_identity: Option<String>,
    #[pyo3(get)] pub rules_count: usize,
    #[pyo3(get)] pub memory_facts_count: usize,
    #[pyo3(get)] pub user_profile_len: usize,
    #[pyo3(get)] pub skills_count: usize,
}

#[pyclass(module = "atomr_agents._native.host", name = "AgentHandle")]
#[derive(Clone)]
pub struct PyAgentHandle { inner: host_runtime::AgentHandle }

#[pymethods]
impl PyAgentHandle {
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id.clone() }
    fn identify(&self) -> PyResult<PyIdentitySnapshot> {
        let rt = tokio_runtime();
        let id = rt.block_on(self.inner.identify()).map_err(map_err)?;
        Ok(PyIdentitySnapshot {
            agent_id: id.agent_id,
            model: id.model,
            persona_identity: id.persona_identity,
        })
    }
    fn status(&self) -> PyResult<PyStatusSnapshot> {
        let rt = tokio_runtime();
        let s = rt.block_on(self.inner.status()).map_err(map_err)?;
        Ok(PyStatusSnapshot {
            agent_id: s.agent_id, model: s.model, persona_identity: s.persona_identity,
            rules_count: s.rules_count, memory_facts_count: s.memory_facts_count,
            user_profile_len: s.user_profile_len, skills_count: s.skills_count,
        })
    }
    fn preview(&self, user_message: String) -> PyResult<String> {
        let rt = tokio_runtime();
        rt.block_on(self.inner.preview(user_message)).map_err(map_err)
    }
    fn stop(&self) { self.inner.stop() }
}

fn tokio_runtime() -> Arc<tokio::runtime::Runtime> {
    use once_cell::sync::OnceCell;
    static RT: OnceCell<Arc<tokio::runtime::Runtime>> = OnceCell::new();
    RT.get_or_init(|| {
        Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("atomr-host")
                .build()
                .expect("build host tokio runtime"),
        )
    })
    .clone()
}

#[pyclass(module = "atomr_agents._native.host", name = "HostRuntime")]
#[derive(Clone)]
pub struct PyHostRuntime { inner: host_runtime::HostRuntime }

#[pymethods]
impl PyHostRuntime {
    #[staticmethod]
    fn start(config: &PyHostConfig) -> PyResult<Self> {
        let rt = tokio_runtime();
        let inner = rt
            .block_on(host_runtime::HostRuntime::start(config.inner.clone()))
            .map_err(map_err)?;
        Ok(Self { inner })
    }
    fn spawn_agent(&self, agent_id: &str) -> PyResult<PyAgentHandle> {
        let rt = tokio_runtime();
        let h = rt.block_on(self.inner.spawn_agent(agent_id)).map_err(map_err)?;
        Ok(PyAgentHandle { inner: h })
    }
    fn lookup(&self, agent_id: &str) -> Option<PyAgentHandle> {
        self.inner.lookup(agent_id).map(|h| PyAgentHandle { inner: h })
    }
    fn reload(&self, agent_id: &str) -> PyResult<PyAgentHandle> {
        let rt = tokio_runtime();
        let h = rt.block_on(self.inner.reload(agent_id)).map_err(map_err)?;
        Ok(PyAgentHandle { inner: h })
    }
    fn stop_agent(&self, agent_id: &str) { self.inner.stop_agent(agent_id) }
    fn shutdown(&self) -> PyResult<()> {
        let rt = tokio_runtime();
        let inner = self.inner.clone();
        rt.block_on(async move { inner.shutdown().await });
        Ok(())
    }
}

// =========================================================================
// Chat: render_chat_preview, AgentRouter
// =========================================================================

#[pyfunction(name = "render_chat_preview")]
fn py_render_chat_preview(loaded: &PyLoadedAgent, user_message: &str) -> String {
    host_chat::render_chat_preview(&loaded.inner, user_message)
}

#[pyclass(module = "atomr_agents._native.host", name = "AgentRouter")]
#[derive(Clone)]
pub struct PyAgentRouter { inner: host_chat::AgentRouter }

#[pymethods]
impl PyAgentRouter {
    #[new]
    #[pyo3(signature = (default_agent=None))]
    fn new(default_agent: Option<String>) -> Self {
        Self { inner: host_chat::AgentRouter::new(default_agent) }
    }
    fn pin_channel(&self, channel_id: String, agent_id: String) {
        self.inner.pin_channel(channel_id, agent_id)
    }
    fn pin_peer(&self, channel_id: String, peer_id: String, agent_id: String) {
        self.inner.pin_peer(channel_id, peer_id, agent_id)
    }
    fn route(&self, channel_id: &str, peer_id: &str) -> Option<String> {
        self.inner.route(channel_id, peer_id)
    }
    #[getter] fn default_agent(&self) -> Option<String> { self.inner.default_agent().map(|s| s.to_string()) }
}

// =========================================================================
// Memory sync (M3): system-prompt builders
// =========================================================================

#[pyfunction(name = "render_persona_block")]
fn py_render_persona_block(loaded: &PyLoadedAgent) -> Option<String> { memory_sync::render_persona_block(&loaded.inner) }
#[pyfunction(name = "render_rules_block")]
fn py_render_rules_block(loaded: &PyLoadedAgent) -> Option<String> { memory_sync::render_rules_block(&loaded.inner) }
#[pyfunction(name = "render_memory_block")]
fn py_render_memory_block(loaded: &PyLoadedAgent) -> Option<String> { memory_sync::render_memory_block(&loaded.inner) }
#[pyfunction(name = "render_user_block")]
fn py_render_user_block(loaded: &PyLoadedAgent) -> Option<String> { memory_sync::render_user_block(&loaded.inner) }
#[pyfunction(name = "build_system_prompt")]
fn py_build_system_prompt(loaded: &PyLoadedAgent) -> String { memory_sync::build_system_prompt(&loaded.inner) }

// =========================================================================
// Skills (M4)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "SkillValidationReport", frozen)]
#[derive(Clone)]
pub struct PySkillValidationReport { inner: skills_registry::SkillValidationReport }
#[pymethods]
impl PySkillValidationReport {
    #[getter] fn skill_id(&self) -> String { self.inner.skill_id.clone() }
    #[getter] fn path(&self) -> PathBuf { self.inner.path.clone() }
    #[getter] fn errors(&self) -> Vec<String> { self.inner.errors.clone() }
    #[getter] fn warnings(&self) -> Vec<String> { self.inner.warnings.clone() }
    #[getter] fn ok(&self) -> bool { self.inner.is_ok() }
}

#[pyfunction(name = "select_skills_for")]
fn py_select_skills_for<'py>(
    py: Python<'py>,
    skills: Vec<PyRef<'_, PySkillDefinition>>,
    user_message: &str,
) -> PyResult<Bound<'py, PyList>> {
    let owned: Vec<loader::SkillDefinition> = skills.iter().map(|s| s.inner.clone()).collect();
    let refs = skills_registry::select_skills_for(&owned, user_message);
    let lst = PyList::empty_bound(py);
    for r in refs {
        lst.append(Py::new(py, PySkillDefinition { inner: r.clone() })?)?;
    }
    Ok(lst)
}

#[pyfunction(name = "scaffold_skill", signature = (paths, skill_id, name, priority=5, keywords=None))]
fn py_scaffold_skill(
    paths: &PyAgentPaths,
    skill_id: &str,
    name: &str,
    priority: u8,
    keywords: Option<Vec<String>>,
) -> PyResult<PathBuf> {
    let kw = keywords.unwrap_or_default();
    skills_registry::scaffold_skill(&paths.inner, skill_id, name, priority, &kw).map_err(map_err)
}

#[pyfunction(name = "validate_skills")]
fn py_validate_skills(paths: &PyAgentPaths) -> PyResult<Vec<PySkillValidationReport>> {
    let reports = skills_registry::validate_skills(&paths.inner).map_err(map_err)?;
    Ok(reports.into_iter().map(|r| PySkillValidationReport { inner: r }).collect())
}

// =========================================================================
// Hooks (M5)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "HookResult", frozen)]
#[derive(Clone)]
pub struct PyHookResult { inner: host_hooks::HookResult }
#[pymethods]
impl PyHookResult {
    #[getter] fn hook_id(&self) -> String { self.inner.hook_id.clone() }
    #[getter] fn event(&self) -> String { self.inner.event.clone() }
    #[getter] fn when(&self) -> String { match self.inner.when { host_hooks::HookWhen::Pre => "pre".into(), host_hooks::HookWhen::Post => "post".into(), host_hooks::HookWhen::Both => "both".into() } }
    #[getter] fn ok(&self) -> bool { self.inner.ok }
    #[getter] fn error(&self) -> Option<String> { self.inner.error.clone() }
    #[getter] fn duration_ms(&self) -> u64 { self.inner.duration_ms }
    #[getter]
    fn output<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match &self.inner.output {
            Some(v) => json_to_py(py, v),
            None => Ok(py.None().into_bound(py)),
        }
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "HookRegistry")]
#[derive(Clone)]
pub struct PyHookRegistry { inner: host_hooks::HookRegistry }
#[pymethods]
impl PyHookRegistry {
    #[new] fn new() -> Self { Self { inner: host_hooks::HookRegistry::new() } }
    #[pyo3(signature = (id, event, when, match_=None, timeout_ms=1_000))]
    fn register_builtin(&self, py: Python<'_>, id: String, event: String, when: &str, match_: Option<Bound<'_, PyDict>>, timeout_ms: u64) -> PyResult<()> {
        let when = parse_when(when)?;
        let mut m = std::collections::HashMap::new();
        if let Some(d) = match_ {
            for (k, v) in d.iter() {
                let ks: String = k.extract()?;
                m.insert(ks, py_to_json(py, &v)?);
            }
        }
        let f = self.inner.builtin(&id).ok_or_else(|| PyValueError::new_err(format!("no builtin `{id}`")))?;
        self.inner.register(host_hooks::HookSpec {
            id, event, when, match_: m,
            call: host_hooks::HookCall::Builtin("builtin".into(), f),
            timeout_ms,
        });
        Ok(())
    }
    fn list_ids(&self) -> Vec<String> { self.inner.list().iter().map(|h| h.id.clone()).collect() }
}

fn parse_when(s: &str) -> PyResult<host_hooks::HookWhen> {
    match s {
        "pre" => Ok(host_hooks::HookWhen::Pre),
        "post" => Ok(host_hooks::HookWhen::Post),
        "both" => Ok(host_hooks::HookWhen::Both),
        other => Err(PyValueError::new_err(format!("bad when `{other}`"))),
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "HookDispatcher")]
pub struct PyHookDispatcher { inner: host_hooks::HookDispatcher }
#[pymethods]
impl PyHookDispatcher {
    #[new] fn new(registry: &PyHookRegistry) -> Self { Self { inner: host_hooks::HookDispatcher::new(registry.inner.clone()) } }
    fn dispatch(&self, py: Python<'_>, event: &str, when: &str, payload: Bound<'_, PyAny>) -> PyResult<Vec<PyHookResult>> {
        let when = parse_when(when)?;
        let payload = py_to_json(py, &payload)?;
        let rt = tokio_runtime();
        let res = rt.block_on(self.inner.dispatch(event, when, payload));
        Ok(res.into_iter().map(|r| PyHookResult { inner: r }).collect())
    }
}

#[pyfunction(name = "redact_secrets")]
fn py_redact_secrets<'py>(py: Python<'py>, payload: Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let v = py_to_json(py, &payload)?;
    let out = host_hooks::builtin_redact_secrets(&v).map_err(map_err)?;
    json_to_py(py, &out)
}

#[pyfunction(name = "record_to_jsonl")]
fn py_record_to_jsonl<'py>(py: Python<'py>, payload: Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let v = py_to_json(py, &payload)?;
    let out = host_hooks::builtin_record_to_jsonl(&v).map_err(map_err)?;
    json_to_py(py, &out)
}

// =========================================================================
// Scheduler (M6)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "CronEntry", frozen)]
#[derive(Clone)]
pub struct PyCronEntry { inner: host_scheduler::CronEntry }
#[pymethods]
impl PyCronEntry {
    #[new]
    #[pyo3(signature = (id, expression, call, input=None, enabled=true))]
    fn new(py: Python<'_>, id: String, expression: String, call: Bound<'_, PyAny>, input: Option<Bound<'_, PyAny>>, enabled: bool) -> PyResult<Self> {
        let call = py_to_json(py, &call)?;
        let input = match input { Some(v) => py_to_json(py, &v)?, None => serde_json::json!({}) };
        Ok(Self { inner: host_scheduler::CronEntry { id, expression, call, input, enabled } })
    }
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn expression(&self) -> String { self.inner.expression.clone() }
    #[getter] fn enabled(&self) -> bool { self.inner.enabled }
    #[getter] fn call<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.call) }
    #[getter] fn input<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.input) }
}

#[pyclass(module = "atomr_agents._native.host", name = "CronFireResult", frozen)]
#[derive(Clone)]
pub struct PyCronFireResult { inner: host_scheduler::CronFireResult }
#[pymethods]
impl PyCronFireResult {
    #[getter] fn cron_id(&self) -> String { self.inner.cron_id.clone() }
    #[getter] fn fired_at_ms(&self) -> u64 { self.inner.fired_at_ms }
    #[getter] fn call<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.call) }
    #[getter] fn input<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.input) }
}

#[pyclass(module = "atomr_agents._native.host", name = "Scheduler")]
#[derive(Clone)]
pub struct PyScheduler { inner: host_scheduler::Scheduler }
#[pymethods]
impl PyScheduler {
    #[new] fn new() -> Self { Self { inner: host_scheduler::Scheduler::new() } }
    fn register(&self, entry: PyCronEntry) -> PyResult<u64> { self.inner.register(entry.inner).map_err(map_err) }
    fn remove(&self, id: &str) -> bool { self.inner.remove(id) }
    fn list(&self) -> Vec<PyCronEntry> { self.inner.list().into_iter().map(|e| PyCronEntry { inner: e }).collect() }
    fn fire_due(&self) -> Vec<PyCronFireResult> { self.inner.fire_due().into_iter().map(|r| PyCronFireResult { inner: r }).collect() }
}

#[pyfunction(name = "parse_expression")]
fn py_parse_expression(expr: &str) -> PyResult<u64> { host_scheduler::parse_expression(expr).map_err(map_value_err) }

#[pyfunction(name = "scaffold_cron")]
fn py_scaffold_cron(py: Python<'_>, crons_dir: PathBuf, cron_id: &str, expression: &str, call: Bound<'_, PyAny>) -> PyResult<PathBuf> {
    let c = py_to_json(py, &call)?;
    host_scheduler::scaffold_cron(&crons_dir, cron_id, expression, c).map_err(map_err)
}

#[pyfunction(name = "load_crons")]
fn py_load_crons(crons_dir: PathBuf) -> PyResult<Vec<PyCronEntry>> {
    Ok(host_scheduler::load_crons(&crons_dir).map_err(map_err)?.into_iter().map(|c| PyCronEntry { inner: c }).collect())
}

// =========================================================================
// Routes (M7 helper)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "AgentsRoutingRules", frozen)]
#[derive(Clone)]
pub struct PyAgentsRoutingRules { inner: routes::AgentsRoutingRules }
#[pymethods]
impl PyAgentsRoutingRules {
    #[getter] fn default_agent(&self) -> Option<String> { self.inner.default_agent.clone() }
    #[getter]
    fn channel_pins<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in &self.inner.channel_pins { d.set_item(k, v)?; }
        Ok(d)
    }
    #[getter]
    fn peer_pins<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for ((c, p), v) in &self.inner.peer_pins {
            d.set_item((c.clone(), p.clone()), v)?;
        }
        Ok(d)
    }
}

#[pyfunction(name = "parse_agents_md")]
fn py_parse_agents_md(text: &str) -> PyAgentsRoutingRules {
    PyAgentsRoutingRules { inner: routes::parse_agents_md(text) }
}

#[pyfunction(name = "load_agents_md")]
fn py_load_agents_md(path: PathBuf) -> PyResult<PyAgentsRoutingRules> {
    Ok(PyAgentsRoutingRules { inner: host_gateway::load_agents_md(&path).map_err(map_err)? })
}

#[pyfunction(name = "build_router")]
#[pyo3(signature = (rules, default_from_config=None))]
fn py_build_router(rules: &PyAgentsRoutingRules, default_from_config: Option<String>) -> PyAgentRouter {
    PyAgentRouter { inner: host_gateway::build_router_from_rules(rules.inner.clone(), default_from_config) }
}

#[pyclass(module = "atomr_agents._native.host", name = "Gateway")]
#[derive(Clone)]
pub struct PyGateway { inner: host_gateway::Gateway }
#[pymethods]
impl PyGateway {
    #[new]
    fn new(runtime: &PyHostRuntime, router: &PyAgentRouter) -> Self {
        Self { inner: host_gateway::Gateway::new(runtime.inner.clone(), router.inner.clone()) }
    }
    fn route(&self, channel_id: &str, peer_id: &str) -> Option<String> { self.inner.route(channel_id, peer_id) }
    fn handle(&self, channel_id: &str, peer_id: &str, user_message: &str) -> PyResult<String> {
        let rt = tokio_runtime();
        rt.block_on(self.inner.handle(channel_id, peer_id, user_message)).map_err(map_err)
    }
}

// =========================================================================
// MCP (M8)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "MCPToolSpec", frozen)]
#[derive(Clone)]
pub struct PyMCPToolSpec { inner: mcp::MCPToolSpec }
#[pymethods]
impl PyMCPToolSpec {
    #[new]
    #[pyo3(signature = (name, description=String::new(), schema=None))]
    fn new(py: Python<'_>, name: String, description: String, schema: Option<Bound<'_, PyAny>>) -> PyResult<Self> {
        let schema = match schema { Some(v) => py_to_json(py, &v)?, None => serde_json::Value::Null };
        Ok(Self { inner: mcp::MCPToolSpec { name, description, schema } })
    }
    #[getter] fn name(&self) -> String { self.inner.name.clone() }
    #[getter] fn description(&self) -> String { self.inner.description.clone() }
    #[getter] fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.schema) }
}

#[pyclass(module = "atomr_agents._native.host", name = "MCPServerConfig", frozen)]
#[derive(Clone)]
pub struct PyMCPServerConfig { inner: mcp::MCPServerConfig }
#[pymethods]
impl PyMCPServerConfig {
    #[new]
    #[pyo3(signature = (id, command, env=None, tools=None))]
    fn new(id: String, command: Vec<String>, env: Option<HashMap<String, String>>, tools: Option<Vec<PyRef<'_, PyMCPToolSpec>>>) -> Self {
        Self {
            inner: mcp::MCPServerConfig {
                id, command, env: env.unwrap_or_default(),
                tools: tools.map(|v| v.iter().map(|t| t.inner.clone()).collect()).unwrap_or_default(),
            }
        }
    }
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn command(&self) -> Vec<String> { self.inner.command.clone() }
    #[getter] fn env(&self) -> HashMap<String, String> { self.inner.env.clone() }
    #[getter] fn tools(&self) -> Vec<PyMCPToolSpec> {
        self.inner.tools.iter().cloned().map(|t| PyMCPToolSpec { inner: t }).collect()
    }
}

#[pyclass(module = "atomr_agents._native.host", name = "McpBridge")]
#[derive(Clone)]
pub struct PyMcpBridge { inner: mcp::McpBridge }
#[pymethods]
impl PyMcpBridge {
    #[new] fn new(config: &PyMCPServerConfig) -> Self { Self { inner: mcp::McpBridge::new(config.inner.clone()) } }
    fn set_mock(&self, py: Python<'_>, handler: PyObject) {
        let h = Arc::new(handler);
        self.inner.set_mock(Arc::new(move |name: &str, args: &serde_json::Value| {
            let res = Python::with_gil(|py| -> PyResult<serde_json::Value> {
                let args_py = json_to_py(py, args)?;
                let r = h.bind(py).call1((name, args_py))?;
                py_to_json(py, &r)
            });
            res.map_err(|e| atomr_agents_host::error::HostError::Mcp(e.to_string()))
        }));
        let _ = py;
    }
    fn call<'py>(&self, py: Python<'py>, name: &str, args: Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let args = py_to_json(py, &args)?;
        let rt = tokio_runtime();
        let out = rt.block_on(self.inner.call(name, &args)).map_err(map_err)?;
        json_to_py(py, &out)
    }
    #[getter] fn tools(&self) -> Vec<PyMCPToolSpec> { self.inner.tools().iter().cloned().map(|t| PyMCPToolSpec { inner: t }).collect() }
}

#[pyfunction(name = "load_mcp_servers")]
fn py_load_mcp_servers(mcp_dir: PathBuf) -> PyResult<Vec<PyMCPServerConfig>> {
    Ok(mcp::load_mcp_servers(&mcp_dir).map_err(map_err)?.into_iter().map(|c| PyMCPServerConfig { inner: c }).collect())
}

// =========================================================================
// Events (M9)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "EventRecord", frozen)]
#[derive(Clone)]
pub struct PyEventRecord { inner: events::EventRecord }
#[pymethods]
impl PyEventRecord {
    #[getter] fn ts_ms(&self) -> u64 { self.inner.ts_ms }
    #[getter] fn kind(&self) -> String { self.inner.kind.clone() }
    #[getter] fn agent_id(&self) -> Option<String> { self.inner.agent_id.clone() }
    #[getter] fn payload<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.payload) }
}

#[pyclass(module = "atomr_agents._native.host", name = "EventLog")]
#[derive(Clone)]
pub struct PyEventLog { inner: events::EventLog }
#[pymethods]
impl PyEventLog {
    #[new] fn new(path: PathBuf) -> Self { Self { inner: events::EventLog::new(path) } }
    #[pyo3(signature = (kind, payload, agent_id=None))]
    fn emit(&self, py: Python<'_>, kind: &str, payload: Bound<'_, PyAny>, agent_id: Option<String>) -> PyResult<()> {
        let p = py_to_json(py, &payload)?;
        self.inner.emit(kind, agent_id, p).map_err(map_err)
    }
    fn read_all(&self) -> PyResult<Vec<PyEventRecord>> {
        Ok(self.inner.read_all().map_err(map_err)?.into_iter().map(|r| PyEventRecord { inner: r }).collect())
    }
    #[getter] fn path(&self) -> PathBuf { self.inner.path().to_path_buf() }
}

// =========================================================================
// Curator (M9)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "SkillProposal", frozen)]
#[derive(Clone)]
pub struct PySkillProposal { inner: curator::SkillProposal }
#[pymethods]
impl PySkillProposal {
    #[new]
    #[pyo3(signature = (agent_id, skill_id, name, body, keywords=None, tool_overlay=None, priority=5, rationale=None, success_rate=None))]
    fn new(agent_id: String, skill_id: String, name: String, body: String, keywords: Option<Vec<String>>, tool_overlay: Option<Vec<String>>, priority: u8, rationale: Option<String>, success_rate: Option<f64>) -> Self {
        Self { inner: curator::SkillProposal {
            agent_id, skill_id, name, body,
            keywords: keywords.unwrap_or_default(),
            tool_overlay: tool_overlay.unwrap_or_default(),
            priority, rationale, success_rate,
        }}
    }
    fn to_markdown(&self) -> String { self.inner.to_markdown() }
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id.clone() }
    #[getter] fn skill_id(&self) -> String { self.inner.skill_id.clone() }
    #[getter] fn priority(&self) -> u8 { self.inner.priority }
    #[getter] fn keywords(&self) -> Vec<String> { self.inner.keywords.clone() }
    #[getter] fn rationale(&self) -> Option<String> { self.inner.rationale.clone() }
    #[getter] fn success_rate(&self) -> Option<f64> { self.inner.success_rate }
}

#[pyfunction(name = "promote_proposal")]
fn py_promote_proposal(paths: &PyAgentPaths, proposal: &PySkillProposal, history_limit: usize) -> PyResult<PathBuf> {
    curator::promote_proposal(&paths.inner, &proposal.inner, history_limit).map_err(map_err)
}
#[pyfunction(name = "write_proposal")]
fn py_write_proposal(paths: &PyAgentPaths, proposal: &PySkillProposal) -> PyResult<PathBuf> {
    curator::write_proposal(&paths.inner, &proposal.inner).map_err(map_err)
}
#[pyfunction(name = "reject_proposal")]
fn py_reject_proposal(paths: &PyAgentPaths, skill_id: &str) -> PyResult<bool> {
    curator::reject_proposal(&paths.inner, skill_id).map_err(map_err)
}
#[pyfunction(name = "revert_skill")]
fn py_revert_skill(paths: &PyAgentPaths, skill_id: &str) -> PyResult<Option<PathBuf>> {
    curator::revert_skill(&paths.inner, skill_id).map_err(map_err)
}
#[pyfunction(name = "list_proposals")]
fn py_list_proposals(paths: &PyAgentPaths) -> PyResult<Vec<String>> {
    curator::list_proposals(&paths.inner).map_err(map_err)
}
#[pyfunction(name = "list_history")]
fn py_list_history(paths: &PyAgentPaths, skill_id: &str) -> PyResult<Vec<PathBuf>> {
    curator::list_history(&paths.inner, skill_id).map_err(map_err)
}

// =========================================================================
// Branching (M10)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "Checkpoint", frozen)]
#[derive(Clone)]
pub struct PyCheckpoint { inner: branching::Checkpoint }
#[pymethods]
impl PyCheckpoint {
    #[getter] fn branch_id(&self) -> String { self.inner.branch_id.clone() }
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id.clone() }
    #[getter] fn ts_ms(&self) -> u64 { self.inner.ts_ms }
    #[getter] fn parent_branch(&self) -> Option<String> { self.inner.parent_branch.clone() }
    #[getter] fn path(&self) -> Option<PathBuf> { self.inner.path.clone() }
    #[getter] fn working_memory<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.working_memory) }
    #[getter] fn thread_head<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match &self.inner.thread_head {
            Some(v) => json_to_py(py, v),
            None => Ok(py.None().into_bound(py)),
        }
    }
}

#[pyfunction(name = "write_checkpoint")]
#[pyo3(signature = (paths, branch_id, working_memory, thread_head=None, parent_branch=None))]
fn py_write_checkpoint(py: Python<'_>, paths: &PyAgentPaths, branch_id: &str, working_memory: Bound<'_, PyAny>, thread_head: Option<Bound<'_, PyAny>>, parent_branch: Option<String>) -> PyResult<PyCheckpoint> {
    let wm = py_to_json(py, &working_memory)?;
    let th = match thread_head { Some(v) => Some(py_to_json(py, &v)?), None => None };
    let cp = branching::write_checkpoint(&paths.inner, branch_id, wm, th, parent_branch).map_err(map_err)?;
    Ok(PyCheckpoint { inner: cp })
}
#[pyfunction(name = "latest_checkpoint")]
fn py_latest_checkpoint(paths: &PyAgentPaths, branch_id: &str) -> PyResult<Option<PyCheckpoint>> {
    Ok(branching::latest_checkpoint(&paths.inner, branch_id).map_err(map_err)?.map(|c| PyCheckpoint { inner: c }))
}
#[pyfunction(name = "fork_branch")]
fn py_fork_branch(paths: &PyAgentPaths, source: &str, new_branch: &str) -> PyResult<PyCheckpoint> {
    Ok(PyCheckpoint { inner: branching::fork_branch(&paths.inner, source, new_branch).map_err(map_err)? })
}
#[pyfunction(name = "current_branch")]
fn py_current_branch(paths: &PyAgentPaths) -> PyResult<String> { branching::current_branch(&paths.inner).map_err(map_err) }
#[pyfunction(name = "switch_branch")]
fn py_switch_branch(paths: &PyAgentPaths, branch_id: &str) -> PyResult<()> { branching::switch_branch(&paths.inner, branch_id).map_err(map_err) }
#[pyfunction(name = "delete_branch", signature = (paths, branch_id, force=false))]
fn py_delete_branch(paths: &PyAgentPaths, branch_id: &str, force: bool) -> PyResult<()> { branching::delete_branch(&paths.inner, branch_id, force).map_err(map_err) }
#[pyfunction(name = "list_branches")]
fn py_list_branches(paths: &PyAgentPaths) -> PyResult<Vec<String>> { branching::list_branches(&paths.inner).map_err(map_err) }
#[pyfunction(name = "list_checkpoints")]
fn py_list_checkpoints(paths: &PyAgentPaths, branch_id: &str) -> PyResult<Vec<PathBuf>> { branching::list_checkpoints(&paths.inner, branch_id).map_err(map_err) }
#[pyfunction(name = "prune_branch")]
fn py_prune_branch(paths: &PyAgentPaths, branch_id: &str, keep: usize) -> PyResult<usize> { branching::prune_branch(&paths.inner, branch_id, keep).map_err(map_err) }

#[pyfunction(name = "diff_branches")]
fn py_diff_branches<'py>(py: Python<'py>, paths: &PyAgentPaths, a: &str, b: &str) -> PyResult<Bound<'py, PyDict>> {
    let d = branching::diff_branches(&paths.inner, a, b).map_err(map_err)?;
    let out = PyDict::new_bound(py);
    out.set_item("added_keys", d.added_keys)?;
    out.set_item("removed_keys", d.removed_keys)?;
    let changed = PyList::empty_bound(py);
    for c in d.changed_keys {
        let row = PyDict::new_bound(py);
        row.set_item("key", c.key)?;
        row.set_item("a", json_to_py(py, &c.a)?)?;
        row.set_item("b", json_to_py(py, &c.b)?)?;
        changed.append(row)?;
    }
    out.set_item("changed_keys", changed)?;
    Ok(out)
}

// =========================================================================
// Registry cache (M11)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "CachedArtifact", frozen)]
#[derive(Clone)]
pub struct PyCachedArtifact { inner: registry_cache::CachedArtifact }
#[pymethods]
impl PyCachedArtifact {
    #[getter] fn kind(&self) -> String { self.inner.kind.clone() }
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn version(&self) -> String { self.inner.version.clone() }
    #[getter] fn cached_at_ms(&self) -> u64 { self.inner.cached_at_ms }
    #[getter] fn path(&self) -> Option<PathBuf> { self.inner.path.clone() }
    #[getter] fn slug(&self) -> String { self.inner.slug() }
    #[getter] fn payload<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.payload) }
}

#[pyclass(module = "atomr_agents._native.host", name = "SlugRef", frozen)]
#[derive(Clone)]
pub struct PySlugRef { inner: registry_cache::SlugRef }
#[pymethods]
impl PySlugRef {
    #[getter] fn kind(&self) -> String { self.inner.kind.clone() }
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn version(&self) -> Option<String> { self.inner.version.clone() }
}

#[pyfunction(name = "parse_slug")]
fn py_parse_slug(slug: &str) -> PyResult<PySlugRef> {
    Ok(PySlugRef { inner: registry_cache::parse_slug(slug).map_err(map_value_err)? })
}

#[pyfunction(name = "cache_artifact")]
fn py_cache_artifact(py: Python<'_>, host: &PyHostPaths, kind: &str, id: &str, version: &str, payload: Bound<'_, PyAny>) -> PyResult<PyCachedArtifact> {
    let p = py_to_json(py, &payload)?;
    Ok(PyCachedArtifact { inner: registry_cache::cache_artifact(&host.inner, kind, id, version, p).map_err(map_err)? })
}
#[pyfunction(name = "resolve_artifact")]
fn py_resolve_artifact(host: &PyHostPaths, kind: &str, id: &str, version: &str) -> PyResult<PyCachedArtifact> {
    Ok(PyCachedArtifact { inner: registry_cache::resolve_artifact(&host.inner, kind, id, version).map_err(map_err)? })
}
#[pyfunction(name = "delete_artifact")]
fn py_delete_artifact(host: &PyHostPaths, kind: &str, id: &str, version: &str) -> PyResult<bool> {
    registry_cache::delete_artifact(&host.inner, kind, id, version).map_err(map_err)
}
#[pyfunction(name = "list_artifacts", signature = (host, kind=None))]
fn py_list_artifacts(host: &PyHostPaths, kind: Option<&str>) -> PyResult<Vec<PyCachedArtifact>> {
    Ok(registry_cache::list_artifacts(&host.inner, kind).map_err(map_err)?.into_iter().map(|a| PyCachedArtifact { inner: a }).collect())
}

// =========================================================================
// Evals (M12)
// =========================================================================

#[pyclass(module = "atomr_agents._native.host", name = "EvalCase", frozen)]
#[derive(Clone)]
pub struct PyEvalCase { inner: host_evals::EvalCase }
#[pymethods]
impl PyEvalCase {
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn input(&self) -> String { self.inner.input.clone() }
    #[getter] fn expected<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.inner.expected) }
}

#[pyclass(module = "atomr_agents._native.host", name = "EvalSuite", frozen)]
#[derive(Clone)]
pub struct PyEvalSuite { inner: host_evals::EvalSuite }
#[pymethods]
impl PyEvalSuite {
    #[getter] fn id(&self) -> String { self.inner.id.clone() }
    #[getter] fn scorer(&self) -> String { self.inner.scorer.clone() }
    #[getter] fn description(&self) -> Option<String> { self.inner.description.clone() }
    #[getter] fn cases(&self) -> Vec<PyEvalCase> { self.inner.cases.iter().cloned().map(|c| PyEvalCase { inner: c }).collect() }
}

#[pyclass(module = "atomr_agents._native.host", name = "EvalCaseResult", frozen)]
#[derive(Clone)]
pub struct PyEvalCaseResult { inner: host_evals::EvalCaseResult }
#[pymethods]
impl PyEvalCaseResult {
    #[getter] fn case_id(&self) -> String { self.inner.case_id.clone() }
    #[getter] fn passed(&self) -> bool { self.inner.passed }
    #[getter] fn score(&self) -> f64 { self.inner.score }
    #[getter] fn reason(&self) -> Option<String> { self.inner.reason.clone() }
    #[getter] fn output(&self) -> String { self.inner.output.clone() }
}

#[pyclass(module = "atomr_agents._native.host", name = "EvalRun", frozen)]
#[derive(Clone)]
pub struct PyEvalRun { inner: host_evals::EvalRun }
#[pymethods]
impl PyEvalRun {
    #[getter] fn suite_id(&self) -> String { self.inner.suite_id.clone() }
    #[getter] fn agent_id(&self) -> String { self.inner.agent_id.clone() }
    #[getter] fn passed(&self) -> usize { self.inner.passed }
    #[getter] fn total(&self) -> usize { self.inner.total }
    fn pass_rate(&self) -> f64 { self.inner.pass_rate() }
    #[getter] fn results(&self) -> Vec<PyEvalCaseResult> { self.inner.results.iter().cloned().map(|r| PyEvalCaseResult { inner: r }).collect() }
}

#[pyfunction(name = "load_suite")]
fn py_load_suite(host: &PyHostPaths, suite_id: &str) -> PyResult<PyEvalSuite> {
    Ok(PyEvalSuite { inner: host_evals::load_suite(&host.inner, suite_id).map_err(map_err)? })
}
#[pyfunction(name = "load_suite_at")]
fn py_load_suite_at(path: PathBuf) -> PyResult<PyEvalSuite> {
    Ok(PyEvalSuite { inner: host_evals::load_suite_at(&path).map_err(map_err)? })
}
#[pyfunction(name = "list_suites")]
fn py_list_suites(host: &PyHostPaths) -> PyResult<Vec<String>> {
    host_evals::list_suites(&host.inner).map_err(map_err)
}
#[pyfunction(name = "scaffold_suite")]
fn py_scaffold_suite(host: &PyHostPaths, suite_id: &str) -> PyResult<PathBuf> {
    host_evals::scaffold_suite(&host.inner, suite_id).map_err(map_err)
}

#[pyfunction(name = "run_suite_sync", signature = (suite, agent_id, responder=None, runtime=None))]
fn py_run_suite_sync(
    py: Python<'_>,
    suite: &PyEvalSuite,
    agent_id: &str,
    responder: Option<PyObject>,
    runtime: Option<&PyHostRuntime>,
) -> PyResult<PyEvalRun> {
    // Prefer Python responder if provided.
    let responder_fn: Box<dyn Fn(&str) -> String + Send + Sync> = if let Some(r) = responder {
        let r = Arc::new(r);
        Box::new(move |input: &str| {
            Python::with_gil(|py| {
                match r.bind(py).call1((input,)) {
                    Ok(v) => v.extract::<String>().unwrap_or_else(|_| format!("{v:?}")),
                    Err(e) => {
                        let _ = e.display(py);
                        format!("ERROR: {e}")
                    }
                }
            })
        })
    } else if let Some(rt) = runtime {
        let rt = rt.inner.clone();
        let agent_id = agent_id.to_string();
        Box::new(move |input: &str| {
            let rt2 = tokio_runtime();
            let agent_id = agent_id.clone();
            let rt = rt.clone();
            let input = input.to_string();
            rt2.block_on(async move {
                let h = rt.spawn_agent(&agent_id).await.expect("spawn agent");
                h.preview(input).await.unwrap_or_else(|e| format!("ERROR: {e}"))
            })
        })
    } else {
        let _ = py;
        return Err(PyValueError::new_err(
            "run_suite_sync requires either `responder` callable or `runtime` HostRuntime",
        ));
    };
    let r = host_evals::run_suite_sync(&suite.inner, agent_id, |s| responder_fn(s)).map_err(map_err)?;
    Ok(PyEvalRun { inner: r })
}

// =========================================================================
// Module registration
// =========================================================================

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "host")?;

    // Layout / config
    m.add_class::<PyHostPaths>()?;
    m.add_class::<PyAgentPaths>()?;
    m.add_class::<PyHostConfig>()?;
    m.add_class::<PyProviderConfig>()?;
    m.add_function(wrap_pyfunction!(py_default_root, &m)?)?;

    // Markdown
    m.add_class::<PyMarkdownDoc>()?;
    m.add_function(wrap_pyfunction!(py_split_bullets, &m)?)?;

    // Loader (M1)
    m.add_class::<PySkillDefinition>()?;
    m.add_class::<PyHookDefinition>()?;
    m.add_class::<PyAgentDefinition>()?;
    m.add_class::<PyLoadedAgent>()?;
    m.add_class::<PyAgentLoader>()?;

    // Runtime / actor (M1)
    m.add_class::<PyHostRuntime>()?;
    m.add_class::<PyAgentHandle>()?;
    m.add_class::<PyIdentitySnapshot>()?;
    m.add_class::<PyStatusSnapshot>()?;

    // Chat (M2)
    m.add_function(wrap_pyfunction!(py_render_chat_preview, &m)?)?;
    m.add_class::<PyAgentRouter>()?;

    // Memory sync (M3)
    m.add_function(wrap_pyfunction!(py_render_persona_block, &m)?)?;
    m.add_function(wrap_pyfunction!(py_render_rules_block, &m)?)?;
    m.add_function(wrap_pyfunction!(py_render_memory_block, &m)?)?;
    m.add_function(wrap_pyfunction!(py_render_user_block, &m)?)?;
    m.add_function(wrap_pyfunction!(py_build_system_prompt, &m)?)?;

    // Skills (M4)
    m.add_class::<PySkillValidationReport>()?;
    m.add_function(wrap_pyfunction!(py_select_skills_for, &m)?)?;
    m.add_function(wrap_pyfunction!(py_scaffold_skill, &m)?)?;
    m.add_function(wrap_pyfunction!(py_validate_skills, &m)?)?;

    // Hooks (M5)
    m.add_class::<PyHookResult>()?;
    m.add_class::<PyHookRegistry>()?;
    m.add_class::<PyHookDispatcher>()?;
    m.add_function(wrap_pyfunction!(py_redact_secrets, &m)?)?;
    m.add_function(wrap_pyfunction!(py_record_to_jsonl, &m)?)?;

    // Scheduler (M6)
    m.add_class::<PyCronEntry>()?;
    m.add_class::<PyCronFireResult>()?;
    m.add_class::<PyScheduler>()?;
    m.add_function(wrap_pyfunction!(py_parse_expression, &m)?)?;
    m.add_function(wrap_pyfunction!(py_scaffold_cron, &m)?)?;
    m.add_function(wrap_pyfunction!(py_load_crons, &m)?)?;

    // Gateway / routes (M7)
    m.add_class::<PyAgentsRoutingRules>()?;
    m.add_function(wrap_pyfunction!(py_parse_agents_md, &m)?)?;
    m.add_function(wrap_pyfunction!(py_load_agents_md, &m)?)?;
    m.add_function(wrap_pyfunction!(py_build_router, &m)?)?;
    m.add_class::<PyGateway>()?;

    // MCP (M8)
    m.add_class::<PyMCPToolSpec>()?;
    m.add_class::<PyMCPServerConfig>()?;
    m.add_class::<PyMcpBridge>()?;
    m.add_function(wrap_pyfunction!(py_load_mcp_servers, &m)?)?;

    // Curator / events (M9)
    m.add_class::<PyEventRecord>()?;
    m.add_class::<PyEventLog>()?;
    m.add_class::<PySkillProposal>()?;
    m.add_function(wrap_pyfunction!(py_promote_proposal, &m)?)?;
    m.add_function(wrap_pyfunction!(py_write_proposal, &m)?)?;
    m.add_function(wrap_pyfunction!(py_reject_proposal, &m)?)?;
    m.add_function(wrap_pyfunction!(py_revert_skill, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_proposals, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_history, &m)?)?;

    // Branching (M10)
    m.add_class::<PyCheckpoint>()?;
    m.add_function(wrap_pyfunction!(py_write_checkpoint, &m)?)?;
    m.add_function(wrap_pyfunction!(py_latest_checkpoint, &m)?)?;
    m.add_function(wrap_pyfunction!(py_fork_branch, &m)?)?;
    m.add_function(wrap_pyfunction!(py_current_branch, &m)?)?;
    m.add_function(wrap_pyfunction!(py_switch_branch, &m)?)?;
    m.add_function(wrap_pyfunction!(py_delete_branch, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_branches, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_checkpoints, &m)?)?;
    m.add_function(wrap_pyfunction!(py_prune_branch, &m)?)?;
    m.add_function(wrap_pyfunction!(py_diff_branches, &m)?)?;

    // Registry cache (M11)
    m.add_class::<PyCachedArtifact>()?;
    m.add_class::<PySlugRef>()?;
    m.add_function(wrap_pyfunction!(py_parse_slug, &m)?)?;
    m.add_function(wrap_pyfunction!(py_cache_artifact, &m)?)?;
    m.add_function(wrap_pyfunction!(py_resolve_artifact, &m)?)?;
    m.add_function(wrap_pyfunction!(py_delete_artifact, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_artifacts, &m)?)?;

    // Evals (M12)
    m.add_class::<PyEvalCase>()?;
    m.add_class::<PyEvalSuite>()?;
    m.add_class::<PyEvalCaseResult>()?;
    m.add_class::<PyEvalRun>()?;
    m.add_function(wrap_pyfunction!(py_load_suite, &m)?)?;
    m.add_function(wrap_pyfunction!(py_load_suite_at, &m)?)?;
    m.add_function(wrap_pyfunction!(py_list_suites, &m)?)?;
    m.add_function(wrap_pyfunction!(py_scaffold_suite, &m)?)?;
    m.add_function(wrap_pyfunction!(py_run_suite_sync, &m)?)?;

    parent.add_submodule(&m)?;
    Ok(())
}
