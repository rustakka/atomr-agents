//! Python bindings for the microVM sandbox.
//!
//! Exposes `atomr_agents._native.sandbox`:
//!
//! - `SandboxProfile` — the 5 toolchain profiles (enum).
//! - `SandboxConfig` — backend / topology selection (`local`, `mock`,
//!   `docker`, `firecracker`, `cluster`).
//! - `SandboxClient` — `local_default()` builder, `run()` (ephemeral one-shot),
//!   `create()` (persistent sandbox), `events()`.
//! - `Sandbox` — a live sandbox: `exec`, `run`, `write_file`, `read_file`,
//!   `fork`, `snapshot`, `destroy`.
//! - `SandboxEventStream` — `recv()` async iterator over lifecycle events.
//!
//! Mirrors the `coding_cli` submodule. Backed by `SandboxHarness::local_default()`
//! (mock backend) it runs cross-platform; the Docker / Firecracker / remote
//! backends slot in behind the same surface.

use std::sync::Arc;

use atomr_agents_sandbox_core::{
    CreateSandbox, ExecRequest, ExecResult, Language, SandboxBackendSel, SandboxEventStream,
    SandboxId, SandboxProfile,
};
use atomr_agents_sandbox_harness::SandboxHarness;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::Mutex as AsyncMutex;

use crate::conv::{json_to_py, py_to_json};

// ----- helpers -----------------------------------------------------------

fn exec_from_py(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<ExecRequest> {
    let value = py_to_json(py, obj)?;
    serde_json::from_value::<ExecRequest>(value)
        .map_err(|e| PyValueError::new_err(format!("invalid ExecRequest: {e}")))
}

fn exec_result_to_py(py: Python<'_>, r: &ExecResult) -> PyResult<PyObject> {
    // Same flat shape as the `execute_in_sandbox` tool and the harness Callable.
    json_to_py(py, &r.to_tool_json())
}

// ----- SandboxProfile ----------------------------------------------------

#[pyclass(name = "SandboxProfile", module = "atomr_agents._native.sandbox", eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PySandboxProfile {
    PythonOnly,
    NpmOnly,
    RustOnly,
    PythonAndNpm,
    FullStack,
}

impl PySandboxProfile {
    fn to_core(self) -> SandboxProfile {
        match self {
            PySandboxProfile::PythonOnly => SandboxProfile::PythonOnly,
            PySandboxProfile::NpmOnly => SandboxProfile::NpmOnly,
            PySandboxProfile::RustOnly => SandboxProfile::RustOnly,
            PySandboxProfile::PythonAndNpm => SandboxProfile::PythonAndNpm,
            PySandboxProfile::FullStack => SandboxProfile::FullStack,
        }
    }
}

#[pymethods]
impl PySandboxProfile {
    /// snake_case wire value (`python_only`, `full_stack`, …).
    #[getter]
    fn value(&self) -> &'static str {
        match self {
            PySandboxProfile::PythonOnly => "python_only",
            PySandboxProfile::NpmOnly => "npm_only",
            PySandboxProfile::RustOnly => "rust_only",
            PySandboxProfile::PythonAndNpm => "python_and_npm",
            PySandboxProfile::FullStack => "full_stack",
        }
    }

    fn __repr__(&self) -> String {
        format!("SandboxProfile.{self:?}")
    }
}

// ----- SandboxConfig -----------------------------------------------------

/// Backend / topology selection. `profile` is supplied separately to
/// `SandboxClient.create`.
#[pyclass(name = "SandboxConfig", module = "atomr_agents._native.sandbox")]
#[derive(Clone)]
pub struct PySandboxConfig {
    sel: SandboxBackendSel,
}

#[pymethods]
impl PySandboxConfig {
    #[new]
    fn new() -> Self {
        Self { sel: SandboxBackendSel::Auto }
    }

    /// Pick the most secure locally-available backend.
    #[staticmethod]
    fn local() -> Self {
        Self { sel: SandboxBackendSel::Auto }
    }

    /// Force the deterministic in-memory mock backend.
    #[staticmethod]
    fn mock() -> Self {
        Self { sel: SandboxBackendSel::Mock }
    }

    /// "Insecure dev mode" Docker backend, optionally with a specific image.
    #[staticmethod]
    #[pyo3(signature = (image=None))]
    fn docker(image: Option<String>) -> Self {
        Self { sel: SandboxBackendSel::Docker { image } }
    }

    /// Local Firecracker (Linux + KVM) backend.
    #[staticmethod]
    fn firecracker() -> Self {
        Self { sel: SandboxBackendSel::Firecracker }
    }

    /// Tier-3 cluster: route through the gRPC control plane at `endpoint`.
    #[staticmethod]
    fn cluster(endpoint: String) -> Self {
        Self { sel: SandboxBackendSel::Remote { endpoint } }
    }

    fn __repr__(&self) -> String {
        format!("SandboxConfig({:?})", self.sel)
    }
}

// ----- SandboxEventStream ------------------------------------------------

#[pyclass(name = "SandboxEventStream", module = "atomr_agents._native.sandbox")]
pub struct PySandboxEventStream {
    inner: Arc<AsyncMutex<SandboxEventStream>>,
}

