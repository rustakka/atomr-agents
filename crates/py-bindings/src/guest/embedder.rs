//! Python class → Rust `Embedder` adapter.
//!
//! - `dim()` — sync, returns an int.
//! - `embed(text)` — async, returns a list of floats.
//! - `embed_batch(texts)` — async, returns a list of lists of floats.
//!
//! `dim()` is called inside the GIL synchronously; the others go
//! through the coroutine-aware path.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result as AgentResult};
use atomr_agents_embed::Embedder;
use pyo3::prelude::*;

use super::conv_helpers::{await_and_jsonify, resolve_instance};
use super::registry::GUESTS;

pub struct PyEmbedderAdapter {
    target: Arc<PyObject>,
    label: String,
}

impl PyEmbedderAdapter {
    pub(crate) fn new(target: Arc<PyObject>, label: String) -> Self {
        Self { target, label }
    }
}

#[async_trait]
impl Embedder for PyEmbedderAdapter {
    fn dim(&self) -> usize {
        // Sync method — best-effort. If the call fails (target not
        // callable, missing dim method), we log via stderr and return
        // 0 to avoid panicking inside a `#[pymethod]`-adjacent context.
        let target = self.target.clone();
        Python::with_gil(|py| -> PyResult<usize> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "dim")?;
            let m = instance.getattr("dim")?;
            let result = m.call0()?;
            result.extract::<usize>()
        })
        .unwrap_or_else(|e| {
            eprintln!("guest embedder {} dim error: {e}", self.label);
            0
        })
    }

    async fn embed(&self, text: &str) -> AgentResult<Vec<f32>> {
        let target = self.target.clone();
        let label = self.label.clone();
        let text = text.to_string();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "embed")?;
            let m = instance.getattr("embed")?;
            let result = m.call1((text,))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Inference(format!("guest embedder {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Inference(format!("guest embedder {label}: {e}")))?;

        parse_f32_array(&value).map_err(|e| {
            AgentError::Inference(format!("guest embedder {label}: {e}"))
        })
    }

    async fn embed_batch(&self, texts: &[String]) -> AgentResult<Vec<Vec<f32>>> {
        let target = self.target.clone();
        let label = self.label.clone();
        let owned: Vec<String> = texts.to_vec();

        let returned = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance = resolve_instance(bound, "embed_batch")?;
            // If the user only implemented `embed`, fall back to the
            // default trait body via N sequential calls — handled
            // below by returning None and letting the caller iterate.
            let m = instance.getattr("embed_batch")?;
            let result = m.call1((owned,))?;
            Ok(result.unbind())
        })
        .map_err(|e| AgentError::Inference(format!("guest embedder {label}: {e}")))?;

        let value = await_and_jsonify(returned)
            .await
            .map_err(|e| AgentError::Inference(format!("guest embedder {label}: {e}")))?;

        let arr = value.as_array().ok_or_else(|| {
            AgentError::Inference(format!(
                "guest embedder {label}: expected array, got {value}"
            ))
        })?;
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            out.push(parse_f32_array(v).map_err(|e| {
                AgentError::Inference(format!("guest embedder {label}: {e}"))
            })?);
        }
        Ok(out)
    }
}

fn parse_f32_array(v: &serde_json::Value) -> Result<Vec<f32>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| format!("expected array, got {v}"))?;
    let mut out = Vec::with_capacity(arr.len());
    for x in arr {
        let f = x
            .as_f64()
            .ok_or_else(|| format!("expected number, got {x}"))?;
        out.push(f as f32);
    }
    Ok(out)
}

#[pyclass(name = "GuestEmbedder", module = "atomr_agents._native.guest")]
#[derive(Clone)]
pub struct PyEmbedder {
    /// Held for downstream APIs (e.g. an embedding-backed retriever
    /// constructed from Python) that take a `Box<dyn Embedder>`.
    #[allow(dead_code)]
    pub(crate) inner: Arc<dyn Embedder>,
    #[pyo3(get)]
    pub key: String,
}

#[pymethods]
impl PyEmbedder {
    fn __repr__(&self) -> String {
        format!("GuestEmbedder(key={:?})", self.key)
    }
}

#[pyfunction]
pub(super) fn build_guest_embedder(key: String) -> PyResult<PyEmbedder> {
    let entry = GUESTS
        .get(&("embedder".to_string(), key.clone()))
        .ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!(
                "no embedder registered with key {key:?}"
            ))
        })?;
    let target = entry.value().clone();
    let adapter = PyEmbedderAdapter::new(target, key.clone());
    Ok(PyEmbedder {
        inner: Arc::new(adapter),
        key,
    })
}
