//! Persona — identity / style / archetype rendering.
//!
//! Exposes the persona value types (`Persona`, `PersonaSet`,
//! `PersonaMetadata`, `StyleSpec`, `TraitFragment`, `RenderedPersona`),
//! the structural strategy framework (`PersonaStrategy` dyn handle plus
//! concrete factories: static / Big Five / MBTI / Jungian / composite /
//! from-factory), and the emphasis strategy family
//! (`PersonaEmphasisStrategy` dyn handle plus
//! static / task / audience / mood / goal factories). The Python-side
//! guest pattern goes through `register_persona_factory` and is
//! materialised via `persona_strategy_from_factory(key)`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, AgentError, Result as AgentResult, TokenBudget};
use atomr_agents_persona::{
    Archetype, AudienceAdaptive, CognitiveFunction, CognitiveStack, CompositePersonaStrategy,
    GoalConditioned, JungianArchetypeStrategy, MbtiType, MoodState, Persona, PersonaEmphasisStrategy,
    PersonaMetadata, PersonaReconciler, PersonaSet, PersonaStrategy, RenderedPersona, StaticEmphasis,
    StaticPersonaStrategy, StyleSpec, TaskAdaptive, TraitFragment,
};
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use semver::Version;

use crate::conv::{parse_version, py_to_json};
use crate::strategy::{agent_context_to_pydict, await_if_coro};

// ----- StyleSpec -----------------------------------------------------------

#[pyclass(name = "StyleSpec", module = "atomr_agents._native.persona")]
#[derive(Clone, Default)]
pub struct PyStyleSpec {
    pub(crate) inner: StyleSpec,
}

#[pymethods]
impl PyStyleSpec {
    #[new]
    #[pyo3(signature = (tone=None, register=None, verbosity=None))]
    fn new(tone: Option<String>, register: Option<String>, verbosity: Option<u8>) -> Self {
        Self {
            inner: StyleSpec {
                tone,
                register,
                verbosity,
            },
        }
    }

    #[getter]
    fn tone(&self) -> Option<&str> {
        self.inner.tone.as_deref()
    }

    #[getter]
    fn register(&self) -> Option<&str> {
        self.inner.register.as_deref()
    }

    #[getter]
    fn verbosity(&self) -> Option<u8> {
        self.inner.verbosity
    }

    fn __repr__(&self) -> String {
        format!(
            "StyleSpec(tone={:?}, register={:?}, verbosity={:?})",
            self.inner.tone, self.inner.register, self.inner.verbosity
        )
    }
}

// ----- PersonaMetadata -----------------------------------------------------

#[pyclass(name = "PersonaMetadata", module = "atomr_agents._native.persona")]
#[derive(Clone, Default)]
pub struct PyPersonaMetadata {
    pub(crate) inner: PersonaMetadata,
}

#[pymethods]
impl PyPersonaMetadata {
    #[new]
    #[pyo3(signature = (framework=None))]
    fn new(framework: Option<String>) -> Self {
        Self {
            inner: PersonaMetadata { framework },
        }
    }

    #[getter]
    fn framework(&self) -> Option<&str> {
        self.inner.framework.as_deref()
    }

    fn __repr__(&self) -> String {
        format!("PersonaMetadata(framework={:?})", self.inner.framework)
    }
}

// ----- TraitFragment -------------------------------------------------------

#[pyclass(name = "TraitFragment", module = "atomr_agents._native.persona")]
#[derive(Clone, Default)]
pub struct PyTraitFragment {
    pub(crate) inner: TraitFragment,
}

#[pymethods]
impl PyTraitFragment {
    #[new]
    fn new(label: String, weight: f32, description: String) -> Self {
        Self {
            inner: TraitFragment {
                label,
                weight,
                description,
            },
        }
    }

    #[getter]
    fn label(&self) -> &str {
        &self.inner.label
    }

