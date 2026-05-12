//! Tool descriptors, sets, and parsers.
//!
//! Exposes the data shape of `Tool` (descriptor, name, schema,
//! provider) so Python can construct `ToolSet`s from typed parts.
//! Tool execution from Python lives in `crate::guest::tool_adapter`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::CallableHandle;
use atomr_agents_core::{
    AgentContext, AgentError, InvokeCtx, Result as AgentResult, ToolId, ToolSetId, TokenBudget,
    Value,
};
use atomr_agents_tool::{
    HandoffTool, ParsedToolCall, PermissionSpec, Provider, RichTool, StaticToolStrategy, Tool,
    ToolCallParser, ToolControl, ToolDescriptor, ToolReturn, ToolSchema, ToolSet,
};
use atomr_agents_strategy::{ToolRef, ToolStrategy};
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use semver::Version;

use crate::callable::PyCallable;
use crate::conv::{json_to_py, parse_version, py_to_json};
use crate::strategy::PyToolStrategy;

// ----- ToolSchema + ToolDescriptor ------------------------------------------

#[pyclass(name = "ToolSchema", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyToolSchema {
    pub(crate) inner: ToolSchema,
}

#[pymethods]
impl PyToolSchema {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let v = py_to_json(value.py(), value)?;
        Ok(Self {
            inner: ToolSchema(v),
        })
    }

    #[staticmethod]
    fn empty_object() -> Self {
        Self {
            inner: ToolSchema::empty_object(),
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.0)
    }

    fn __repr__(&self) -> String {
        format!("ToolSchema({})", self.inner.0)
    }
}

#[pyclass(name = "ToolDescriptor", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyToolDescriptor {
    pub(crate) inner: ToolDescriptor,
}

#[pymethods]
impl PyToolDescriptor {
    #[new]
    #[pyo3(signature = (id, name, description, schema=None))]
    fn new(
        id: String,
        name: String,
        description: String,
        schema: Option<PyToolSchema>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: ToolDescriptor {
                id: ToolId::from(id),
                name,
                description,
                schema: schema.map(|s| s.inner).unwrap_or_else(ToolSchema::empty_object),
            },
        })
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
    fn description(&self) -> &str {
        &self.inner.description
    }

    fn schema(&self) -> PyToolSchema {
        PyToolSchema {
            inner: self.inner.schema.clone(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ToolDescriptor(id={:?}, name={:?})",
            self.inner.id.as_str(),
            self.inner.name,
        )
    }
}

// ----- Provider (string-tagged) --------------------------------------------

#[pyclass(name = "Provider", module = "atomr_agents._native.tool", eq, frozen)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PyProvider {
    pub(crate) inner: Provider,
}

#[pymethods]
impl PyProvider {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        let inner = match name {
            "openai" | "open_ai" => Provider::OpenAi,
            "anthropic" => Provider::Anthropic,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown provider: {other:?}"
                )));
            }
        };
        Ok(Self { inner })
    }

    #[getter]
    fn name(&self) -> &'static str {
        match self.inner {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
        }
    }

    #[staticmethod]
    fn anthropic() -> Self {
        Self {
            inner: Provider::Anthropic,
        }
    }
    #[staticmethod]
    fn openai() -> Self {
        Self {
            inner: Provider::OpenAi,
        }
    }

    fn __repr__(&self) -> String {
        format!("Provider({:?})", self.name())
    }
}

// ----- ParsedToolCall -------------------------------------------------------

#[pyclass(name = "ParsedToolCall", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyParsedToolCall {
    pub(crate) inner: ParsedToolCall,
}

#[pymethods]
impl PyParsedToolCall {
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    #[getter]
    fn arguments_raw(&self) -> &str {
        &self.inner.arguments_raw
    }

    /// Parse the accumulated raw JSON arguments. Returns the parsed
    /// Python value, or raises `ValueError` for malformed JSON.
    fn arguments(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = self.inner.arguments().map_err(crate::errors::map)?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!(
            "ParsedToolCall(id={:?}, name={:?})",
            self.inner.id, self.inner.name
        )
    }
}

