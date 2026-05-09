//! Shared conversion helpers between Python objects and serde_json /
//! semver values. Used by every submodule that round-trips arbitrary
//! JSON-shaped payloads (registry artifacts, tool args, memory items,
//! eval cases, …).

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use semver::Version;

/// Convert a `serde_json::Value` to a Python object via the stdlib
/// `json` module. The round-trip avoids hand-rolling Bound→PyAny
/// constructors for each numeric type.
pub fn json_to_py(py: Python<'_>, v: &serde_json::Value) -> PyResult<PyObject> {
    let s = serde_json::to_string(v).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let json = py.import_bound("json")?;
    let loads = json.getattr("loads")?;
    let obj = loads.call1((s,))?;
    Ok(obj.unbind())
}

/// Inverse of `json_to_py`: serialize a Python object back to
/// `serde_json::Value`. Reuses the stdlib `json.dumps` for type
/// coercion (handles dict/list/str/int/float/bool/None).
pub fn py_to_json(_py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    let json = obj.py().import_bound("json")?;
    let dumps = json.getattr("dumps")?;
    let s: String = dumps.call1((obj,))?.extract()?;
    serde_json::from_str(&s).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// Parse a SemVer string, mapping parse errors to a `ValueError`.
pub fn parse_version(s: &str) -> PyResult<Version> {
    Version::parse(s).map_err(|e| PyValueError::new_err(e.to_string()))
}
