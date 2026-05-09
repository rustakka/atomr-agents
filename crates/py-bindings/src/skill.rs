//! Skills — instruction fragments + tool overlays.

use atomr_agents_core::{SkillId, ToolId};
use atomr_agents_skill::{Skill, SkillSet};
use pyo3::prelude::*;
use semver::Version;

use crate::conv::parse_version;
use crate::core::PyMemoryNamespace;

#[pyclass(name = "Skill", module = "atomr_agents._native.skill")]
#[derive(Clone)]
pub struct PySkill {
    pub(crate) inner: Skill,
}

#[pymethods]
impl PySkill {
    #[new]
    #[pyo3(signature = (id, name, instruction_fragment, tool_overlay=None, memory_namespace=None, keywords=None, priority=5))]
    fn new(
        id: String,
        name: String,
        instruction_fragment: String,
        tool_overlay: Option<Vec<String>>,
        memory_namespace: Option<PyMemoryNamespace>,
        keywords: Option<Vec<String>>,
        priority: u8,
    ) -> Self {
        Self {
            inner: Skill {
                id: SkillId::from(id),
                name,
                instruction_fragment,
                tool_overlay: tool_overlay
                    .unwrap_or_default()
                    .into_iter()
                    .map(ToolId::from)
                    .collect(),
                memory_namespace: memory_namespace.map(|n| n.inner),
                keywords: keywords.unwrap_or_default(),
                priority,
            },
        }
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }
    #[getter]
    fn instruction_fragment(&self) -> &str {
        &self.inner.instruction_fragment
    }
    #[getter]
    fn tool_overlay(&self) -> Vec<String> {
        self.inner
            .tool_overlay
            .iter()
            .map(|t| t.as_str().to_string())
            .collect()
    }
    #[getter]
    fn keywords(&self) -> Vec<String> {
        self.inner.keywords.clone()
    }
    #[getter]
    fn priority(&self) -> u8 {
        self.inner.priority
    }

    fn __repr__(&self) -> String {
        format!(
            "Skill(id={:?}, name={:?}, priority={})",
            self.inner.id.as_str(),
            self.inner.name,
            self.inner.priority,
        )
    }
}

#[pyclass(name = "SkillSet", module = "atomr_agents._native.skill")]
#[derive(Clone)]
pub struct PySkillSet {
    pub(crate) inner: SkillSet,
}

#[pymethods]
impl PySkillSet {
    #[new]
    fn new(id: String, version: &str, skills: Vec<PySkill>) -> PyResult<Self> {
        let v: Version = parse_version(version)?;
        Ok(Self {
            inner: SkillSet {
                id,
                version: v,
                skills: skills.into_iter().map(|s| s.inner).collect(),
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    fn __len__(&self) -> usize {
        self.inner.skills.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "SkillSet(id={:?}, version={:?}, skills={})",
            self.inner.id,
            self.inner.version.to_string(),
            self.inner.skills.len()
        )
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "skill")?;
    m.add_class::<PySkill>()?;
    m.add_class::<PySkillSet>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