    #[getter]
    fn weight(&self) -> f32 {
        self.inner.weight
    }

    #[getter]
    fn description(&self) -> &str {
        &self.inner.description
    }

    fn __repr__(&self) -> String {
        format!(
            "TraitFragment(label={:?}, weight={}, description={:?})",
            self.inner.label, self.inner.weight, self.inner.description
        )
    }
}

// ----- Persona (concrete data class, not the trait) ------------------------

#[pyclass(name = "PersonaValue", module = "atomr_agents._native.persona")]
#[derive(Clone, Default)]
pub struct PyPersona {
    pub(crate) inner: Persona,
}

#[pymethods]
impl PyPersona {
    #[new]
    #[pyo3(signature = (identity, salient_traits=Vec::new(), style=None, metadata=None))]
    fn new(
        identity: String,
        salient_traits: Vec<PyTraitFragment>,
        style: Option<PyStyleSpec>,
        metadata: Option<PyPersonaMetadata>,
    ) -> Self {
        Self {
            inner: Persona {
                identity,
                salient_traits: salient_traits.into_iter().map(|t| t.inner).collect(),
                style: style.map(|s| s.inner).unwrap_or_default(),
                metadata: metadata.map(|m| m.inner).unwrap_or_default(),
            },
        }
    }

    #[getter]
    fn identity(&self) -> &str {
        &self.inner.identity
    }

    #[getter]
    fn salient_traits(&self) -> Vec<PyTraitFragment> {
        self.inner
            .salient_traits
            .iter()
            .cloned()
            .map(|t| PyTraitFragment { inner: t })
            .collect()
    }

    #[getter]
    fn style(&self) -> PyStyleSpec {
        PyStyleSpec {
            inner: self.inner.style.clone(),
        }
    }

    #[getter]
    fn metadata(&self) -> PyPersonaMetadata {
        PyPersonaMetadata {
            inner: self.inner.metadata.clone(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "PersonaValue(identity={:?}, traits={})",
            self.inner.identity,
            self.inner.salient_traits.len()
        )
    }
}

// ----- RenderedPersona -----------------------------------------------------

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

    #[getter]
    fn salient_traits(&self) -> Vec<PyTraitFragment> {
        self.inner
            .salient_traits
            .iter()
            .cloned()
            .map(|t| PyTraitFragment { inner: t })
            .collect()
    }

    #[getter]
    fn style(&self) -> PyStyleSpec {
        PyStyleSpec {
            inner: self.inner.style.clone(),
        }
    }

    #[getter]
    fn metadata(&self) -> PyPersonaMetadata {
        PyPersonaMetadata {
            inner: self.inner.metadata.clone(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "RenderedPersona(identity={:?}, est_tokens={})",
            self.inner.identity, self.inner.estimated_tokens
        )
    }
}

// ----- PersonaSet ----------------------------------------------------------

#[pyclass(name = "PersonaSet", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyPersonaSet {
    pub(crate) inner: PersonaSet,
}

#[pymethods]
impl PyPersonaSet {
    /// `PersonaSet` construction from Python is not yet supported
    /// because `PersonaEntry` is not re-exported from the persona crate.
    /// Use a Rust-side factory and return `PyPersonaSet` instead.
    #[new]
    #[pyo3(signature = (id, version, entries=Vec::new()))]
    fn new(id: String, version: String, entries: Vec<(String, String, PyPersona)>) -> PyResult<Self> {
        let _: Version = parse_version(&version)?;
        let _ = (id, entries);
        Err(PyNotImplementedError::new_err(
            "PersonaSet construction not yet exposed: PersonaEntry is private in the persona crate",
        ))
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
    fn entries<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let list = PyList::empty_bound(py);
        for e in &self.inner.entries {
            let d = PyDict::new_bound(py);
            d.set_item("id", e.id.as_str())?;
            d.set_item("label", &e.label)?;
            d.set_item(
                "baseline",
                Py::new(
                    py,
                    PyPersona {
                        inner: e.baseline.clone(),
                    },
                )?,
            )?;
            list.append(d)?;
        }
        Ok(list)
    }

