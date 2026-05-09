//! Rich capability metadata advertised by every backend.
//!
//! Backends export a `pub const CAPS: Capabilities = Capabilities { .. }`
//! and return `&CAPS` from [`crate::SpeechToText::capabilities`]. The
//! struct is `serde`-derived so it round-trips to Python as a dict
//! and JSON for telemetry / registry artifacts.

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::audio::AudioFormat;

/// `Serialize`-only by design: the slice fields (`languages` and
/// `supported_audio_formats`) are `&'static`, which `serde` can't
/// deserialize into. Capabilities flow outward (to JSON / Python /
/// telemetry) and are never round-tripped back into Rust.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    /// Single-shot file/buffer transcription.
    pub batch: bool,
    /// Caller pushes audio chunks; transcript events stream back.
    pub streaming_push: bool,
    /// Backend can sustain a continuous live microphone feed.
    pub realtime_microphone: bool,
    /// Speaker diarization support (none / count-only / named).
    pub diarization: DiarizationSupport,
    /// Per-word timing in segments.
    pub word_timestamps: bool,
    /// Per-utterance/segment timing (almost always true if any timing).
    pub utterance_timestamps: bool,
    /// Backend autodetects the spoken language.
    pub language_detection: bool,
    /// Languages the backend is willing to transcribe.
    pub languages: Languages,
    pub punctuation: bool,
    pub profanity_filter: bool,
    /// Hard upper bound on a single batch call (whisper-1 = 25 min).
    pub max_audio_secs: Option<u32>,
    pub max_concurrent_streams: Option<u16>,
    /// For local backends: typical RTF on a reference machine (CPU).
    /// `None` for cloud backends.
    pub real_time_factor: Option<f32>,
    pub requires_network: bool,
    /// Audio formats the backend will accept directly.
    pub supported_audio_formats: &'static [AudioFormat],
    /// Streaming-only: minimum chunk size to push.
    pub min_chunk_ms: Option<u32>,
    /// Backend emits partial (non-final) transcripts during streaming.
    pub partial_results: bool,
    /// Server-side PII redaction.
    pub redaction: bool,
    /// Backend signals end-of-utterance via VAD on the wire.
    pub vad_endpointing: bool,
    /// Backend supports a custom vocabulary / keyword boost list.
    pub custom_vocabulary: bool,
    /// Approximate USD cost per minute of input audio.
    pub cost_per_audio_min_usd: Option<f32>,
}

impl Capabilities {
    /// All-false / `None` baseline that backends spread-update from.
    /// Use as `Capabilities { batch: true, .. Capabilities::ZERO }`.
    pub const ZERO: Self = Self {
        batch: false,
        streaming_push: false,
        realtime_microphone: false,
        diarization: DiarizationSupport::None,
        word_timestamps: false,
        utterance_timestamps: false,
        language_detection: false,
        languages: Languages::All,
        punctuation: false,
        profanity_filter: false,
        max_audio_secs: None,
        max_concurrent_streams: None,
        real_time_factor: None,
        requires_network: true,
        supported_audio_formats: &[],
        min_chunk_ms: None,
        partial_results: false,
        redaction: false,
        vad_endpointing: false,
        custom_vocabulary: false,
        cost_per_audio_min_usd: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiarizationSupport {
    /// No diarization. Caller can layer
    /// `atomr-agents-stt-diarize-sherpa` on top.
    None,
    /// Backend assigns numeric speaker IDs but does not name them.
    SpeakerCount,
    /// Backend can be primed with named speakers (e.g. enrollment).
    NamedSpeakers,
}

#[derive(Debug, Clone)]
pub enum Languages {
    /// Backend handles any language without an enrollment list.
    All,
    /// Restricted set of BCP-47 codes.
    Subset(&'static [&'static str]),
}

impl Languages {
    pub fn supports(&self, bcp47: &str) -> bool {
        match self {
            Languages::All => true,
            Languages::Subset(list) => list.iter().any(|l| l.eq_ignore_ascii_case(bcp47)),
        }
    }
}

// Custom serialize: serde's internally-tagged tuple variants don't
// accept sequence payloads, so we emit `{kind: "all"}` or
// `{kind: "subset", codes: [...]}` by hand.
impl Serialize for Languages {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            Languages::All => {
                let mut m = ser.serialize_map(Some(1))?;
                m.serialize_entry("kind", "all")?;
                m.end()
            }
            Languages::Subset(codes) => {
                let mut m = ser.serialize_map(Some(2))?;
                m.serialize_entry("kind", "subset")?;
                m.serialize_entry("codes", codes)?;
                m.end()
            }
        }
    }
}
