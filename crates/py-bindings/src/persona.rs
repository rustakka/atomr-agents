//! Persona — identity / style / archetype rendering.
//!
//! Phase B exposes the rendered shape (`RenderedPersona`) and a thin
//! `StaticPersonaStrategy` wrapper. Strategy variants (Big Five, MBTI,
//! Jungian, Composite, Adaptive) live in `atomr_agents_persona` as
//! Rust-side strategies; they're constructable from Python via
//! `crate::guest::register_persona_factory`.

use atomr_agents_persona::RenderedPersona;
use pyo3::prelude::*;

#[pyclass(name = "RenderedPersona", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyRenderedPersona {
    pub(crate) inner: RenderedPersona,
}

#[pymethods]
impl PyRenderedPersona {
    #[new]
    fn new(identity: String, estimated_tokens: u32) -> Self {
        Self {
            inner: RenderedPersona {
                identity,
                estimated_tokens,
                ..Default::default()
            },
        }
    }

    #[getter]
    fn identity(&self) -> &str {
        &self.inner.identity
    }

    #[getter]
    fn estimated_tokens(&self) -> u32 {
        self.inner.estimated_tokens
    }

    fn __repr__(&self) -> String {
        format!(
            "RenderedPersona(identity={:?}, est_tokens={})",
            self.inner.identity, self.inner.estimated_tokens
        )
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "persona")?;
    m.add_class::<PyRenderedPersona>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