    fn __repr__(&self) -> String {
        format!(
            "PersonaSet(id={:?}, version={}, entries={})",
            self.inner.id,
            self.inner.version,
            self.inner.entries.len()
        )
    }
}

// ----- MBTI / Archetype / Cognitive types ----------------------------------

#[pyclass(name = "MbtiType", module = "atomr_agents._native.persona", eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyMbtiType {
    pub(crate) inner: MbtiType,
}

impl PyMbtiType {
    fn from_str(s: &str) -> PyResult<MbtiType> {
        Ok(match s.to_ascii_uppercase().as_str() {
            "INTJ" => MbtiType::INTJ,
            "INTP" => MbtiType::INTP,
            "ENTJ" => MbtiType::ENTJ,
            "ENTP" => MbtiType::ENTP,
            "INFJ" => MbtiType::INFJ,
            "INFP" => MbtiType::INFP,
            "ENFJ" => MbtiType::ENFJ,
            "ENFP" => MbtiType::ENFP,
            "ISTJ" => MbtiType::ISTJ,
            "ISFJ" => MbtiType::ISFJ,
            "ESTJ" => MbtiType::ESTJ,
            "ESFJ" => MbtiType::ESFJ,
            "ISTP" => MbtiType::ISTP,
            "ISFP" => MbtiType::ISFP,
            "ESTP" => MbtiType::ESTP,
            "ESFP" => MbtiType::ESFP,
            other => {
                return Err(PyValueError::new_err(format!(
                    "invalid MBTI type: {other:?} (expected one of the 16 four-letter codes)"
                )))
            }
        })
    }
}

fn mbti_to_str(t: MbtiType) -> &'static str {
    match t {
        MbtiType::INTJ => "INTJ",
        MbtiType::INTP => "INTP",
        MbtiType::ENTJ => "ENTJ",
        MbtiType::ENTP => "ENTP",
        MbtiType::INFJ => "INFJ",
        MbtiType::INFP => "INFP",
        MbtiType::ENFJ => "ENFJ",
        MbtiType::ENFP => "ENFP",
        MbtiType::ISTJ => "ISTJ",
        MbtiType::ISFJ => "ISFJ",
        MbtiType::ESTJ => "ESTJ",
        MbtiType::ESFJ => "ESFJ",
        MbtiType::ISTP => "ISTP",
        MbtiType::ISFP => "ISFP",
        MbtiType::ESTP => "ESTP",
        MbtiType::ESFP => "ESFP",
    }
}

#[pymethods]
impl PyMbtiType {
    #[new]
    fn new(code: &str) -> PyResult<Self> {
        Ok(Self {
            inner: Self::from_str(code)?,
        })
    }

    #[getter]
    fn code(&self) -> &'static str {
        mbti_to_str(self.inner)
    }

    fn __repr__(&self) -> String {
        format!("MbtiType({})", mbti_to_str(self.inner))
    }

    fn __str__(&self) -> &'static str {
        mbti_to_str(self.inner)
    }
}

#[pyclass(name = "CognitiveFunction", module = "atomr_agents._native.persona", eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyCognitiveFunction {
    pub(crate) inner: CognitiveFunction,
}

impl PyCognitiveFunction {
    fn from_str(s: &str) -> PyResult<CognitiveFunction> {
        Ok(match s {
            "Ni" => CognitiveFunction::Ni,
            "Ne" => CognitiveFunction::Ne,
            "Si" => CognitiveFunction::Si,
            "Se" => CognitiveFunction::Se,
            "Ti" => CognitiveFunction::Ti,
            "Te" => CognitiveFunction::Te,
            "Fi" => CognitiveFunction::Fi,
            "Fe" => CognitiveFunction::Fe,
            other => {
                return Err(PyValueError::new_err(format!(
                    "invalid cognitive function: {other:?} (expected Ni/Ne/Si/Se/Ti/Te/Fi/Fe)"
                )))
            }
        })
    }
}

