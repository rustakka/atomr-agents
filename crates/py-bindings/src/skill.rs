//! Skills — instruction fragments + tool overlays.

use std::sync::Arc;

use atomr_agents_core::{SkillId, ToolId};
use atomr_agents_skill::{KeywordSkillStrategy, Skill, SkillSet, StaticSkillStrategy};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use semver::Version;

use crate::conv::parse_version;
use crate::core::PyMemoryNamespace;
use crate::strategy::PySkillStrategy;

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

// ----- Strategy factories --------------------------------------------------

/// Build a `SkillStrategy` that always reports the same fixed set of
/// skills as applicable.
#[pyfunction]
fn static_skill_strategy(skills: Vec<PySkill>) -> PySkillStrategy {
    let skills: Vec<Skill> = skills.into_iter().map(|s| s.inner).collect();
    PySkillStrategy {
        inner: Arc::new(StaticSkillStrategy::new(skills)),
    }
}

/// Build a `SkillStrategy` that selects skills whose configured
/// trigger keywords appear in the user turn.
///
/// `keywords` maps `skill_name -> [trigger_word, ...]`. Entries in
/// the dict replace any keywords already attached to the skill; a
/// skill missing from the dict falls back to its own
/// `Skill.keywords` list.
#[pyfunction]
fn keyword_skill_strategy(
    skills: Vec<PySkill>,
    keywords: &Bound<'_, PyDict>,
) -> PyResult<PySkillStrategy> {
    let mut merged: Vec<Skill> = Vec::with_capacity(skills.len());
    for s in skills {
        let mut skill = s.inner;
        if let Some(v) = keywords.get_item(&skill.name)? {
            let words: Vec<String> = v.extract()?;
            skill.keywords = words;
        }
        merged.push(skill);
    }
    Ok(PySkillStrategy {
        inner: Arc::new(KeywordSkillStrategy::new(merged)),
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "skill")?;
    m.add_class::<PySkill>()?;
    m.add_class::<PySkillSet>()?;
    m.add_function(wrap_pyfunction!(static_skill_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(keyword_skill_strategy, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
