//! Python bindings for atomr-agents.
//!
//! Module exposes a working "host mode" surface — Python code can
//! construct a `Registry`, publish/look-up artifacts, and subscribe
//! to an `EventBus`. Full guest mode (Python-defined `@tool` /
//! `@strategy` decorators executing inside Rust actors) layers on
//! top of `atomr/crates/py-bindings/pycore`'s subinterpreter-pool
//! dispatcher. Wiring that requires the atomr `_native` extension as
//! a runtime dep, which is a maturin-built artifact and lives on the
//! Python side; the Rust shape below is the surface those wrappers
//! consume.

use std::sync::Arc;

use atomr_agents_core::{AgentId, Event, EventEnvelope, ToolId};
use atomr_agents_observability::EventBus;
use atomr_agents_registry::{ArtifactKind, ArtifactRecord, EvalSummary, Registry};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use semver::Version;

/// Convert a `serde_json::Value` to a Python object.
fn json_to_py(py: Python<'_>, v: &serde_json::Value) -> PyResult<PyObject> {
    let s = serde_json::to_string(v).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let json = py.import_bound("json")?;
    let loads = json.getattr("loads")?;
    let obj = loads.call1((s,))?;
    Ok(obj.unbind())
}

fn py_to_json(_py: Python<'_>, obj: &Bound<PyAny>) -> PyResult<serde_json::Value> {
    let json = obj.py().import_bound("json")?;
    let dumps = json.getattr("dumps")?;
    let s: String = dumps.call1((obj,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

#[pyclass(name = "Event", module = "atomr_agents._native")]
struct PyEvent {
    inner: EventEnvelope,
}

#[pymethods]
impl PyEvent {
    #[getter]
    fn timestamp_ms(&self) -> i64 {
        self.inner.timestamp_ms
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner.event {
            Event::StrategyResolved { .. } => "strategy_resolved",
            Event::ToolInvoked { .. } => "tool_invoked",
            Event::AgentTurn { .. } => "agent_turn",
            Event::WorkflowStep { .. } => "workflow_step",
            Event::HarnessIteration { .. } => "harness_iteration",
            Event::Backpressure { .. } => "backpressure",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }
}

#[pyclass(name = "EventBus", module = "atomr_agents._native")]
struct PyEventBus {
    inner: EventBus,
}

#[pymethods]
impl PyEventBus {
    #[new]
    fn new() -> Self {
        Self {
            inner: EventBus::new(),
        }
    }

    /// Register a Python callable that receives every emitted event
    /// as a dict.
    fn subscribe(&self, callback: PyObject) -> PyResult<()> {
        let cb = Arc::new(callback);
        self.inner.subscribe(move |env: &EventEnvelope| {
            let cb = cb.clone();
            let env = env.clone();
            Python::with_gil(|py| {
                if let Ok(pyev) = Py::new(py, PyEvent { inner: env }).and_then(|e| Ok(e.into_py(py))) {
                    let _ = cb.call1(py, (pyev,));
                }
            });
        });
        Ok(())
    }

    /// Emit a tool-invoked event from Python (handy for testing).
    fn emit_tool_invoked(&self, tool_id: String, args_hash: u64, elapsed_ms: u64, ok: bool) -> PyResult<()> {
        self.inner.emit(Event::ToolInvoked {
            tool_id: ToolId::from(tool_id),
            args_hash,
            elapsed_ms,
            ok,
        });
        Ok(())
    }

    /// Emit an agent-turn event from Python.
    fn emit_agent_turn(
        &self,
        agent_id: String,
        input_tokens: u32,
        output_tokens: u32,
        elapsed_ms: u64,
    ) -> PyResult<()> {
        self.inner.emit(Event::AgentTurn {
            agent_id: AgentId::from(agent_id),
            input_tokens,
            output_tokens,
            finish_reason: None,
            elapsed_ms,
        });
        Ok(())
    }
}

fn parse_kind(s: &str) -> PyResult<ArtifactKind> {
    Ok(match s {
        "tool_set" => ArtifactKind::ToolSet,
        "skill" => ArtifactKind::Skill,
        "persona" => ArtifactKind::Persona,
        "agent" => ArtifactKind::Agent,
        "workflow" => ArtifactKind::Workflow,
        "harness" => ArtifactKind::Harness,
        "harness_set" => ArtifactKind::HarnessSet,
        other => return Err(PyValueError::new_err(format!("unknown artifact kind {other}"))),
    })
}

fn parse_version(s: &str) -> PyResult<Version> {
    Version::parse(s).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyclass(name = "Registry", module = "atomr_agents._native")]
struct PyRegistry {
    inner: Registry,
}

#[pymethods]
impl PyRegistry {
    #[new]
    fn new() -> Self {
        Self {
            inner: Registry::new(),
        }
    }

    /// Publish an artifact unconditionally. `payload` is any
    /// JSON-serializable Python value.
    fn publish(&self, kind: &str, id: String, version: &str, payload: &Bound<PyAny>) -> PyResult<()> {
        let kind = parse_kind(kind)?;
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

    /// Publish gated by an eval-regression check.
    #[pyo3(signature = (kind, id, version, payload, current_pass_rate, baseline_pass_rate=None, tolerance=0.05))]
    fn publish_gated(
        &self,
        kind: &str,
        id: String,
        version: &str,
        payload: &Bound<PyAny>,
        current_pass_rate: f32,
        baseline_pass_rate: Option<f32>,
        tolerance: f32,
    ) -> PyResult<()> {
        let kind = parse_kind(kind)?;
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
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(())
    }

    /// Get a specific version. Returns the payload as a Python dict.
    fn get(&self, py: Python<'_>, kind: &str, id: &str, version: &str) -> PyResult<Option<PyObject>> {
        let kind = parse_kind(kind)?;
        let version = parse_version(version)?;
        Ok(self.inner.get(kind, id, &version).map(|r| {
            let dict = PyDict::new_bound(py);
            dict.set_item("id", r.id.clone()).unwrap();
            dict.set_item("version", r.version.to_string()).unwrap();
            if let Ok(payload_obj) = json_to_py(py, &r.payload) {
                dict.set_item("payload", payload_obj).unwrap();
            }
            dict.unbind().into()
        }))
    }

    /// Get the highest version.
    fn latest(&self, py: Python<'_>, kind: &str, id: &str) -> PyResult<Option<PyObject>> {
        let kind = parse_kind(kind)?;
        Ok(self.inner.latest(kind, id).map(|r| {
            let dict = PyDict::new_bound(py);
            dict.set_item("id", r.id.clone()).unwrap();
            dict.set_item("version", r.version.to_string()).unwrap();
            if let Ok(payload_obj) = json_to_py(py, &r.payload) {
                dict.set_item("payload", payload_obj).unwrap();
            }
            dict.unbind().into()
        }))
    }
}

/// Module init. Exposed as `atomr_agents._native`.
#[pymodule]
fn atomr_agents_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyEvent>()?;
    m.add_class::<PyEventBus>()?;
    m.add_class::<PyRegistry>()?;
    Ok(())
}