fn cf_to_str(c: CognitiveFunction) -> &'static str {
    match c {
        CognitiveFunction::Ni => "Ni",
        CognitiveFunction::Ne => "Ne",
        CognitiveFunction::Si => "Si",
        CognitiveFunction::Se => "Se",
        CognitiveFunction::Ti => "Ti",
        CognitiveFunction::Te => "Te",
        CognitiveFunction::Fi => "Fi",
        CognitiveFunction::Fe => "Fe",
    }
}

#[pymethods]
impl PyCognitiveFunction {
    #[new]
    fn new(code: &str) -> PyResult<Self> {
        Ok(Self {
            inner: Self::from_str(code)?,
        })
    }

    #[getter]
    fn code(&self) -> &'static str {
        cf_to_str(self.inner)
    }

    fn __repr__(&self) -> String {
        format!("CognitiveFunction({})", cf_to_str(self.inner))
    }

    fn __str__(&self) -> &'static str {
        cf_to_str(self.inner)
    }
}

#[pyclass(name = "CognitiveStack", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyCognitiveStack {
    pub(crate) inner: CognitiveStack,
}

#[pymethods]
impl PyCognitiveStack {
    #[new]
    fn new(
        dominant: PyCognitiveFunction,
        auxiliary: PyCognitiveFunction,
        tertiary: PyCognitiveFunction,
        inferior: PyCognitiveFunction,
    ) -> Self {
        Self {
            inner: CognitiveStack {
                dominant: dominant.inner,
                auxiliary: auxiliary.inner,
                tertiary: tertiary.inner,
                inferior: inferior.inner,
            },
        }
    }

    #[getter]
    fn dominant(&self) -> PyCognitiveFunction {
        PyCognitiveFunction {
            inner: self.inner.dominant,
        }
    }

    #[getter]
    fn auxiliary(&self) -> PyCognitiveFunction {
        PyCognitiveFunction {
            inner: self.inner.auxiliary,
        }
    }

    #[getter]
    fn tertiary(&self) -> PyCognitiveFunction {
        PyCognitiveFunction {
            inner: self.inner.tertiary,
        }
    }

    #[getter]
    fn inferior(&self) -> PyCognitiveFunction {
        PyCognitiveFunction {
            inner: self.inner.inferior,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "CognitiveStack(dom={}, aux={}, tert={}, inf={})",
            cf_to_str(self.inner.dominant),
            cf_to_str(self.inner.auxiliary),
            cf_to_str(self.inner.tertiary),
            cf_to_str(self.inner.inferior),
        )
    }
}

#[pyclass(name = "Archetype", module = "atomr_agents._native.persona", eq)]
#[derive(Clone, PartialEq, Eq)]
pub struct PyArchetype {
    pub(crate) inner: Archetype,
}

impl PyArchetype {
    fn from_str(s: &str) -> PyResult<Archetype> {
        Ok(match s {
            "Sage" => Archetype::Sage,
            "Caregiver" => Archetype::Caregiver,
            "Explorer" => Archetype::Explorer,
            "Hero" => Archetype::Hero,
            "Magician" => Archetype::Magician,
            "Outlaw" => Archetype::Outlaw,
            "Lover" => Archetype::Lover,
            "Jester" => Archetype::Jester,
            "Everyman" => Archetype::Everyman,
            "Innocent" => Archetype::Innocent,
            "Ruler" => Archetype::Ruler,
            "Creator" => Archetype::Creator,
            other => {
                return Err(PyValueError::new_err(format!(
                    "invalid archetype: {other:?} (expected Sage/Caregiver/Explorer/Hero/Magician/Outlaw/Lover/Jester/Everyman/Innocent/Ruler/Creator)"
                )))
            }
        })
    }
}

