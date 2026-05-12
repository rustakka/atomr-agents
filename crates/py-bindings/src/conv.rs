//! Shared conversion helpers between Python objects and serde_json /
//! semver values. Used by every submodule that round-trips arbitrary
//! JSON-shaped payloads (registry artifacts, tool args, memory items,
//! eval cases, …).

use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use semver::Version;
use std::time::Duration;

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

/// Like `py_to_json` but treats `None`/missing as the caller-provided
/// default. Useful for callable inputs where a caller might pass
/// nothing to mean "empty object".
pub fn py_to_value_or(
    py: Python<'_>,
    obj: Option<&Bound<'_, PyAny>>,
    default: serde_json::Value,
) -> PyResult<serde_json::Value> {
    match obj {
        Some(b) if !b.is_none() => py_to_json(py, b),
        _ => Ok(default),
    }
}

/// Parse a SemVer string, mapping parse errors to a `ValueError`.
pub fn parse_version(s: &str) -> PyResult<Version> {
    Version::parse(s).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Build a `CallCtx` from an optional Python dict. Recognised keys
/// (all optional, fall back to plentiful defaults): `agent_id`,
/// `tokens` / `token_budget`, `time_ms` / `time_budget_ms`,
/// `money_usd` / `money_budget_usd`, `iterations` / `iteration_budget`,
/// `trace` (list of strings). Used by every async adapter that
/// receives a ctx from Python.
pub fn callctx_from_pydict(
    py: Python<'_>,
    ctx_dict: Option<&Bound<'_, PyAny>>,
) -> PyResult<CallCtx> {
    let mut agent_id: Option<atomr_agents_core::AgentId> = None;
    let mut tokens = TokenBudget::new(1_000_000);
    let mut time = TimeBudget::new(Duration::from_secs(3600));
    let mut money = MoneyBudget::from_usd(1_000.0);
    let mut iterations = IterationBudget::new(1_000);
    let mut trace: Vec<String> = Vec::new();

    if let Some(d) = ctx_dict {
        if d.is_none() {
            return Ok(CallCtx {
                agent_id,
                tokens,
                time,
                money,
                iterations,
                trace,
            });
        }
        if let Ok(dict) = d.downcast::<PyDict>() {
            if let Some(v) = dict.get_item("agent_id")? {
                if !v.is_none() {
                    let s: String = v.extract()?;
                    agent_id = Some(atomr_agents_core::AgentId::from(s));
                }
            }
            for key in ["tokens", "token_budget"] {
                if let Some(v) = dict.get_item(key)? {
                    if !v.is_none() {
                        let n: u32 = v.extract()?;
                        tokens = TokenBudget::new(n);
                    }
                }
            }
            for key in ["time_ms", "time_budget_ms"] {
                if let Some(v) = dict.get_item(key)? {
                    if !v.is_none() {
                        let n: u64 = v.extract()?;
                        time = TimeBudget::new(Duration::from_millis(n));
                    }
                }
            }
            for key in ["money_usd", "money_budget_usd"] {
                if let Some(v) = dict.get_item(key)? {
                    if !v.is_none() {
                        let n: f64 = v.extract()?;
                        money = MoneyBudget::from_usd(n);
                    }
                }
            }
            for key in ["iterations", "iteration_budget"] {
                if let Some(v) = dict.get_item(key)? {
                    if !v.is_none() {
                        let n: u32 = v.extract()?;
                        iterations = IterationBudget::new(n);
                    }
                }
            }
            if let Some(v) = dict.get_item("trace")? {
                if !v.is_none() {
                    trace = v.extract()?;
                }
            }
        } else {
            return Err(PyValueError::new_err(
                "ctx must be a dict or None",
            ));
        }
    }
    let _ = py;
    Ok(CallCtx {
        agent_id,
        tokens,
        time,
        money,
        iterations,
        trace,
    })
}

/// Inverse of `callctx_from_pydict` — present a `CallCtx` as a dict so
/// Python callbacks can introspect budgets/trace.
pub fn callctx_to_pydict<'py>(py: Python<'py>, ctx: &CallCtx) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new_bound(py);
    if let Some(aid) = &ctx.agent_id {
        d.set_item("agent_id", aid.as_str())?;
    }
    d.set_item("tokens_remaining", ctx.tokens.remaining)?;
    d.set_item("time_ms_remaining", ctx.time.remaining_ms)?;
    d.set_item("money_micro_usd_remaining", ctx.money.remaining_micro_usd)?;
    d.set_item("iterations_remaining", ctx.iterations.remaining)?;
    d.set_item("trace", ctx.trace.clone())?;
    Ok(d)
}
