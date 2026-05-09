//! Output parsers — async `parse(raw: str) -> Any`.
//!
//! Wraps the concrete parser impls from `atomr_agents_parser` whose
//! associated type is `Value` (so we can JSON-round-trip the output
//! back to Python). Generic parsers (`SchemaParser<T>`,
//! `OutputFixingParser<P, T>`) require a Rust-side type and are not
//! exposed; users wanting typed output can use `JsonSchemaParser` and
//! validate downstream.

use std::sync::Arc;

use atomr_agents_parser::{
    CommaListParser, JsonParser, JsonSchemaParser, Parser, StreamingPartialJsonParser, XmlParser,
    YamlParser,
};
use pyo3::prelude::*;

use crate::conv::{json_to_py, py_to_json};

#[pyclass(name = "JsonParser", module = "atomr_agents._native.parser")]
pub struct PyJsonParser {
    inner: Arc<JsonParser>,
}

#[pymethods]
impl PyJsonParser {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(JsonParser),
        }
    }

    fn parse<'py>(&self, py: Python<'py>, raw: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = p.parse(&raw).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &v))
        })
    }

    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[pyclass(name = "JsonSchemaParser", module = "atomr_agents._native.parser")]
pub struct PyJsonSchemaParser {
    inner: Arc<JsonSchemaParser>,
}

#[pymethods]
impl PyJsonSchemaParser {
    #[new]
    fn new(schema: &Bound<'_, PyAny>) -> PyResult<Self> {
        let v = py_to_json(schema.py(), schema)?;
        Ok(Self {
            inner: Arc::new(JsonSchemaParser { schema: v }),
        })
    }

    fn parse<'py>(&self, py: Python<'py>, raw: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = p.parse(&raw).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &v))
        })
    }

    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[pyclass(name = "CommaListParser", module = "atomr_agents._native.parser")]
pub struct PyCommaListParser {
    inner: Arc<CommaListParser>,
}

#[pymethods]
impl PyCommaListParser {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(CommaListParser),
        }
    }

    fn parse<'py>(&self, py: Python<'py>, raw: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v: Vec<String> = p.parse(&raw).await.map_err(crate::errors::map)?;
            Ok(v)
        })
    }

    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[pyclass(name = "XmlParser", module = "atomr_agents._native.parser")]
pub struct PyXmlParser {
    inner: Arc<XmlParser>,
}

#[pymethods]
impl PyXmlParser {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(XmlParser),
        }
    }

    fn parse<'py>(&self, py: Python<'py>, raw: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = p.parse(&raw).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &v))
        })
    }

    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[pyclass(name = "YamlParser", module = "atomr_agents._native.parser")]
pub struct PyYamlParser {
    inner: Arc<YamlParser>,
}

#[pymethods]
impl PyYamlParser {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(YamlParser),
        }
    }

    fn parse<'py>(&self, py: Python<'py>, raw: String) -> PyResult<Bound<'py, PyAny>> {
        let p = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let v = p.parse(&raw).await.map_err(crate::errors::map)?;
            Python::with_gil(|py| json_to_py(py, &v))
        })
    }

    fn format_instructions(&self) -> String {
        self.inner.format_instructions()
    }
}

#[pyclass(name = "StreamingPartialJsonParser", module = "atomr_agents._native.parser")]
pub struct PyStreamingPartialJsonParser {
    inner: Option<StreamingPartialJsonParser>,
}

#[pymethods]
impl PyStreamingPartialJsonParser {
    #[new]
    fn new() -> Self {
        Self {
            inner: Some(StreamingPartialJsonParser::new()),
        }
    }

    /// Feed a chunk of JSON text. Returns the latest parse result (or
    /// `None` if no valid prefix yet).
    fn feed(&mut self, py: Python<'_>, chunk: &str) -> PyResult<Option<PyObject>> {
        let p = self
            .inner
            .as_mut()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("parser already finished"))?;
        match p.feed(chunk).map_err(crate::errors::map)? {
            Some(v) => Ok(Some(json_to_py(py, &v)?)),
            None => Ok(None),
        }
    }

    /// Drain the final value. After `finish()` the parser is consumed.
    fn finish(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        let p = self
            .inner
            .take()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("parser already finished"))?;
        let v = p.finish().map_err(crate::errors::map)?;
        json_to_py(py, &v)
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "parser")?;
    m.add_class::<PyJsonParser>()?;
    m.add_class::<PyJsonSchemaParser>()?;
    m.add_class::<PyCommaListParser>()?;
    m.add_class::<PyXmlParser>()?;
    m.add_class::<PyYamlParser>()?;
    m.add_class::<PyStreamingPartialJsonParser>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