fn archetype_to_str(a: Archetype) -> &'static str {
    match a {
        Archetype::Sage => "Sage",
        Archetype::Caregiver => "Caregiver",
        Archetype::Explorer => "Explorer",
        Archetype::Hero => "Hero",
        Archetype::Magician => "Magician",
        Archetype::Outlaw => "Outlaw",
        Archetype::Lover => "Lover",
        Archetype::Jester => "Jester",
        Archetype::Everyman => "Everyman",
        Archetype::Innocent => "Innocent",
        Archetype::Ruler => "Ruler",
        Archetype::Creator => "Creator",
    }
}

#[pymethods]
impl PyArchetype {
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        Ok(Self {
            inner: Self::from_str(name)?,
        })
    }

    #[getter]
    fn name(&self) -> &'static str {
        archetype_to_str(self.inner)
    }

    fn __repr__(&self) -> String {
        format!("Archetype({})", archetype_to_str(self.inner))
    }

    fn __str__(&self) -> &'static str {
        archetype_to_str(self.inner)
    }
}

// ----- PersonaStrategy dyn handle ------------------------------------------

#[pyclass(name = "Persona", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyPersonaStrategy {
    pub(crate) inner: Arc<dyn PersonaStrategy>,
}

#[pymethods]
impl PyPersonaStrategy {
    fn __repr__(&self) -> String {
        "Persona(strategy=<dyn>)".into()
    }
}

// ----- PersonaEmphasisStrategy dyn handle ----------------------------------

#[pyclass(name = "PersonaEmphasis", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyEmphasisStrategy {
    pub(crate) inner: Arc<dyn PersonaEmphasisStrategy>,
}

#[pymethods]
impl PyEmphasisStrategy {
    fn __repr__(&self) -> String {
        "PersonaEmphasis(strategy=<dyn>)".into()
    }
}

// ----- PersonaReconciler dyn handle ----------------------------------------

#[pyclass(name = "PersonaReconciler", module = "atomr_agents._native.persona")]
#[derive(Clone)]
pub struct PyPersonaReconciler {
    pub(crate) inner: Arc<dyn PersonaReconciler>,
}

#[pymethods]
impl PyPersonaReconciler {
    fn __repr__(&self) -> String {
        "PersonaReconciler(<dyn>)".into()
    }
}

// ----- TraitRenderer (placeholder concrete handle) -------------------------
//
// The Rust trait `TraitRenderer` only consumes `BigFiveScores` and is
// internal to `BigFivePersonaStrategy`. Until a public surface emerges,
// we expose a tag-only dyn handle so callers can pass a default
// renderer through PyO3 without crossing the GIL on every render.

#[pyclass(name = "TraitRenderer", module = "atomr_agents._native.persona")]
#[derive(Clone, Default)]
pub struct PyTraitRenderer;

#[pymethods]
impl PyTraitRenderer {
    #[new]
    fn new() -> Self {
        Self
    }

    fn __repr__(&self) -> String {
        "TraitRenderer(default)".into()
    }
}

// ----- Python guest adapter for PersonaStrategy ----------------------------

