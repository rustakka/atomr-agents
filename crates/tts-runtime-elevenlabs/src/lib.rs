//! ElevenLabs TTS backend.
//!
//! Implements [`atomr_agents_tts_core::TextToSpeech`] for batch
//! REST `/v1/text-to-speech/{voice_id}`, streaming WS
//! `/v1/text-to-speech/{voice_id}/stream-input`, sound-effect
//! `/v1/sound-generation`, and voice-cloning `/v1/voices/add`.
//!
//! Conversational AI realtime
//! (`wss://api.elevenlabs.io/v1/convai/conversation`) is exposed
//! via `open_realtime` — covers the bidirectional surface for
//! voice agents.

mod caps;
mod config;
mod runner;
mod stream;

pub use caps::CAPS;
pub use config::ElevenLabsConfig;
pub use runner::ElevenLabsRunner;