// ----- ToolCallParser (stateful streaming) ---------------------------------

#[pyclass(name = "ToolCallParser", module = "atomr_agents._native.tool")]
pub struct PyToolCallParser {
    inner: Option<ToolCallParser>,
}

#[pymethods]
impl PyToolCallParser {
    #[new]
    fn new(provider: PyProvider) -> Self {
        Self {
            inner: Some(ToolCallParser::new(provider.inner)),
        }
    }

    /// Feed a streaming tool-call delta. `delta` is a JSON-serialisable
    /// Python value.
    fn feed(&mut self, delta: &Bound<'_, PyAny>) -> PyResult<()> {
        let p = self.inner.as_mut().ok_or_else(|| {
            PyValueError::new_err("ToolCallParser: parser already finished")
        })?;
        let v = py_to_json(delta.py(), delta)?;
        p.feed(&v).map_err(crate::errors::map)?;
        Ok(())
    }

    /// Drain accumulated tool calls. After `finish()` the parser is
    /// consumed; create a new one for the next stream.
    fn finish(&mut self) -> PyResult<Vec<PyParsedToolCall>> {
        let p = self.inner.take().ok_or_else(|| {
            PyValueError::new_err("ToolCallParser: parser already finished")
        })?;
        Ok(p.finish()
            .into_iter()
            .map(|inner| PyParsedToolCall { inner })
            .collect())
    }
}

// ----- ToolSet --------------------------------------------------------------

#[pyclass(name = "ToolSet", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyToolSet {
    pub(crate) inner: Arc<ToolSet>,
}

#[pymethods]
impl PyToolSet {
    /// Construct an empty toolset.
    #[new]
    fn new(id: String, version: &str) -> PyResult<Self> {
        let v: Version = parse_version(version)?;
        Ok(Self {
            inner: Arc::new(ToolSet::new(ToolSetId::from(id), v, vec![])),
        })
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    #[getter]
    fn description(&self) -> &str {
        &self.inner.metadata.description
    }

    #[getter]
    fn author(&self) -> Option<String> {
        self.inner.metadata.author.clone()
    }

    fn __len__(&self) -> usize {
        self.inner.tools.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "ToolSet(id={:?}, version={:?}, tools={})",
            self.inner.id.as_str(),
            self.inner.version.to_string(),
            self.inner.tools.len()
        )
    }
}

// ----- HandoffTool ----------------------------------------------------------

/// Wraps the standard-library `HandoffTool`.
///
/// `target_agent` is a `PyCallable` whose `label` is used as the
/// handoff target name. The wrapped tool emits a
/// `ToolReturn::Command(ToolControl::Handoff { target, payload })`
/// when invoked.
#[pyclass(name = "HandoffTool", module = "atomr_agents._native.tool")]
pub struct PyHandoffTool {
    pub(crate) inner: Arc<HandoffTool>,
}

#[pymethods]
impl PyHandoffTool {
    #[new]
    fn new(target_agent: PyCallable) -> Self {
        let target = target_agent.inner.label().to_string();
        Self {
            inner: Arc::new(HandoffTool::new(target)),
        }
    }

    #[getter]
    fn default_target(&self) -> &str {
        &self.inner.default_target
    }

    /// Return the underlying `ToolDescriptor` for downstream wiring
    /// (e.g. building a `ToolSet`).
    fn descriptor(&self) -> PyToolDescriptor {
        PyToolDescriptor {
            inner: RichTool::descriptor(self.inner.as_ref()).clone(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "HandoffTool(target={:?})",
            self.inner.default_target.as_str()
        )
    }
}

/// Factory: build a `ToolDescriptor` advertising a handoff tool that
/// transfers control to the given target agent.
#[pyfunction]
fn handoff_tool(target_agent: PyCallable) -> PyToolDescriptor {
    let target = target_agent.inner.label().to_string();
    let tool = HandoffTool::new(target);
    PyToolDescriptor {
        inner: RichTool::descriptor(&tool).clone(),
    }
}

// ----- ToolControl ---------------------------------------------------------

/// Control-flow command a rich tool may return: `handoff`, `done`, or
/// `update`. Variants are constructed via the static methods.
#[pyclass(name = "ToolControl", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyToolControl {
    pub(crate) inner: ToolControl,
}