pub(crate) struct PyPersonaStrategyAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl PersonaStrategy for PyPersonaStrategyAdapter {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<RenderedPersona> {
        let target = self.target.clone();
        let ctx_owned = ctx.clone();
        let budget_remaining = budget.remaining;
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let ctx_dict = agent_context_to_pydict(py, &ctx_owned)?;
            let bud = PyDict::new_bound(py);
            bud.set_item("remaining", budget_remaining)?;
            let instance: Bound<'_, PyAny> = if bound.hasattr("resolve")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("resolve")?.call1((ctx_dict, bud))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("persona strategy resolve: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        // Two acceptable return shapes:
        //   1. a `RenderedPersona` PyO3 instance (recommended);
        //   2. a dict with at least an `identity` field.
        let rendered = Python::with_gil(|py| -> PyResult<RenderedPersona> {
            let bound = final_val.bind(py);
            if let Ok(r) = bound.extract::<PyRenderedPersona>() {
                return Ok(r.inner);
            }
            // Fall back to dict-shaped payload.
            let v = py_to_json(py, bound)?;
            let obj = v.as_object().ok_or_else(|| {
                PyValueError::new_err("persona strategy must return RenderedPersona or dict")
            })?;
            let identity = obj
                .get("identity")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let estimated_tokens = obj.get("estimated_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Ok(RenderedPersona {
                identity,
                estimated_tokens,
                ..Default::default()
            })
        })
        .map_err(|e| AgentError::Internal(format!("persona strategy result: {e}")))?;

        // Reflect token usage in the parent budget so composite layers
        // see a consistent picture.
        budget
            .consume(rendered.estimated_tokens.min(budget.remaining))
            .ok();
        Ok(rendered)
    }
}

// ----- Factory functions: PersonaStrategy ----------------------------------

#[pyfunction]
fn static_persona_strategy(identity: String) -> PyPersonaStrategy {
    PyPersonaStrategy {
        inner: Arc::new(StaticPersonaStrategy::new(identity)),
    }
}

/// Construct an MBTI strategy.
///
/// Scaffold: `MbtiPersonaStrategy::new` requires `ExpressionLevel`,
/// which is private to the persona crate. The factory validates the
/// MBTI type input and returns `NotImplementedError` until the crate
/// re-exports `ExpressionLevel`.
#[pyfunction]
#[pyo3(signature = (mbti_type, stack=None, expression=None))]
fn mbti_persona_strategy(
    mbti_type: PyMbtiType,
    stack: Option<PyCognitiveStack>,
    expression: Option<String>,
) -> PyResult<PyPersonaStrategy> {
    let _ = mbti_type;
    let _ = stack;
    let _ = expression;
    Err(PyNotImplementedError::new_err(
        "mbti_persona_strategy not yet wired: ExpressionLevel is private in the persona crate",
    ))
}

/// Construct a Jungian-archetype persona strategy. Optional
/// `shadow` (an `Archetype` code string) refines the primary
/// archetype with a shadow facet. `individuation` (0.0..1.0)
/// controls maturity expression; ignored unless the crate exposes
/// setters in a later phase.
#[pyfunction]
#[pyo3(signature = (archetype, shadow=None, individuation=None))]
fn jungian_archetype_strategy(
    archetype: PyArchetype,
    shadow: Option<PyArchetype>,
    individuation: Option<f32>,
) -> PyPersonaStrategy {
    let mut s = JungianArchetypeStrategy::new(archetype.inner);
    if let Some(sh) = shadow {
        s = s.with_shadow(sh.inner);
    }
    // `individuation` is `pub` on the struct, so we can set it post-hoc.
    if let Some(v) = individuation {
        s.individuation = v;
    }
    PyPersonaStrategy { inner: Arc::new(s) }
}

/// Construct a Big Five persona strategy from a dict of scores.
/// Keys: `openness`, `conscientiousness`, `extraversion`,
/// `agreeableness`, `neuroticism`. Missing keys default to 0.5.
///
/// Currently a scaffold: `BigFiveScores` is private to the persona
/// crate. The factory validates input shape and returns
/// `NotImplementedError` until the crate re-exports the score struct.
#[pyfunction]
fn big_five_persona_strategy(scores: Bound<'_, PyDict>) -> PyResult<PyPersonaStrategy> {
    fn extract(d: &Bound<'_, PyDict>, key: &str) -> PyResult<f32> {
        match d.get_item(key)? {
            Some(v) if !v.is_none() => v.extract::<f32>(),
            _ => Ok(0.5),
        }
    }
    let _o = extract(&scores, "openness")?;
    let _c = extract(&scores, "conscientiousness")?;
    let _e = extract(&scores, "extraversion")?;
    let _a = extract(&scores, "agreeableness")?;
    let _n = extract(&scores, "neuroticism")?;
    Err(PyNotImplementedError::new_err(
        "big_five_persona_strategy not yet wired: BigFiveScores is private in the persona crate",
    ))
}

