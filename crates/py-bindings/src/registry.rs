//! Versioned artifact registry. Mirrors the existing 0.2.x surface
//! (`publish`, `publish_gated`, `get`, `latest`) and adds:
//!   - `ArtifactKind` as a string-tagged enum class
//!   - `ArtifactRecord` builder/getter wrapper
//!   - `EvalSummary` data class
//!   - sync `list()` helper

use atomr_agents_core::AgentError;
use atomr_agents_registry::{ArtifactKind, ArtifactRecord, EvalSummary, Registry};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::conv::{json_to_py, parse_version, py_to_json};
use crate::errors;

// ----- ArtifactKind ---------------------------------------------------------

#[pyclass(
    name = "ArtifactKind",
    module = "atomr_agents._native.registry",
    eq,
    hash,
    frozen
)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyArtifactKind {
    pub(crate) inner: ArtifactKind,
}

impl PyArtifactKind {
    pub(crate) fn from_str(s: &str) -> PyResult<Self> {
        let inner = match s {
            "tool_set" => ArtifactKind::ToolSet,
            "skill" => ArtifactKind::Skill,
            "persona" => ArtifactKind::Persona,
            "agent" => ArtifactKind::Agent,
            "workflow" => ArtifactKind::Workflow,
            "harness" => ArtifactKind::Harness,
            "harness_set" => ArtifactKind::HarnessSet,
            "channel" => ArtifactKind::Channel,
            "avatar" => ArtifactKind::Avatar,
            other => {
                return Err(PyValueError::new_err(format!("unknown artifact kind: {other:?}")));
            }
        };
        Ok(Self { inner })
    }
}

#[pymethods]
impl PyArtifactKind {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        Self::from_str(name)
    }

    #[getter]
    fn name(&self) -> &'static str {
        match self.inner {
            ArtifactKind::ToolSet => "tool_set",
            ArtifactKind::Skill => "skill",
            ArtifactKind::Persona => "persona",
            ArtifactKind::Agent => "agent",
            ArtifactKind::Workflow => "workflow",
            ArtifactKind::Harness => "harness",
            ArtifactKind::HarnessSet => "harness_set",
            ArtifactKind::Channel => "channel",
            ArtifactKind::Avatar => "avatar",
        }
    }

    #[staticmethod]
    fn tool_set() -> Self {
        Self {
            inner: ArtifactKind::ToolSet,
        }
    }
    #[staticmethod]
    fn skill() -> Self {
        Self {
            inner: ArtifactKind::Skill,
        }
    }
    #[staticmethod]
    fn persona() -> Self {
        Self {
            inner: ArtifactKind::Persona,
        }
    }
    #[staticmethod]
    fn agent() -> Self {
        Self {
            inner: ArtifactKind::Agent,
        }
    }
    #[staticmethod]
    fn workflow() -> Self {
        Self {
            inner: ArtifactKind::Workflow,
        }
    }
    #[staticmethod]
    fn harness() -> Self {
        Self {
            inner: ArtifactKind::Harness,
        }
    }
    #[staticmethod]
    fn harness_set() -> Self {
        Self {
            inner: ArtifactKind::HarnessSet,
        }
    }
    #[staticmethod]
    fn channel() -> Self {
        Self {
            inner: ArtifactKind::Channel,
        }
    }
    #[staticmethod]
    fn avatar() -> Self {
        Self {
            inner: ArtifactKind::Avatar,
        }
    }

    fn __repr__(&self) -> String {
        format!("ArtifactKind({:?})", self.name())
    }
}

// ----- ArtifactRecord -------------------------------------------------------

#[pyclass(name = "ArtifactRecord", module = "atomr_agents._native.registry")]
#[derive(Clone)]
pub struct PyArtifactRecord {
    pub(crate) inner: ArtifactRecord,
}

#[pymethods]
impl PyArtifactRecord {
    #[getter]
    fn kind(&self) -> PyArtifactKind {
        PyArtifactKind {
            inner: self.inner.kind,
        }
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    #[getter]
    fn published_at_ms(&self) -> i64 {
        self.inner.published_at_ms
    }

    #[getter]
    fn baseline_pass_rate(&self) -> Option<f32> {
        self.inner.baseline_pass_rate
    }

    #[getter]
    fn current_pass_rate(&self) -> Option<f32> {
        self.inner.current_pass_rate
    }

    fn payload(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.payload)
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        dict.set_item("kind", self.kind().name())?;
        dict.set_item("id", &self.inner.id)?;
        dict.set_item("version", self.inner.version.to_string())?;
        dict.set_item("payload", json_to_py(py, &self.inner.payload)?)?;
        dict.set_item("published_at_ms", self.inner.published_at_ms)?;
        if let Some(p) = self.inner.baseline_pass_rate {
            dict.set_item("baseline_pass_rate", p)?;
        }
        if let Some(p) = self.inner.current_pass_rate {
            dict.set_item("current_pass_rate", p)?;
        }
        Ok(dict.unbind().into())
    }

    fn __repr__(&self) -> String {
        format!(
            "ArtifactRecord(kind={:?}, id={:?}, version={:?})",
            self.kind().name(),
            self.inner.id,
            self.inner.version.to_string()
        )
    }
}

// ----- EvalSummary ----------------------------------------------------------

#[pyclass(name = "EvalSummary", module = "atomr_agents._native.registry")]
#[derive(Clone)]
pub struct PyEvalSummary {
    pub(crate) pass_rate: f32,
}

