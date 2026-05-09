//! Adapters that expose speech-to-text capability inside the
//! atomr-agents framework as a [`Tool`] (callable by the model) and
//! as a [`Skill`] (declarative bundle that opts an agent into voice
//! input).

mod skill;
mod tool;

pub use skill::voice_input_skill;
pub use tool::TranscribeTool;