/// Combine multiple persona strategies. Each entry is
/// `(strategy, weight)`. If `reconciler_key` is provided, a
/// Python-registered reconciler is looked up via
/// `register_persona_reconciler_factory`; otherwise a
/// `WeightedAverageReconciler` is used.
#[pyfunction]
#[pyo3(signature = (layers, reconciler_key=None))]
fn composite_persona_strategy(
    layers: Vec<(PyPersonaStrategy, f32)>,
    reconciler_key: Option<String>,
) -> PyResult<PyPersonaStrategy> {
    // Wrap each Arc<dyn PersonaStrategy> as a Boxed forwarder so the
    // composite owns its layers (the crate's API takes Box, not Arc).
    let boxed: Vec<(Box<dyn PersonaStrategy>, f32)> = layers
        .into_iter()
        .map(|(h, w)| {
            let arc = h.inner;
            let fwd: Box<dyn PersonaStrategy> = Box::new(ArcForwarder { inner: arc });
            (fwd, w)
        })
        .collect();

    let composite = if let Some(key) = reconciler_key {
        let target = crate::guest::must_lookup("persona_reconciler", &key)?;
        let reconciler: Box<dyn PersonaReconciler> = Box::new(PyPersonaReconcilerAdapter { target });
        CompositePersonaStrategy::new(boxed, reconciler)
    } else {
        CompositePersonaStrategy::weighted_average(boxed)
    };

    Ok(PyPersonaStrategy {
        inner: Arc::new(composite),
    })
}

/// Look up a Python-registered persona factory and wrap it as a
/// `PersonaStrategy` handle. Pair with
/// `guest.register_persona_factory(key, target)`.
#[pyfunction]
fn persona_strategy_from_factory(key: String) -> PyResult<PyPersonaStrategy> {
    let target = crate::guest::must_lookup("persona", &key)?;
    Ok(PyPersonaStrategy {
        inner: Arc::new(PyPersonaStrategyAdapter { target }),
    })
}

// Forwards Arc<dyn PersonaStrategy> as a Boxed PersonaStrategy so the
// composite (which owns Box) can hold cloned handles.
struct ArcForwarder {
    inner: Arc<dyn PersonaStrategy>,
}

#[async_trait]
impl PersonaStrategy for ArcForwarder {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> AgentResult<RenderedPersona> {
        self.inner.resolve(ctx, budget).await
    }
}

// ----- PyPersonaReconcilerAdapter ------------------------------------------

pub(crate) struct PyPersonaReconcilerAdapter {
    target: Arc<PyObject>,
}

impl PersonaReconciler for PyPersonaReconcilerAdapter {
    fn reconcile(&self, layers: Vec<(RenderedPersona, f32)>) -> Persona {
        // Convert each (rendered, weight) into a Python tuple and
        // ask the target to reconcile. If the target raises, fall
        // back to an empty Persona.
        let result = Python::with_gil(|py| -> PyResult<Persona> {
            let bound = self.target.bind(py);
            let py_layers = PyList::empty_bound(py);
            for (r, w) in &layers {
                let pr = Py::new(py, PyRenderedPersona { inner: r.clone() })?;
                py_layers.append((pr, *w))?;
            }
            let instance: Bound<'_, PyAny> = if bound.hasattr("reconcile")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("reconcile")?.call1((py_layers,))?;
            // Accept either a PyPersona or a dict {identity, ...}
            if let Ok(p) = r.extract::<PyPersona>() {
                return Ok(p.inner);
            }
            let v = py_to_json(py, &r)?;
            let obj = v.as_object().ok_or_else(|| {
                PyValueError::new_err("persona reconciler must return PersonaValue or dict")
            })?;
            let identity = obj
                .get("identity")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(Persona {
                identity,
                ..Default::default()
            })
        });
        result.unwrap_or_else(|_| Persona::default())
    }
}

