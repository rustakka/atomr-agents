use async_trait::async_trait;
use atomr_agents_core::{AgentContext, PersonaId, Result, TokenBudget};
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