#[pymethods]
impl PyToolControl {
    /// `ToolControl.handoff(target, payload=None)`.
    #[staticmethod]
    #[pyo3(signature = (target, payload=None))]
    fn handoff(
        py: Python<'_>,
        target: String,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let payload = match payload {
            Some(p) if !p.is_none() => py_to_json(py, p)?,
            _ => Value::Null,
        };
        Ok(Self {
            inner: ToolControl::Handoff { target, payload },
        })
    }

    /// `ToolControl.done(value)`.
    #[staticmethod]
    fn done(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let v = py_to_json(py, value)?;
        Ok(Self {
            inner: ToolControl::Done(v),
        })
    }

    /// `ToolControl.update({key: value, ...})` — patch workflow channels.
    #[staticmethod]
    fn update(py: Python<'_>, updates: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut entries: Vec<(String, Value)> = Vec::with_capacity(updates.len());
        for (k, v) in updates.iter() {
            let key: String = k.extract()?;
            let val = py_to_json(py, &v)?;
            entries.push((key, val));
        }
        Ok(Self {
            inner: ToolControl::Update(entries),
        })
    }

    /// Discriminator: "handoff" | "done" | "update".
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            ToolControl::Handoff { .. } => "handoff",
            ToolControl::Done(_) => "done",
            ToolControl::Update(_) => "update",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyValueError::new_err(format!("ToolControl serialize: {e}")))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!("ToolControl(kind={:?})", self.kind())
    }
}

// ----- ToolReturn ----------------------------------------------------------

/// What a `RichTool` may return: model-visible `content`, content +
/// out-of-band `artifact`, or a control-flow `command`.
#[pyclass(name = "ToolReturn", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyToolReturn {
    pub(crate) inner: ToolReturn,
}

#[pymethods]
impl PyToolReturn {
    /// `ToolReturn.content(value)` — plain content.
    #[staticmethod]
    fn content(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let v = py_to_json(py, value)?;
        Ok(Self {
            inner: ToolReturn::Content(v),
        })
    }

    /// `ToolReturn.content_and_artifact(content, artifact)` — pair a
    /// model-visible value with an out-of-band artifact slot.
    #[staticmethod]
    fn content_and_artifact(
        py: Python<'_>,
        content: &Bound<'_, PyAny>,
        artifact: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        let c = py_to_json(py, content)?;
        let a = py_to_json(py, artifact)?;
        Ok(Self {
            inner: ToolReturn::ContentAndArtifact {
                content: c,
                artifact: a,
            },
        })
    }

    /// `ToolReturn.command(ctrl)` — drive the harness/graph.
    #[staticmethod]
    fn command(ctrl: PyToolControl) -> Self {
        Self {
            inner: ToolReturn::Command(ctrl.inner),
        }
    }

    /// Discriminator: "content" | "content_and_artifact" | "command".
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            ToolReturn::Content(_) => "content",
            ToolReturn::ContentAndArtifact { .. } => "content_and_artifact",
            ToolReturn::Command(_) => "command",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyValueError::new_err(format!("ToolReturn serialize: {e}")))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!("ToolReturn(kind={:?})", self.kind())
    }
}

// ----- RichTool (Python-facing handle) -------------------------------------

/// Python-facing handle for a `RichTool`. Pairs a `ToolDescriptor`
/// with an asynchronous invoker that returns a `ToolReturn`.
///
/// Constructing a fully-functional `RichTool` from Python (a target
/// callable that maps args -> `ToolReturn`) is part of the broader
/// guest-mode wiring; this scaffold exposes the descriptor end of the
/// surface. Use `invoke_rich` to evaluate against a Python target.
#[pyclass(name = "RichTool", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyRichTool {
    pub(crate) descriptor: ToolDescriptor,
}