#[pymethods]
impl PySandboxEventStream {
    /// Async `recv()` → dict, or `None` once the stream closes.
    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let next = {
                let mut guard = inner.lock().await;
                guard.recv().await
            };
            Python::with_gil(|py| match next {
                None => Ok(py.None()),
                Some(ev) => {
                    let value = serde_json::to_value(&ev).unwrap_or(serde_json::Value::Null);
                    json_to_py(py, &value)
                }
            })
        })
    }
}

// ----- Sandbox -----------------------------------------------------------

/// A live sandbox in the harness registry.
#[pyclass(name = "Sandbox", module = "atomr_agents._native.sandbox")]
pub struct PySandbox {
    harness: Arc<SandboxHarness>,
    id: SandboxId,
}

#[pymethods]
impl PySandbox {
    #[getter]
    fn id(&self) -> String {
        self.id.to_string()
    }

    /// Async: run a full `ExecRequest` dict, resolve to a result dict.
    fn exec<'py>(&self, py: Python<'py>, request: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let req = exec_from_py(py, request)?;
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let res = harness
                .exec(&id, req)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| exec_result_to_py(py, &res))
        })
    }

    /// Async: run a shell command (`Bash`), resolve to a result dict.
    fn run<'py>(&self, py: Python<'py>, command: String) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let res = harness
                .exec(&id, ExecRequest::new(Language::Bash, command))
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| exec_result_to_py(py, &res))
        })
    }

    /// Async: write bytes to a file in the sandbox.
    fn write_file<'py>(
        &self,
        py: Python<'py>,
        path: String,
        data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let handle = harness
                .get(&id)
                .ok_or_else(|| PyRuntimeError::new_err("sandbox not found"))?;
            handle
                .write_file(&path, &data)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    /// Async: read a file from the sandbox as `bytes`.
    fn read_file<'py>(&self, py: Python<'py>, path: String) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let handle = harness
                .get(&id)
                .ok_or_else(|| PyRuntimeError::new_err("sandbox not found"))?;
            let bytes = handle
                .read_file(&path)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Ok(PyBytes::new_bound(py, &bytes).into_py(py)))
        })
    }

    /// Async: fork this sandbox into a new one (backs agent branch states).
    fn fork<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let info = harness
                .fork(&id)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PySandbox { harness: harness.clone(), id: info.id },
                )
            })
        })
    }

    /// Async: snapshot this sandbox, returning the snapshot id.
    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let snap = harness
                .snapshot(&id)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(snap.to_string())
        })
    }

    /// Async: destroy and de-register this sandbox.
    fn destroy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let harness = self.harness.clone();
        let id = self.id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            harness
                .destroy(&id)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        format!("Sandbox(id={})", self.id)
    }
}

// ----- SandboxClient -----------------------------------------------------

#[pyclass(name = "SandboxClient", module = "atomr_agents._native.sandbox")]
pub struct PySandboxClient {
    inner: Arc<SandboxHarness>,
}

#[pymethods]
impl PySandboxClient {
    /// Build a client over the mock backend + best-fit scheduler. Runs
    /// cross-platform with no Docker or KVM.
    #[staticmethod]
    fn local_default() -> Self {
        Self { inner: Arc::new(SandboxHarness::local_default()) }
    }

    #[getter]
    fn backend(&self) -> String {
        self.inner.backend_name().to_string()
    }

    fn live_count(&self) -> usize {
        self.inner.live_count()
    }

    /// Subscribe to the lifecycle event stream.
    fn events(&self) -> PySandboxEventStream {
        PySandboxEventStream {
            inner: Arc::new(AsyncMutex::new(self.inner.events())),
        }
    }

    /// Async ephemeral one-shot: run an `ExecRequest` dict in a fresh sandbox
    /// and resolve to the result dict.
    fn run<'py>(&self, py: Python<'py>, request: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let exec = exec_from_py(py, request)?;
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let res = inner
                .run_exec(exec, None, SandboxBackendSel::Auto)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| exec_result_to_py(py, &res))
        })
    }

    /// Async: provision a persistent `Sandbox` with the given profile and
    /// optional backend config.
    #[pyo3(signature = (profile, config=None))]
    fn create<'py>(
        &self,
        py: Python<'py>,
        profile: PySandboxProfile,
        config: Option<PyRef<'py, PySandboxConfig>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sel = config.map(|c| c.sel.clone()).unwrap_or_default();
        let req = CreateSandbox::new(profile.to_core()).with_backend(sel);
        let harness = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let info = harness
                .create(req)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PySandbox { harness: harness.clone(), id: info.id },
                )
            })
        })
    }

    fn __repr__(&self) -> String {
        format!("SandboxClient(backend={})", self.inner.backend_name())
    }
}

// ----- module registration -----------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "sandbox")?;
    m.add_class::<PySandboxClient>()?;
    m.add_class::<PySandbox>()?;
    m.add_class::<PySandboxConfig>()?;
    m.add_class::<PySandboxProfile>()?;
    m.add_class::<PySandboxEventStream>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
