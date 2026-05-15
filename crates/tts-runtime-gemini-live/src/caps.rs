use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor};

pub const GEMINI_LIVE_VOICES: &[VoiceDescriptor] = &[
    VoiceDescriptor {
        id: "Puck",
        name: "Puck",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "Charon",
        name: "Charon",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "Kore",
        name: "Kore",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "Fenrir",
        name: "Fenrir",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "Aoede",
        name: "Aoede",
        language: "en-us",
        gender: Gender::Female,
    },
];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: false,
    voicegen_from_text: false,
    voice_cloning: VoiceCloningSupport::None,
    dialogue_multispeaker: None,
    sound_effects: false,
    realtime_bidirectional: true,
    streaming_output: true,
    voice_library: VoiceCatalog::Static {
        voices: GEMINI_LIVE_VOICES,
    },
    max_concurrent_streams: Some(4),
    languages: Languages::All,
    style_control: false,
    ssml: false,
    prosody_control: false,
    word_timestamps: false,
    max_chars_per_request: None,
    real_time_factor: None,
    typical_ttfb_ms: Some(280),
    requires_network: true,
    supported_output_formats: &[AudioFormat::Pcm {
        sample_rate: 24_000,
        channels: 1,
        sample: SampleType::I16,
    }],
    partial_results: true,
    cost_per_1k_chars_usd: None,
    cost_per_audio_min_usd: None,
};