#[pymethods]
impl PyRichTool {
    #[new]
    fn new(descriptor: PyToolDescriptor) -> Self {
        Self {
            descriptor: descriptor.inner,
        }
    }

    fn descriptor(&self) -> PyToolDescriptor {
        PyToolDescriptor {
            inner: self.descriptor.clone(),
        }
    }

    /// Placeholder for a fully wired Python-implementable RichTool.
    /// Production wiring lives alongside the guest tool adapter; until
    /// then, attempting to invoke raises.
    fn invoke_rich(&self, _args: &Bound<'_, PyAny>) -> PyResult<PyToolReturn> {
        Err(PyNotImplementedError::new_err(
            "RichTool.invoke_rich: register a guest tool factory and dispatch via guest.PyToolAdapter; \
             a Python-facing RichTool callable interface lands in a follow-up.",
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "RichTool(id={:?}, name={:?})",
            self.descriptor.id.as_str(),
            self.descriptor.name,
        )
    }
}

// ----- PermissionSpec ------------------------------------------------------

#[pyclass(name = "PermissionSpec", module = "atomr_agents._native.tool")]
#[derive(Clone)]
pub struct PyPermissionSpec {
    pub(crate) inner: PermissionSpec,
}

#[pymethods]
impl PyPermissionSpec {
    #[new]
    #[pyo3(signature = (depends_on=Vec::new(), requires_explicit_grant=false))]
    fn new(depends_on: Vec<String>, requires_explicit_grant: bool) -> Self {
        Self {
            inner: PermissionSpec {
                depends_on: depends_on.into_iter().map(ToolSetId::from).collect(),
                requires_explicit_grant,
            },
        }
    }

    #[getter]
    fn depends_on(&self) -> Vec<String> {
        self.inner
            .depends_on
            .iter()
            .map(|t| t.as_str().to_string())
            .collect()
    }

    #[getter]
    fn requires_explicit_grant(&self) -> bool {
        self.inner.requires_explicit_grant
    }

    fn __repr__(&self) -> String {
        format!(
            "PermissionSpec(depends_on={}, requires_explicit_grant={})",
            self.inner.depends_on.len(),
            self.inner.requires_explicit_grant,
        )
    }
}

// ----- Stub tool used by the strategy factories ----------------------------
//
// `StaticToolStrategy::new` requires `Vec<DynTool>`. The Python-facing
// factories receive `Vec<PyToolDescriptor>` (no invocation handle), so
// we wrap each descriptor in a stub `Tool` that errors when actually
// invoked. The descriptor surface — name, id, schema — is the load
// bearing part for routing/strategy selection; callers that need a
// working invoke should register a guest tool factory instead.

struct DescriptorOnlyTool {
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for DescriptorOnlyTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, _args: Value, _ctx: &InvokeCtx) -> AgentResult<Value> {
        Err(AgentError::Tool(format!(
            "tool {:?} was registered as a descriptor-only strategy entry; \
             attach an executable via guest.register_tool_factory to invoke it",
            self.descriptor.name
        )))
    }
}

fn descriptors_to_dyn_tools(
    tools: Vec<PyToolDescriptor>,
) -> Vec<atomr_agents_tool::DynTool> {
    tools
        .into_iter()
        .map(|d| {
            let t: atomr_agents_tool::DynTool = Arc::new(DescriptorOnlyTool {
                descriptor: d.inner,
            });
            t
        })
        .collect()
}

// ----- Strategy factory: static_tool_strategy ------------------------------

/// Build a `ToolStrategy` that always offers the same fixed set of
/// tool descriptors. Useful for agents whose tool surface is known
/// statically.
#[pyfunction]
fn static_tool_strategy(tools: Vec<PyToolDescriptor>) -> PyToolStrategy {
    let dyn_tools = descriptors_to_dyn_tools(tools);
    PyToolStrategy {
        inner: Arc::new(StaticToolStrategy::new(dyn_tools)),
    }
}

