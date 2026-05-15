//! Output parsers — async `parse(raw: str) -> Any`.
//!
//! Wraps the concrete parser impls from `atomr_agents_parser` whose
//! associated type is `Value` (so we can JSON-round-trip the output
//! back to Python). Generic parsers (`SchemaParser<T>`,
//! `OutputFixingParser<P, T>`) are specialized to `T = Value` and
//! exposed through a unified `PyParser` dyn handle so they compose
//! with `output_fixing_parser` / `retry_with_error_parser`.
//!
//! The concrete `PyJsonParser`, `PyJsonSchemaParser`, `PyXmlParser`,
//! `PyYamlParser` types are preserved for back-compat; each grew a
//! `to_parser()` method that lifts them into the unified `PyParser`
//! handle. `CommaListParser` and `EnumParser` produce non-`Value`
//! outputs natively, so we wrap them in small adapters that emit
//! `Value::Array(...)` / `Value::String(...)` when surfaced through
//! `PyParser`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result as AgentResult, Value};
use atomr_agents_parser::{
    CommaListParser, EnumParser, JsonParser, JsonSchemaParser, OutputFixingParser, Parser, RepairModel,
    RetryWithErrorParser, SchemaParser, StreamingPartialJsonParser, XmlParser, YamlParser,
};
use pyo3::prelude::*;

use crate::conv::{json_to_py, py_to_json};
use crate::strategy::await_if_coro;

// --------------------------------------------------------------------
// Unified PyParser dyn handle (Parser<Value>)
// --------------------------------------------------------------------

#[pyclass(name = "Parser", module = "atomr_agents._native.parser")]
#[derive(Clone)]
pub struct PyParser {
    pub(crate) inner: Arc<dyn Parser<Value>>,
}

#[pymethods]
impl PyParser {
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

    fn __repr__(&self) -> String {
        "Parser(handle)".into()
    }
}

// Small adapter that wraps a `Parser<Vec<String>>` and exposes it as
// `Parser<Value>` by lifting the strings into a JSON array.
struct StringListParserAdapter<P>(Arc<P>)
where
    P: Parser<Vec<String>>;

#[async_trait]
impl<P> Parser<Value> for StringListParserAdapter<P>
where
    P: Parser<Vec<String>>,
{
    async fn parse(&self, raw: &str) -> AgentResult<Value> {
        let v = self.0.parse(raw).await?;
        Ok(Value::Array(v.into_iter().map(Value::String).collect()))
    }
    fn format_instructions(&self) -> String {
        self.0.format_instructions()
    }
}

// Adapter that wraps a `Parser<String>` and exposes it as
// `Parser<Value>` by lifting the string into `Value::String`.
struct StringParserAdapter<P>(Arc<P>)
where
    P: Parser<String>;

#[async_trait]
impl<P> Parser<Value> for StringParserAdapter<P>
where
    P: Parser<String>,
{
    async fn parse(&self, raw: &str) -> AgentResult<Value> {
        let v = self.0.parse(raw).await?;
        Ok(Value::String(v))
    }
    fn format_instructions(&self) -> String {
        self.0.format_instructions()
    }
}