#[pymethods]
impl PyEvalSummary {
    #[new]
    fn new(pass_rate: f32) -> Self {
        Self { pass_rate }
    }

    #[getter]
    fn pass_rate(&self) -> f32 {
        self.pass_rate
    }

    fn __repr__(&self) -> String {
        format!("EvalSummary(pass_rate={:.3})", self.pass_rate)
    }
}

// ----- Registry ------------------------------------------------------------

#[pyclass(name = "Registry", module = "atomr_agents._native.registry")]
pub struct PyRegistry {
    pub(crate) inner: Registry,
}

#[pymethods]
impl PyRegistry {
    #[new]
    fn new() -> Self {
        Self {
            inner: Registry::new(),
        }
    }

    /// Publish an artifact unconditionally. Sync — the underlying
    /// registry is in-memory.
    fn publish(&self, kind: &str, id: String, version: &str, payload: &Bound<'_, PyAny>) -> PyResult<()> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        let version = parse_version(version)?;
        let payload_v = py_to_json(payload.py(), payload)?;
        self.inner.publish(ArtifactRecord {
            kind,
            id,
            version,
            payload: payload_v,
            published_at_ms: chrono::Utc::now().timestamp_millis(),
            baseline_pass_rate: None,
            current_pass_rate: None,
        });
        Ok(())
    }

    /// Async variant — mirrors atomr-infer's async publish surface.
    /// The body still completes synchronously today (DashMap-backed),
    /// but exposing it via `future_into_py` keeps the API stable for
    /// future durable backends.
    fn publish_async<'py>(
        &self,
        py: Python<'py>,
        kind: &str,
        id: String,
        version: &str,
        payload: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        let version = parse_version(version)?;
        let payload_v = py_to_json(payload.py(), payload)?;
        let registry = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            registry.publish(ArtifactRecord {
                kind,
                id,
                version,
                payload: payload_v,
                published_at_ms: chrono::Utc::now().timestamp_millis(),
                baseline_pass_rate: None,
                current_pass_rate: None,
            });
            Ok(())
        })
    }

    #[pyo3(signature = (kind, id, version, payload, current_pass_rate, baseline_pass_rate=None, tolerance=0.05))]
    fn publish_gated(
        &self,
        kind: &str,
        id: String,
        version: &str,
        payload: &Bound<'_, PyAny>,
        current_pass_rate: f32,
        baseline_pass_rate: Option<f32>,
        tolerance: f32,
    ) -> PyResult<()> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        let version = parse_version(version)?;
        let payload_v = py_to_json(payload.py(), payload)?;
        let baseline = baseline_pass_rate.map(|p| EvalSummary { pass_rate: p });
        let current = EvalSummary {
            pass_rate: current_pass_rate,
        };
        self.inner
            .publish_gated(
                ArtifactRecord {
                    kind,
                    id,
                    version,
                    payload: payload_v,
                    published_at_ms: chrono::Utc::now().timestamp_millis(),
                    baseline_pass_rate: None,
                    current_pass_rate: None,
                },
                baseline.as_ref(),
                &current,
                tolerance,
            )
            .map_err(|e: AgentError| PyErr::new::<errors::RegistryError, _>(e.to_string()))?;
        Ok(())
    }

    /// Get a specific version. Returns the payload as a Python dict
    /// or `None` when the version does not exist.
    fn get(&self, py: Python<'_>, kind: &str, id: &str, version: &str) -> PyResult<Option<PyObject>> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        let version = parse_version(version)?;
        Ok(self
            .inner
            .get(kind, id, &version)
            .map(|r| record_to_dict(py, &r).unbind().into()))
    }

    /// Get the highest version, or `None`.
    fn latest(&self, py: Python<'_>, kind: &str, id: &str) -> PyResult<Option<PyObject>> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        Ok(self
            .inner
            .latest(kind, id)
            .map(|r| record_to_dict(py, &r).unbind().into()))
    }

    /// List every published record for a given artifact kind.
    fn list(&self, py: Python<'_>, kind: &str) -> PyResult<Vec<PyObject>> {
        let kind = PyArtifactKind::from_str(kind)?.inner;
        Ok(self
            .inner
            .list(kind)
            .into_iter()
            .map(|r| record_to_dict(py, &r).unbind().into())
            .collect())
    }
}

// Local helper — kept here rather than on PyArtifactRecord because
// the registry returns `Arc<ArtifactRecord>` and we want to return a
// dict directly to keep the existing test_smoke.py contract.
fn record_to_dict<'py>(py: Python<'py>, r: &ArtifactRecord) -> Bound<'py, PyDict> {
    let dict = PyDict::new_bound(py);
    let _ = dict.set_item("id", r.id.clone());
    let _ = dict.set_item("version", r.version.to_string());
    if let Ok(p) = json_to_py(py, &r.payload) {
        let _ = dict.set_item("payload", p);
    }
    let _ = dict.set_item("published_at_ms", r.published_at_ms);
    if let Some(p) = r.baseline_pass_rate {
        let _ = dict.set_item("baseline_pass_rate", p);
    }
    if let Some(p) = r.current_pass_rate {
        let _ = dict.set_item("current_pass_rate", p);
    }
    dict
}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "registry")?;
    m.add_class::<PyArtifactKind>()?;
    m.add_class::<PyArtifactRecord>()?;
    m.add_class::<PyEvalSummary>()?;
    m.add_class::<PyRegistry>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