// ----- Strategy factory: keyword_tool_strategy -----------------------------
//
// The Rust `KeywordToolStrategy` matches words in the user turn
// against tool name/description. The Python API in this wave accepts
// an explicit `{tool_name: [keyword, ...]}` overlay so callers can
// drive selection without overloading descriptions. The
// implementation below filters by the supplied overlay; tools with
// no overlay entry are never selected.

struct KeywordOverlayStrategy {
    entries: Vec<(ToolDescriptor, Vec<String>)>,
}

#[async_trait]
impl ToolStrategy for KeywordOverlayStrategy {
    async fn select(
        &self,
        ctx: &AgentContext,
        _budget: &mut TokenBudget,
    ) -> AgentResult<Vec<ToolRef>> {
        let needle = ctx.turn.user.to_lowercase();
        let mut out: Vec<ToolRef> = Vec::new();
        for (desc, keywords) in &self.entries {
            if keywords
                .iter()
                .any(|k| needle.contains(&k.to_lowercase()))
            {
                let stub: atomr_agents_tool::DynTool = Arc::new(DescriptorOnlyTool {
                    descriptor: desc.clone(),
                });
                out.push(ToolRef {
                    id: desc.id.clone(),
                    name: desc.name.clone(),
                    handle: descriptor_only_callable(stub),
                });
            }
        }
        Ok(out)
    }
}

fn descriptor_only_callable(t: atomr_agents_tool::DynTool) -> CallableHandle {
    struct Adapter {
        inner: atomr_agents_tool::DynTool,
    }
    #[async_trait]
    impl atomr_agents_callable::Callable for Adapter {
        async fn call(
            &self,
            input: Value,
            ctx: atomr_agents_core::CallCtx,
        ) -> AgentResult<Value> {
            let ictx = InvokeCtx {
                call: ctx,
                tool_call_id: String::new(),
                raw_args: input.clone(),
            };
            self.inner.invoke(input, &ictx).await
        }

        fn label(&self) -> &str {
            &self.inner.descriptor().name
        }
    }
    Arc::new(Adapter { inner: t })
}

/// Build a `ToolStrategy` that selects tools whose configured trigger
/// keywords appear in the user turn.
///
/// `keywords` maps `tool_name -> [trigger_word, ...]`. Tool names
/// missing from the dict are never selected.
#[pyfunction]
fn keyword_tool_strategy(
    tools: Vec<PyToolDescriptor>,
    keywords: &Bound<'_, PyDict>,
) -> PyResult<PyToolStrategy> {
    let mut entries: Vec<(ToolDescriptor, Vec<String>)> = Vec::with_capacity(tools.len());
    for d in tools {
        let kws: Vec<String> = match keywords.get_item(&d.inner.name)? {
            Some(v) => v.extract()?,
            None => Vec::new(),
        };
        entries.push((d.inner, kws));
    }
    Ok(PyToolStrategy {
        inner: Arc::new(KeywordOverlayStrategy { entries }),
    })
}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "tool")?;
    m.add_class::<PyToolSchema>()?;
    m.add_class::<PyToolDescriptor>()?;
    m.add_class::<PyProvider>()?;
    m.add_class::<PyParsedToolCall>()?;
    m.add_class::<PyToolCallParser>()?;
    m.add_class::<PyToolSet>()?;
    m.add_class::<PyHandoffTool>()?;
    m.add_class::<PyRichTool>()?;
    m.add_class::<PyToolControl>()?;
    m.add_class::<PyToolReturn>()?;
    m.add_class::<PyPermissionSpec>()?;
    m.add_function(wrap_pyfunction!(handoff_tool, &m)?)?;
    m.add_function(wrap_pyfunction!(static_tool_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(keyword_tool_strategy, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
