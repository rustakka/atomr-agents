//! Persona: a separately-strategizable axis from "what the agent does".

mod binding;
mod emphasis;
mod structural;
mod r#trait;

pub use binding::{bind_persona_to_role, ClearanceMismatch};
pub use emphasis::{
    AudienceAdaptive, GoalConditioned, MoodState, PersonaEmphasisStrategy, StaticEmphasis, TaskAdaptive,
};
pub use r#trait::{
    Persona, PersonaMetadata, PersonaSet, PersonaStrategy, RenderedPersona, StyleSpec, TraitFragment,
};
pub use structural::{
    Archetype, BigFivePersonaStrategy, CognitiveFunction, CognitiveStack, CompositePersonaStrategy,
    JungianArchetypeStrategy, MbtiPersonaStrategy, MbtiType, PersonaReconciler, StaticPersonaStrategy,
    TraitRenderer,
};
