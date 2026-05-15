//! `Callable` — the single universal execution interface for the
//! Python bindings.
//!
//! Every dispatchable type in atomr-agents (agents, workflows,
//! harnesses, retrievers, tools, ingest pipelines, …) becomes a
//! [`PyCallable`] when exposed to Python. The generic Rust types
//! (`WithRetry<T>`, `Pipeline<…>`) never cross the FFI boundary —
//! callers see a uniform `call(input, ctx) -> awaitable[Any]` signature
//! and compose with free functions returning `PyCallable`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atomr_agents_callable::{
    with_config, with_fallbacks, with_retry, with_timeout, Branch, Callable, CallableHandle, FnCallable,
    Pipeline, RetryPolicy, RunConfig,
};
use atomr_agents_core::{AgentError, CallCtx, Result as AgentResult, Value};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::conv::{callctx_from_pydict, callctx_to_pydict, json_to_py, py_to_json};
use crate::errors;
use crate::guest::lookup_guest as lookup;

/// Universal handle wrapping `Arc<dyn Callable>`. Python sees one
/// class; the underlying Rust type may be a pipeline, decorator,
/// adapter, or runtime — they all dispatch through this.
#[pyclass(name = "Callable", module = "atomr_agents._native.callable")]
#[derive(Clone)]
pub struct PyCallable {
    pub(crate) inner: CallableHandle,
}

impl PyCallable {
    pub fn from_handle(inner: CallableHandle) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyCallable {
    /// `await callable.call(input, ctx)` — invoke the underlying
    /// callable. `input` is any JSON-serialisable Python value; `ctx`
    /// is an optional dict (`agent_id`, `tokens`, `time_ms`,
    /// `money_usd`, `iterations`, `trace`).
    #[pyo3(signature = (input=None, ctx=None))]
    fn call<'py>(
        &self,
        py: Python<'py>,
        input: Option<&Bound<'py, PyAny>>,
        ctx: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input_value = match input {
            Some(b) if !b.is_none() => py_to_json(py, b)?,
            _ => Value::Null,
        };
        let call_ctx = callctx_from_pydict(py, ctx)?;
        let h = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let out = h.call(input_value, call_ctx).await.map_err(errors::map)?;
            Python::with_gil(|py| json_to_py(py, &out))
        })
    }

    /// Convenience: synchronously block on `call`, mostly for
    /// scripting / REPL use.
    #[pyo3(signature = (input=None, ctx=None))]
    fn call_sync(
        &self,
        py: Python<'_>,
        input: Option<&Bound<'_, PyAny>>,
        ctx: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyObject> {
        let input_value = match input {
            Some(b) if !b.is_none() => py_to_json(py, b)?,
            _ => Value::Null,
        };
        let call_ctx = callctx_from_pydict(py, ctx)?;
        let h = self.inner.clone();
        let out = py
            .allow_threads(|| {
                crate::runtime::shared().block_on(async move { h.call(input_value, call_ctx).await })
            })
            .map_err(errors::map)?;
        json_to_py(py, &out)
    }

    /// Free-form label used by telemetry. Matches `Callable::label`.
    #[getter]
    fn label(&self) -> String {
        self.inner.label().to_string()
    }

    /// Identity passthrough: useful as a starting Pipeline node.
    #[staticmethod]
    fn identity() -> Self {
        let h: CallableHandle = Arc::new(FnCallable::labeled(
            "identity",
            |v: Value, _ctx| async move { Ok(v) },
        ));
        Self { inner: h }
    }

    /// Build a `PyCallable` from a Python callable. The Python target
    /// may be a regular function, an `async def`, an instance with an
    /// `invoke(input, ctx)` or `__call__(input, ctx)` method, or a
    /// class whose zero-arg constructor produces such an instance.
    #[staticmethod]
    #[pyo3(signature = (target, label=None))]
    fn from_callable(target: PyObject, label: Option<String>) -> Self {
        let label = label.unwrap_or_else(|| "py_callable".to_string());
        let adapter = PyCallableAdapter {
            target: Arc::new(target),
            label,
        };
        Self {
            inner: Arc::new(adapter),
        }
    }

    /// Build a `PyCallable` from a previously-registered guest factory
    /// key (see `atomr_agents.guest.callable_`).
    #[staticmethod]
    fn from_factory(key: String) -> PyResult<Self> {
        let target = crate::guest::must_lookup("callable", &key)?;
        Ok(Self {
            inner: Arc::new(PyCallableAdapter {
                target,
                label: format!("guest:{key}"),
            }),
        })
    }

    fn __repr__(&self) -> String {
        format!("Callable(label={:?})", self.inner.label())
    }
}

// ----- Rust adapter wrapping a Python target -------------------------------