// --------------------------------------------------------------------
// Concrete parser classes (back-compat with the existing API surface)
// --------------------------------------------------------------------

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

    /// Lift into the unified `Parser` handle so it can be passed to
    /// `output_fixing_parser` / `retry_with_error_parser`.
    fn to_parser(&self) -> PyParser {
        PyParser {
            inner: self.inner.clone(),
        }
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

    fn to_parser(&self) -> PyParser {
        PyParser {
            inner: self.inner.clone(),
        }
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

    fn to_parser(&self) -> PyParser {
        PyParser {
            inner: Arc::new(StringListParserAdapter(self.inner.clone())),
        }
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

    fn to_parser(&self) -> PyParser {
        PyParser {
            inner: self.inner.clone(),
        }
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

    fn to_parser(&self) -> PyParser {
        PyParser {
            inner: self.inner.clone(),
        }
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

// --------------------------------------------------------------------
// RepairModel — dyn handle + Python guest adapter
// --------------------------------------------------------------------

#[pyclass(name = "RepairModel", module = "atomr_agents._native.parser")]
#[derive(Clone)]
pub struct PyRepairModel {
    pub(crate) inner: Arc<dyn RepairModel>,
}

#[pymethods]
impl PyRepairModel {
    /// Invoke the repair model. Returns the corrected raw string.
    fn repair<'py>(&self, py: Python<'py>, original: String, hint: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.repair(&original, &hint).await.map_err(crate::errors::map)
        })
    }

    fn __repr__(&self) -> String {
        "RepairModel(handle)".into()
    }
}

pub(crate) struct PyRepairModelAdapter {
    pub(crate) target: Arc<PyObject>,
}

#[async_trait]
impl RepairModel for PyRepairModelAdapter {
    async fn repair(&self, original: &str, hint: &str) -> AgentResult<String> {
        let target = self.target.clone();
        let original = original.to_string();
        let hint = hint.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("repair")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("repair")?.call1((original, hint))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py repair: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| final_val.bind(py).extract::<String>())
            .map_err(|e| AgentError::Internal(format!("py repair result: {e}")))
    }
}

// --------------------------------------------------------------------
// Factories
// --------------------------------------------------------------------

/// Wrap `inner` in `OutputFixingParser` with the given repair model.
/// On parse failure the repair model is called with the failed output
/// + the inner parser's format instructions; the result is re-parsed
/// up to `max_attempts` times (default 3).
#[pyfunction]
#[pyo3(signature = (inner, repair, max_attempts=3))]
fn output_fixing_parser(inner: PyParser, repair: PyRepairModel, max_attempts: u32) -> PyParser {
    // OutputFixingParser is generic over the inner parser type; we
    // specialise the wrapper to `dyn Parser<Value>` so the resulting
    // composite still slots into `PyParser`.
    struct DynInner(Arc<dyn Parser<Value>>);
    #[async_trait]
    impl Parser<Value> for DynInner {
        async fn parse(&self, raw: &str) -> AgentResult<Value> {
            self.0.parse(raw).await
        }
        fn format_instructions(&self) -> String {
            self.0.format_instructions()
        }
    }
    let wrapped: OutputFixingParser<DynInner, Value> =
        OutputFixingParser::new(DynInner(inner.inner), repair.inner, max_attempts);
    PyParser {
        inner: Arc::new(wrapped),
    }
}

/// Wrap `inner` in `RetryWithErrorParser`. On parse failure the repair
/// model is re-prompted with the original prompt + the failure message;
/// retried up to `max_retries` times (default 3).
#[pyfunction]
#[pyo3(signature = (inner, repair, original_prompt, max_retries=3))]
fn retry_with_error_parser(
    inner: PyParser,
    repair: PyRepairModel,
    original_prompt: String,
    max_retries: u32,
) -> PyParser {
    struct DynInner(Arc<dyn Parser<Value>>);
    #[async_trait]
    impl Parser<Value> for DynInner {
        async fn parse(&self, raw: &str) -> AgentResult<Value> {
            self.0.parse(raw).await
        }
        fn format_instructions(&self) -> String {
            self.0.format_instructions()
        }
    }
    let wrapped: RetryWithErrorParser<DynInner, Value> =
        RetryWithErrorParser::new(DynInner(inner.inner), repair.inner, max_retries, original_prompt);
    PyParser {
        inner: Arc::new(wrapped),
    }
}

/// Build an `EnumParser` lifted into the unified `PyParser` handle.
/// The parser accepts exactly one of `variants` (case-insensitive) and
/// returns it as a `Value::String` (Python `str`).
#[pyfunction]
fn enum_parser(variants: Vec<String>) -> PyParser {
    let p = Arc::new(EnumParser::new(variants));
    PyParser {
        inner: Arc::new(StringParserAdapter(p)),
    }
}

/// Build a `SchemaParser<Value>` from a JSON-Schema-shaped dict.
/// The schema is serialised into the parser's format-instructions
/// hint; the parse step deserialises any valid JSON value, so the
/// returned `Parser` produces `Value`.
#[pyfunction]
fn schema_parser(schema: &Bound<'_, PyAny>) -> PyResult<PyParser> {
    let v = py_to_json(schema.py(), schema)?;
    let instructions = format!(
        "Respond with JSON matching this schema:\n```\n{}\n```",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    );
    let p: SchemaParser<Value> = SchemaParser::new(instructions);
    Ok(PyParser { inner: Arc::new(p) })
}

/// Materialise a `RepairModel` from a Python-side factory registered
/// via `guest.register_repair_model(key, target)` (or the generic
/// `guest.register(...)`).
#[pyfunction]
fn repair_model_from_factory(key: String) -> PyResult<PyRepairModel> {
    let target = crate::guest::must_lookup("repair_model", &key)
        .or_else(|_| crate::guest::must_lookup("parser:repair_model", &key))?;
    Ok(PyRepairModel {
        inner: Arc::new(PyRepairModelAdapter { target }),
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "parser")?;
    m.add_class::<PyParser>()?;
    m.add_class::<PyRepairModel>()?;
    m.add_class::<PyJsonParser>()?;
    m.add_class::<PyJsonSchemaParser>()?;
    m.add_class::<PyCommaListParser>()?;
    m.add_class::<PyXmlParser>()?;
    m.add_class::<PyYamlParser>()?;
    m.add_class::<PyStreamingPartialJsonParser>()?;
    m.add_function(wrap_pyfunction!(output_fixing_parser, &m)?)?;
    m.add_function(wrap_pyfunction!(retry_with_error_parser, &m)?)?;
    m.add_function(wrap_pyfunction!(enum_parser, &m)?)?;
    m.add_function(wrap_pyfunction!(schema_parser, &m)?)?;
    m.add_function(wrap_pyfunction!(repair_model_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
