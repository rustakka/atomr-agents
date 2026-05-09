//! Tool + Skill adapters that expose atomr-agents text-to-speech
//! inside the agent framework.
//!
//! - [`SpeakTool`] — a [`atomr_agents_tool::Tool`] the model can
//!   invoke with `{"text": "...", "voice": "..."}` to synthesise
//!   speech and write a WAV file. Returns the path + duration.
//! - [`voice_response_skill`] — bundles [`SpeakTool`] with a
//!   companion `transcribe_audio` tool (from `stt-tool`) into a
//!   single Skill that opts an agent into voice-driven I/O.

mod skill;
mod tool;

pub use skill::{voice_response_skill, voice_speak_skill};
pub use tool::SpeakTool;
