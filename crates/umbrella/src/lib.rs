//! atomr-agents — composable agentic framework on top of atomr.
//!
//! This umbrella crate re-exports each subsystem behind a feature
//! flag, mirroring the convention used by the `atomr` umbrella.

pub use atomr_agents_callable as callable;
pub use atomr_agents_context as context;
pub use atomr_agents_core as core;
pub use atomr_agents_observability as observability;
pub use atomr_agents_strategy as strategy;

#[cfg(feature = "agent")]
pub use atomr_agents_agent as agent;
#[cfg(feature = "embed")]
pub use atomr_agents_embed as embed;
#[cfg(feature = "eval")]
pub use atomr_agents_eval as eval;
#[cfg(feature = "harness")]
pub use atomr_agents_harness as harness;
#[cfg(feature = "instruction")]
pub use atomr_agents_instruction as instruction;
#[cfg(feature = "memory")]
pub use atomr_agents_memory as memory;
#[cfg(feature = "org")]
pub use atomr_agents_org as org;
#[cfg(feature = "persona")]
pub use atomr_agents_persona as persona;
#[cfg(feature = "registry")]
pub use atomr_agents_registry as registry;
#[cfg(feature = "skill")]
pub use atomr_agents_skill as skill;
#[cfg(feature = "testkit")]
pub use atomr_agents_testkit as testkit;
#[cfg(feature = "tool")]
pub use atomr_agents_tool as tool;
#[cfg(feature = "workflow")]
pub use atomr_agents_workflow as workflow;

/// Speech-to-text capability. Pulls in the core trait + types
/// (`atomr_agents_stt_core`), the audio I/O helpers
/// (`atomr_agents_stt_audio`), the tool/skill adapters
/// (`atomr_agents_stt_tool`), and any backends / voice-session
/// modules enabled via `stt-{openai,deepgram,assemblyai,whisper,
/// diarize,voice,mic}` features.
#[cfg(feature = "stt")]
pub mod stt {
    pub use atomr_agents_stt_audio as audio;
    pub use atomr_agents_stt_core::*;
    pub use atomr_agents_stt_tool as tool;

    #[cfg(feature = "stt-openai")]
    pub use atomr_agents_stt_runtime_openai as openai;
    #[cfg(feature = "stt-deepgram")]
    pub use atomr_agents_stt_runtime_deepgram as deepgram;
    #[cfg(feature = "stt-assemblyai")]
    pub use atomr_agents_stt_runtime_assemblyai as assemblyai;
    #[cfg(feature = "stt-whisper")]
    pub use atomr_agents_stt_runtime_whisper as whisper;
    #[cfg(feature = "stt-diarize")]
    pub use atomr_agents_stt_diarize_sherpa as diarize;
    #[cfg(feature = "stt-voice")]
    pub use atomr_agents_stt_voice as voice;
}
