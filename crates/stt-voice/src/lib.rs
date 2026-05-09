//! Higher-level voice-session abstraction layered on top of
//! [`atomr_agents_stt_core::StreamingSession`].
//!
//! This is the "live interactive vs turn-based" surface — given a
//! streaming STT backend and a microphone (or any byte source), it
//! coalesces partial / final transcripts into discrete user turns
//! when [`VoiceMode::TurnBased`] is selected, or passes everything
//! through in [`VoiceMode::Live`] mode.
//!
//! Pieces:
//!
//! - [`Vad`] trait + [`EnergyVad`] (always) and the optional
//!   `SileroVad` (`vad-silero` feature) for endpoint detection.
//! - [`VoiceMode`] enum: `Live` or `TurnBased { silence_ms }`.
//! - [`VoiceSession`] — the orchestrator. Pulls events from a
//!   `StreamingSession`, runs VAD over the same audio, and emits
//!   [`VoiceEvent`]s.
//! - [`pump_mic_to_stream`] (`mic` feature) — convenience that
//!   wires `MicCaptureSession` → `StreamingSession::push_audio`.

mod session;
mod vad;
#[cfg(feature = "mic")]
mod pump;

pub use session::{VoiceEvent, VoiceMode, VoiceSession};
pub use vad::{EnergyVad, Vad};

#[cfg(feature = "vad-silero")]
pub use vad::SileroVad;

#[cfg(feature = "mic")]
pub use pump::pump_mic_to_stream;
