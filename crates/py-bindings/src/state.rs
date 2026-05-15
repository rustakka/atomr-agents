//! StateGraph data shape — `CheckpointKey`, `CheckpointMeta`,
//! `Snapshot`, and reducer markers.

use std::sync::Arc;

use atomr_agents_core::{RunId, WorkflowId};
use atomr_agents_state::{
    AppendList, AppendMessages, CheckpointKey, CheckpointMeta, Checkpointer, InMemoryCheckpointer,
    LastWriteWins, MaxByTimestamp, MergeMap, Snapshot,
};
#[cfg(any(not(feature = "state-sqlite"), not(feature = "state-postgres")))]
use pyo3::exceptions::PyNotImplementedError;
use pyo3::prelude::*;

use crate::conv::json_to_py;

#[pyclass(name = "CheckpointKey", module = "atomr_agents._native.state")]
#[derive(Clone)]
pub struct PyCheckpointKey {
    pub(crate) inner: CheckpointKey,
}

#[pymethods]
impl PyCheckpointKey {
    #[new]
    fn new(workflow_id: String, run_id: String, super_step: u64) -> Self {
        Self {
            inner: CheckpointKey {
                workflow_id: WorkflowId::from(workflow_id),
                run_id: RunId::from(run_id),
                super_step,
            },
        }
    }

    #[getter]
    fn workflow_id(&self) -> &str {
        self.inner.workflow_id.as_str()
    }

    #[getter]
    fn run_id(&self) -> &str {
        self.inner.run_id.as_str()
    }

    #[getter]
    fn super_step(&self) -> u64 {
        self.inner.super_step
    }

    fn __repr__(&self) -> String {
        format!(
            "CheckpointKey(workflow={:?}, run={:?}, super_step={})",
            self.inner.workflow_id.as_str(),
            self.inner.run_id.as_str(),
            self.inner.super_step
        )
    }
}

#[pyclass(name = "CheckpointMeta", module = "atomr_agents._native.state")]
#[derive(Clone)]
pub struct PyCheckpointMeta {
    pub(crate) inner: CheckpointMeta,
}

#[pymethods]
impl PyCheckpointMeta {
    #[getter]
    fn workflow_id(&self) -> &str {
        self.inner.workflow_id.as_str()
    }

    #[getter]
    fn run_id(&self) -> &str {
        self.inner.run_id.as_str()
    }

    #[getter]
    fn super_step(&self) -> u64 {
        self.inner.super_step
    }

    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "CheckpointMeta(workflow={:?}, run={:?}, super_step={}, ts={})",
            self.inner.workflow_id.as_str(),
            self.inner.run_id.as_str(),
            self.inner.super_step,
            self.inner.timestamp_ms,
        )
    }
}

#[pyclass(name = "Snapshot", module = "atomr_agents._native.state")]
pub struct PySnapshot {
    pub(crate) inner: Snapshot,
}

#[pymethods]
impl PySnapshot {
    #[getter]
    fn key(&self) -> PyCheckpointKey {
        PyCheckpointKey {
            inner: self.inner.key.clone(),
        }
    }

    #[getter]
    fn label(&self) -> &str {
        &self.inner.label
    }

    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner.values).map_err(crate::errors::map)?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        format!(
            "Snapshot(workflow={:?}, run={:?}, super_step={})",
            self.inner.key.workflow_id.as_str(),
            self.inner.key.run_id.as_str(),
            self.inner.key.super_step,
        )
    }
}

// Reducer marker classes — the actual reduction happens in Rust.
// Reducer types are unit structs (e.g., `pub struct LastWriteWins;`)
// so we construct them as bare path expressions rather than via
// `Default::default()`.
macro_rules! reducer_marker {
    ($py:ident, $rs:ident, $name:literal) => {
        #[pyclass(name = $name, module = "atomr_agents._native.state")]
        pub struct $py {
            pub(crate) _inner: Arc<$rs>,
        }
        #[pymethods]
        impl $py {
            #[new]
            fn new() -> Self {
                Self {
                    _inner: Arc::new($rs),
                }
            }
            fn __repr__(&self) -> String {
                format!("{}()", $name)
            }
        }
    };
}

reducer_marker!(PyLastWriteWins, LastWriteWins, "LastWriteWins");
reducer_marker!(PyAppendList, AppendList, "AppendList");
reducer_marker!(PyAppendMessages, AppendMessages, "AppendMessages");
reducer_marker!(PyMergeMap, MergeMap, "MergeMap");
reducer_marker!(PyMaxByTimestamp, MaxByTimestamp, "MaxByTimestamp");

