use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor};

pub const OPENAI_REALTIME_VOICES: &[VoiceDescriptor] = &[
    VoiceDescriptor {
        id: "alloy",
        name: "Alloy",
        language: "en-us",
        gender: Gender::Neutral,
    },
    VoiceDescriptor {
        id: "ash",
        name: "Ash",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "ballad",
        name: "Ballad",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "coral",
        name: "Coral",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "echo",
        name: "Echo",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "sage",
        name: "Sage",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "shimmer",
        name: "Shimmer",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "verse",
        name: "Verse",
        language: "en-us",
        gender: Gender::Male,
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
        voices: OPENAI_REALTIME_VOICES,
    },
    max_concurrent_streams: Some(8),
    languages: Languages::All,
    style_control: true,
    ssml: false,
    prosody_control: false,
    word_timestamps: false,
    max_chars_per_request: None,
    real_time_factor: None,
    typical_ttfb_ms: Some(220),
    requires_network: true,
    supported_output_formats: &[
        AudioFormat::Pcm {
            sample_rate: 24_000,
            channels: 1,
            sample: SampleType::I16,
        },
        AudioFormat::Mulaw { sample_rate: 8_000 },
    ],
    partial_results: true,
    cost_per_1k_chars_usd: None,
    cost_per_audio_min_usd: Some(0.06),
};
