//! Tool descriptors, sets, and parsers.
//!
//! Exposes the data shape of `Tool` (descriptor, name, schema,
//! provider) so Python can construct `ToolSet`s from typed parts.
//! Tool execution from Python lives in `crate::guest::tool_adapter`.

use std::sync::Arc;

use atomr_agents_core::{ToolId, ToolSetId};
use atomr_agents_tool::{
    ParsedToolCall, Provider, ToolCallParser, ToolDescriptor, ToolSchema, ToolSet,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use semver::Version;

use crate::conv::{json_to_py, parse_version, py_to_json};

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

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "tool")?;
    m.add_class::<PyToolSchema>()?;
    m.add_class::<PyToolDescriptor>()?;
    m.add_class::<PyProvider>()?;
    m.add_class::<PyParsedToolCall>()?;
    m.add_class::<PyToolCallParser>()?;
    m.add_class::<PyToolSet>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