pub(crate) struct PyCallableAdapter {
    pub(crate) target: Arc<PyObject>,
    pub(crate) label: String,
}

#[async_trait]
impl Callable for PyCallableAdapter {
    fn label(&self) -> &str {
        &self.label
    }

    async fn call(&self, input: Value, ctx: CallCtx) -> AgentResult<Value> {
        // First, synchronously resolve the python-side call result.
        // Then (outside the GIL) optionally await a coroutine return.
        let target = self.target.clone();
        let coro_or_val: PyObject = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let args_obj = json_to_py(py, &input)?;
            let ctx_dict = callctx_to_pydict(py, &ctx)?;
            // Class vs instance vs function:
            //  - prefer `invoke(args, ctx)` if it exists,
            //  - otherwise try `__call__(args, ctx)`,
            //  - if it's a class, instantiate then call invoke.
            let instance: Bound<'_, PyAny> = if bound.hasattr("invoke")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.clone()
            } else if bound.hasattr("__class__")? {
                bound.call0()?
            } else {
                bound.clone()
            };
            let result = if instance.hasattr("invoke")? {
                instance.getattr("invoke")?.call1((args_obj, ctx_dict))?
            } else {
                instance.call1((args_obj, ctx_dict))?
            };
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py callable: {e}")))?;

        let maybe_future = Python::with_gil(|py| -> PyResult<Option<_>> {
            let bound = coro_or_val.bind(py);
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
        .map_err(|e| AgentError::Internal(format!("py callable inspect: {e}")))?;

        let final_val = match maybe_future {
            Some(fut) => fut
                .await
                .map_err(|e| AgentError::Internal(format!("py callable await: {e}")))?,
            None => coro_or_val,
        };

        Python::with_gil(|py| py_to_json(py, final_val.bind(py)))
            .map_err(|e| AgentError::Internal(format!("py callable result: {e}")))
    }
}

// ----- Decorator factories -------------------------------------------------

#[pyfunction]
#[pyo3(signature = (inner, max_attempts=3, initial_backoff_ms=50, backoff_multiplier=2.0, max_backoff_ms=5_000))]
fn with_retry_(
    inner: PyCallable,
    max_attempts: u32,
    initial_backoff_ms: u64,
    backoff_multiplier: f32,
    max_backoff_ms: u64,
) -> PyCallable {
    let policy = RetryPolicy {
        max_attempts,
        initial_backoff: Duration::from_millis(initial_backoff_ms),
        backoff_multiplier,
        max_backoff: Duration::from_millis(max_backoff_ms),
    };
    PyCallable::from_handle(with_retry(inner.inner, policy))
}

#[pyfunction]
fn with_timeout_(inner: PyCallable, milliseconds: u64) -> PyCallable {
    PyCallable::from_handle(with_timeout(inner.inner, Duration::from_millis(milliseconds)))
}

#[pyfunction]
fn with_fallbacks_(primary: PyCallable, alternates: Vec<PyCallable>) -> PyCallable {
    PyCallable::from_handle(with_fallbacks(
        primary.inner,
        alternates.into_iter().map(|c| c.inner).collect(),
    ))
}

#[pyfunction]
#[pyo3(signature = (inner, run_name=None, tags=None, metadata=None))]
fn with_config_(
    py: Python<'_>,
    inner: PyCallable,
    run_name: Option<String>,
    tags: Option<Vec<String>>,
    metadata: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyCallable> {
    let metadata = match metadata {
        Some(m) if !m.is_none() => match py_to_json(py, m)? {
            Value::Object(map) => map,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "with_config(metadata=...) must be a dict or None",
                ))
            }
        },
        _ => serde_json::Map::new(),
    };
    let cfg = RunConfig {
        run_name,
        tags: tags.unwrap_or_default(),
        metadata,
    };
    Ok(PyCallable::from_handle(with_config(inner.inner, cfg)))
}

#[pyfunction]
fn fan_out_(branches: &Bound<'_, PyDict>) -> PyResult<PyCallable> {
    let mut entries: Vec<(String, CallableHandle)> = Vec::new();
    for (k, v) in branches.iter() {
        let name: String = k.extract()?;
        let c: PyCallable = v.extract()?;
        entries.push((name, c.inner));
    }
    Ok(PyCallable::from_handle(atomr_agents_callable::fan_out(entries)))
}

