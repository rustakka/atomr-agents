use atomr_agents_stt_core::{AudioFormat, Languages};
use atomr_agents_tts_core::{Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor};

const OPENAI_VOICES: &[VoiceDescriptor] = &[
    VoiceDescriptor {
        id: "alloy",
        name: "Alloy",
        language: "en",
        gender: Gender::Neutral,
    },
    VoiceDescriptor {
        id: "echo",
        name: "Echo",
        language: "en",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "fable",
        name: "Fable",
        language: "en",
        gender: Gender::Neutral,
    },
    VoiceDescriptor {
        id: "onyx",
        name: "Onyx",
        language: "en",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "nova",
        name: "Nova",
        language: "en",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "shimmer",
        name: "Shimmer",
        language: "en",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "ash",
        name: "Ash",
        language: "en",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "ballad",
        name: "Ballad",
        language: "en",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "coral",
        name: "Coral",
        language: "en",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "sage",
        name: "Sage",
        language: "en",
        gender: Gender::Neutral,
    },
    VoiceDescriptor {
        id: "verse",
        name: "Verse",
        language: "en",
        gender: Gender::Neutral,
    },
];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: false,
    voice_cloning: VoiceCloningSupport::None,
    dialogue_multispeaker: None,
    sound_effects: false,
    realtime_bidirectional: false,
    streaming_output: true,
    voice_library: VoiceCatalog::Static {
        voices: OPENAI_VOICES,
    },
    max_concurrent_streams: None,
    languages: Languages::All,
    style_control: true,
    ssml: false,
    prosody_control: false,
    word_timestamps: false,
    max_chars_per_request: Some(4_096),
    real_time_factor: None,
    typical_ttfb_ms: Some(400),
    requires_network: true,
    supported_output_formats: &[
        AudioFormat::Mp3,
        AudioFormat::Opus,
        AudioFormat::Aac,
        AudioFormat::Flac,
        AudioFormat::Wav,
        AudioFormat::Pcm {
            sample_rate: 24_000,
            channels: 1,
            sample: atomr_agents_stt_core::SampleType::I16,
        },
    ],
    partial_results: true,
    cost_per_1k_chars_usd: Some(0.015),
    cost_per_audio_min_usd: None,
};