// ----- Emphasis strategies -------------------------------------------------

/// `StaticEmphasis` — preserve the persona unchanged each turn.
#[pyfunction]
fn static_emphasis() -> PyEmphasisStrategy {
    PyEmphasisStrategy {
        inner: Arc::new(StaticEmphasis),
    }
}

/// `TaskAdaptive` — heuristic mode tagging based on user prompt keywords.
#[pyfunction]
fn task_adaptive() -> PyEmphasisStrategy {
    PyEmphasisStrategy {
        inner: Arc::new(TaskAdaptive),
    }
}

/// `AudienceAdaptive` — emphasis switches between "expert" and
/// "newcomer" based on history length.
#[pyfunction]
fn audience_adaptive() -> PyEmphasisStrategy {
    PyEmphasisStrategy {
        inner: Arc::new(AudienceAdaptive),
    }
}

/// `GoalConditioned(current_goal)` — annotates identity with a goal
/// label set by the harness.
#[pyfunction]
fn goal_conditioned(current_goal: String) -> PyEmphasisStrategy {
    PyEmphasisStrategy {
        inner: Arc::new(GoalConditioned { current_goal }),
    }
}

/// `MoodState(pressure)` — mood word inferred from pressure (0.0..1.0).
#[pyfunction]
#[pyo3(signature = (pressure=0.0))]
fn mood_state(pressure: f32) -> PyEmphasisStrategy {
    PyEmphasisStrategy {
        inner: Arc::new(MoodState { pressure }),
    }
}

// ----- Persona reconciler factory ------------------------------------------

#[pyfunction]
fn persona_reconciler_from_factory(key: String) -> PyResult<PyPersonaReconciler> {
    let target = crate::guest::must_lookup("persona_reconciler", &key)?;
    Ok(PyPersonaReconciler {
        inner: Arc::new(PyPersonaReconcilerAdapter { target }),
    })
}

// ----- Module registration -------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "persona")?;
    // Value types.
    m.add_class::<PyStyleSpec>()?;
    m.add_class::<PyPersonaMetadata>()?;
    m.add_class::<PyTraitFragment>()?;
    m.add_class::<PyPersona>()?;
    m.add_class::<PyRenderedPersona>()?;
    m.add_class::<PyPersonaSet>()?;
    // Typology enums.
    m.add_class::<PyMbtiType>()?;
    m.add_class::<PyCognitiveFunction>()?;
    m.add_class::<PyCognitiveStack>()?;
    m.add_class::<PyArchetype>()?;
    m.add_class::<PyTraitRenderer>()?;
    // Strategy handles.
    m.add_class::<PyPersonaStrategy>()?;
    m.add_class::<PyEmphasisStrategy>()?;
    m.add_class::<PyPersonaReconciler>()?;
    // PersonaStrategy factories.
    m.add_function(wrap_pyfunction!(static_persona_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(mbti_persona_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(jungian_archetype_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(big_five_persona_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(composite_persona_strategy, &m)?)?;
    m.add_function(wrap_pyfunction!(persona_strategy_from_factory, &m)?)?;
    // Emphasis factories.
    m.add_function(wrap_pyfunction!(static_emphasis, &m)?)?;
    m.add_function(wrap_pyfunction!(task_adaptive, &m)?)?;
    m.add_function(wrap_pyfunction!(audience_adaptive, &m)?)?;
    m.add_function(wrap_pyfunction!(goal_conditioned, &m)?)?;
    m.add_function(wrap_pyfunction!(mood_state, &m)?)?;
    // Reconciler factory.
    m.add_function(wrap_pyfunction!(persona_reconciler_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
