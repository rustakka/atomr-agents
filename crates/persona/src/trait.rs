use async_trait::async_trait;
use atomr_agents_core::{AgentContext, PersonaId, Result, TokenBudget};
use atomr_agents_security::{ClearanceLevel, Compartment};
use semver::Version;
use serde::{Deserialize, Serialize};

/// Describes the persona that emerges from any structural strategy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Persona {
    pub identity: String,
    pub salient_traits: Vec<TraitFragment>,
    pub style: StyleSpec,
    pub metadata: PersonaMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraitFragment {
    pub label: String,
    pub weight: f32,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StyleSpec {
    pub tone: Option<String>,
    pub register: Option<String>,
    pub verbosity: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaMetadata {
    pub framework: Option<String>,

    /// Minimum clearance level the persona is declared to operate at.
    ///
    /// `None` means "no explicit declaration" (treated as
    /// [`ClearanceLevel::Public`] when validated against a Role's
    /// `ClearanceContext`). Declared at compile/host time and validated
    /// fail-closed by [`crate::bind_persona_to_role`] — an MNPI
    /// information-wall hazard if persona and Role disagree. Optional +
    /// `#[serde(default)]` so existing personas keep deserializing.
    #[serde(default)]
    pub clearance: Option<ClearanceLevel>,

    /// Compartments (need-to-know walls) the persona declares it must
    /// access. Validated against the Role's granted compartments at
    /// host composition time; a compartment the Role does not grant is
    /// a fail-closed [`crate::ClearanceMismatch`].
    #[serde(default)]
    pub compartments: Vec<Compartment>,
}

/// What the strategy returns each turn (after emphasis).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderedPersona {
    pub identity: String,
    pub salient_traits: Vec<TraitFragment>,
    pub style: StyleSpec,
    pub metadata: PersonaMetadata,
    pub estimated_tokens: u32,
}

#[async_trait]
pub trait PersonaStrategy: Send + Sync + 'static {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget) -> Result<RenderedPersona>;
}

/// Versioned bundle of personas. Orgs publish a `PersonaSet`; teams
/// grant from it; agents instantiate one persona at a time.
#[derive(Clone)]
pub struct PersonaSet {
    pub id: String,
    pub version: Version,
    pub entries: Vec<PersonaEntry>,
}

#[derive(Clone)]
pub struct PersonaEntry {
    pub id: PersonaId,
    pub label: String,
    pub baseline: Persona,
}
