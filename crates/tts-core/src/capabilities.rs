//! Rich capability metadata advertised by every TTS backend.
//!
//! Backends export a `pub const CAPS: Capabilities = Capabilities { .. }`
//! and return `&CAPS` from [`crate::TextToSpeech::capabilities`]. The
//! struct is `Serialize`-only because of the `&'static` slice fields
//! (matches `stt_core::Capabilities`).
//!
//! Five MOSS-TTS surfaces are first-class: [`Capabilities::plain_tts`],
//! [`Capabilities::voicegen_from_text`],
//! [`Capabilities::voice_cloning`],
//! [`Capabilities::dialogue_multispeaker`],
//! [`Capabilities::sound_effects`],
//! [`Capabilities::realtime_bidirectional`].

use atomr_agents_stt_core::{AudioFormat, Languages};
use serde::Serialize;

/// `Serialize`-only by design. Capabilities flow outward (to JSON /
/// Python / telemetry) and are never round-tripped back into Rust.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    // ----- Five MOSS-TTS surfaces, mapped uniformly across backends -----
    /// Plain text → speech.
    pub plain_tts: bool,
    /// Voice synthesised from a free-text description (no reference clip).
    /// MOSS-VoiceGenerator; ElevenLabs Voice Lab text-to-voice.
    pub voicegen_from_text: bool,
    /// Voice cloning from a reference audio clip.
    pub voice_cloning: VoiceCloningSupport,
    /// Multi-speaker scripted dialogue. `Some(n)` = max speakers.
    pub dialogue_multispeaker: Option<u8>,
    /// Non-speech audio generation (foley, ambient, music-style SFX).
    pub sound_effects: bool,
    /// Bidirectional realtime session (audio + text both ways).
    pub realtime_bidirectional: bool,

    // ----- Streaming + library --------------------------------------------
    /// Backend can emit audio chunks as it generates.
    pub streaming_output: bool,
    pub voice_library: VoiceCatalog,
    pub max_concurrent_streams: Option<u16>,

    // ----- Quality / control surface --------------------------------------
    pub languages: Languages,
    /// Emotional / expressive control (style, intensity).
    pub style_control: bool,
    /// SSML markup support.
    pub ssml: bool,
    /// Pitch / rate / volume control.
    pub prosody_control: bool,
    /// Per-word timing in the output.
    pub word_timestamps: bool,

    // ----- Operational ----------------------------------------------------
    pub max_chars_per_request: Option<u32>,
    /// For local backends: typical RTF on a reference machine.
    pub real_time_factor: Option<f32>,
    /// Time-to-first-byte for streaming output.
    pub typical_ttfb_ms: Option<u16>,
    pub requires_network: bool,
    /// Output container formats the backend can emit.
    pub supported_output_formats: &'static [AudioFormat],
    /// Streaming output emits intermediate chunks before completion.
    pub partial_results: bool,
    pub cost_per_1k_chars_usd: Option<f32>,
    /// For SFX / dialogue / realtime where billing is duration-based.
    pub cost_per_audio_min_usd: Option<f32>,
}

impl Capabilities {
    /// All-false / `None` baseline that backends spread-update from.
    /// `Capabilities { plain_tts: true, .. Capabilities::ZERO }`.
    pub const ZERO: Self = Self {
        plain_tts: false,
        voicegen_from_text: false,
        voice_cloning: VoiceCloningSupport::None,
        dialogue_multispeaker: None,
        sound_effects: false,
        realtime_bidirectional: false,
        streaming_output: false,
        voice_library: VoiceCatalog::None,
        max_concurrent_streams: None,
        languages: Languages::All,
        style_control: false,
        ssml: false,
        prosody_control: false,
        word_timestamps: false,
        max_chars_per_request: None,
        real_time_factor: None,
        typical_ttfb_ms: None,
        requires_network: true,
        supported_output_formats: &[],
        partial_results: false,
        cost_per_1k_chars_usd: None,
        cost_per_audio_min_usd: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceCloningSupport {
    None,
    /// Single-shot clone from a reference clip of at least
    /// `min_sample_secs` seconds.
    ZeroShot { min_sample_secs: f32 },
    /// Requires a finetune step (slower, longer enrollment).
    Finetune,
    /// Backend supports both modes.
    Both { min_sample_secs: f32 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceCatalog {
    /// Backend exposes no preset voices (e.g. cloning-only).
    None,
    /// A fixed set known at compile time.
    Static { voices: &'static [VoiceDescriptor] },
    /// Backend has a `/voices` endpoint for dynamic discovery.
    Dynamic,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct VoiceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub language: &'static str,
    pub gender: Gender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    Female,
    Male,
    Neutral,
    Unspecified,
}