// ----- PyCheckpointer dyn handle ------------------------------------------
//
// Shared Python-facing class that wraps any `Arc<dyn Checkpointer>`.
// The concrete `InMemoryCheckpointer`, feature-gated `SqliteCheckpointer`,
// and feature-gated `PostgresCheckpointer` all return instances of this
// class via the module-level factory functions below.

#[pyclass(name = "Checkpointer", module = "atomr_agents._native.state")]
#[derive(Clone)]
pub struct PyCheckpointer {
    pub(crate) inner: Arc<dyn Checkpointer>,
}

#[pymethods]
impl PyCheckpointer {
    fn __repr__(&self) -> String {
        "Checkpointer(handle)".into()
    }
}

#[pyclass(name = "InMemoryCheckpointer", module = "atomr_agents._native.state")]
pub struct PyInMemoryCheckpointer {
    pub(crate) _inner: Arc<InMemoryCheckpointer>,
}

#[pymethods]
impl PyInMemoryCheckpointer {
    #[new]
    fn new() -> Self {
        Self {
            _inner: Arc::new(InMemoryCheckpointer::new()),
        }
    }

    fn len(&self) -> usize {
        self._inner.len()
    }

    fn is_empty(&self) -> bool {
        self._inner.is_empty()
    }

    fn __repr__(&self) -> String {
        format!("InMemoryCheckpointer(len={})", self._inner.len())
    }
}

// ----- Factory functions --------------------------------------------------

/// In-memory checkpointer, returned as the shared `Checkpointer` dyn
/// handle so it interoperates with the sqlite/postgres variants below.
#[pyfunction]
fn in_memory_checkpointer() -> PyCheckpointer {
    PyCheckpointer {
        inner: Arc::new(InMemoryCheckpointer::new()),
    }
}

/// SQLite-backed checkpointer.
///
/// Requires building atomr-agents-py-bindings with `--features state-sqlite`,
/// which forwards to `atomr-agents-state/sqlite`. Without the feature, this
/// raises `NotImplementedError`.
#[pyfunction]
fn sqlite_checkpointer(path: String) -> PyResult<PyCheckpointer> {
    #[cfg(feature = "state-sqlite")]
    {
        use atomr_agents_state::SqliteCheckpointer;
        let c = crate::runtime::shared()
            .block_on(async move { SqliteCheckpointer::connect(path).await })
            .map_err(crate::errors::map)?;
        Ok(PyCheckpointer { inner: Arc::new(c) })
    }
    #[cfg(not(feature = "state-sqlite"))]
    {
        let _ = path;
        Err(PyNotImplementedError::new_err(
            "sqlite_checkpointer requires building atomr-agents-py-bindings with \
             --features state-sqlite",
        ))
    }
}

/// Postgres-backed checkpointer.
///
/// Requires building atomr-agents-py-bindings with `--features state-postgres`,
/// which forwards to `atomr-agents-state/postgres`. Without the feature, this
/// raises `NotImplementedError`.
#[pyfunction]
fn postgres_checkpointer(dsn: String) -> PyResult<PyCheckpointer> {
    #[cfg(feature = "state-postgres")]
    {
        use atomr_agents_state::PostgresCheckpointer;
        let c = crate::runtime::shared()
            .block_on(async move { PostgresCheckpointer::connect(dsn).await })
            .map_err(crate::errors::map)?;
        Ok(PyCheckpointer { inner: Arc::new(c) })
    }
    #[cfg(not(feature = "state-postgres"))]
    {
        let _ = dsn;
        Err(PyNotImplementedError::new_err(
            "postgres_checkpointer requires building atomr-agents-py-bindings with \
             --features state-postgres",
        ))
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "state")?;
    m.add_class::<PyCheckpointKey>()?;
    m.add_class::<PyCheckpointMeta>()?;
    m.add_class::<PySnapshot>()?;
    m.add_class::<PyLastWriteWins>()?;
    m.add_class::<PyAppendList>()?;
    m.add_class::<PyAppendMessages>()?;
    m.add_class::<PyMergeMap>()?;
    m.add_class::<PyMaxByTimestamp>()?;
    m.add_class::<PyCheckpointer>()?;
    m.add_class::<PyInMemoryCheckpointer>()?;
    m.add_function(wrap_pyfunction!(in_memory_checkpointer, &m)?)?;
    m.add_function(wrap_pyfunction!(sqlite_checkpointer, &m)?)?;
    m.add_function(wrap_pyfunction!(postgres_checkpointer, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