/// Branch on a Python predicate. The predicate is called with the
/// JSON-decoded input and must return a truthy/falsy value.
#[pyfunction]
fn branch_(predicate: PyObject, if_true: PyCallable, if_false: PyCallable) -> PyCallable {
    let predicate = Arc::new(predicate);
    let pred = move |v: &Value| -> bool {
        let predicate = predicate.clone();
        let v = v.clone();
        Python::with_gil(|py| {
            let bound = predicate.bind(py);
            let arg = match json_to_py(py, &v) {
                Ok(o) => o,
                Err(_) => return false,
            };
            let arg_b = arg.bind(py);
            match bound.call1((arg_b,)) {
                Ok(r) => r.is_truthy().unwrap_or(false),
                Err(_) => false,
            }
        })
    };
    PyCallable::from_handle(Arc::new(Branch::new(pred, if_true.inner, if_false.inner)))
}

// ----- Pipeline builder ----------------------------------------------------

#[pyclass(name = "Pipeline", module = "atomr_agents._native.callable")]
pub struct PyPipeline {
    builder: Option<Pipeline>,
}

#[pymethods]
impl PyPipeline {
    /// `Pipeline.from(c)` — seed a pipeline with the first stage.
    #[staticmethod]
    fn from_(c: PyCallable) -> Self {
        Self {
            builder: Some(Pipeline::from(c.inner)),
        }
    }

    /// `pipeline.then(c)` — chain another stage.
    fn then(&mut self, c: PyCallable) -> PyResult<()> {
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pipeline already built"))?;
        self.builder = Some(b.then(c.inner));
        Ok(())
    }

    /// `pipeline.assign(key, c)` — augment the current value-as-object
    /// with the result of `c` under `key`.
    fn assign(&mut self, key: String, c: PyCallable) -> PyResult<()> {
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pipeline already built"))?;
        self.builder = Some(b.assign(key, c.inner));
        Ok(())
    }

    /// `pipeline.fan_out({name: c, ...})` — inline parallel stage.
    fn fan_out_with(&mut self, branches: &Bound<'_, PyDict>) -> PyResult<()> {
        let mut entries: Vec<(String, CallableHandle)> = Vec::new();
        for (k, v) in branches.iter() {
            let name: String = k.extract()?;
            let c: PyCallable = v.extract()?;
            entries.push((name, c.inner));
        }
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pipeline already built"))?;
        self.builder = Some(b.fan_out_with(entries));
        Ok(())
    }

    /// `pipeline.passthrough()` — chain an identity stage.
    fn passthrough(&mut self) -> PyResult<()> {
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pipeline already built"))?;
        self.builder = Some(b.passthrough());
        Ok(())
    }

    /// `pipeline.build() -> Callable` — freeze the pipeline.
    fn build(&mut self) -> PyResult<PyCallable> {
        let b = self
            .builder
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pipeline already built"))?;
        Ok(PyCallable::from_handle(b.build()))
    }
}

/// Free-function alias: `lambda_(py_fn)` mirrors LangChain's
/// `RunnableLambda`. Equivalent to `Callable.from_callable(py_fn)`.
#[pyfunction]
#[pyo3(signature = (target, label=None))]
fn lambda_(target: PyObject, label: Option<String>) -> PyCallable {
    PyCallable::from_callable(target, label)
}

/// `passthrough()` — convenience for an identity callable.
#[pyfunction]
fn passthrough() -> PyCallable {
    PyCallable::identity()
}

// Suppress unused warnings for items only re-exported by name.
#[allow(dead_code)]
fn _list_tuple_imports(_l: &PyList, _t: &PyTuple) {}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "callable")?;
    m.add_class::<PyCallable>()?;
    m.add_class::<PyPipeline>()?;
    m.add_function(wrap_pyfunction!(with_retry_, &m)?)?;
    m.add_function(wrap_pyfunction!(with_timeout_, &m)?)?;
    m.add_function(wrap_pyfunction!(with_fallbacks_, &m)?)?;
    m.add_function(wrap_pyfunction!(with_config_, &m)?)?;
    m.add_function(wrap_pyfunction!(fan_out_, &m)?)?;
    m.add_function(wrap_pyfunction!(branch_, &m)?)?;
    m.add_function(wrap_pyfunction!(lambda_, &m)?)?;
    m.add_function(wrap_pyfunction!(passthrough, &m)?)?;
    // Aliases without the trailing underscore — match the Rust crate
    // names so Python imports read like `from atomr_agents.callable
    // import with_retry`.
    m.add("with_retry", m.getattr("with_retry_")?)?;
    m.add("with_timeout", m.getattr("with_timeout_")?)?;
    m.add("with_fallbacks", m.getattr("with_fallbacks_")?)?;
    m.add("with_config", m.getattr("with_config_")?)?;
    m.add("fan_out", m.getattr("fan_out_")?)?;
    m.add("branch", m.getattr("branch_")?)?;
    parent.add_submodule(&m)?;
    // Hand-built lookup helper for sibling modules. Forward unused
    // import to silence the compiler when no Pipeline.from() exists.
    let _ = lookup; // suppress warning in this file
    Ok(())
}
