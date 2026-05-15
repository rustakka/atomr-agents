use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor};

pub const KOKORO_VOICES: &[VoiceDescriptor] = &[
    VoiceDescriptor {
        id: "af_alloy",
        name: "Alloy",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_aoede",
        name: "Aoede",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_bella",
        name: "Bella",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_jessica",
        name: "Jessica",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_kore",
        name: "Kore",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_nicole",
        name: "Nicole",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_nova",
        name: "Nova",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_sarah",
        name: "Sarah",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "af_sky",
        name: "Sky",
        language: "en-us",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "am_adam",
        name: "Adam",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_echo",
        name: "Echo",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_eric",
        name: "Eric",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_fenrir",
        name: "Fenrir",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_liam",
        name: "Liam",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_michael",
        name: "Michael",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_onyx",
        name: "Onyx",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "am_puck",
        name: "Puck",
        language: "en-us",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "bf_emma",
        name: "Emma",
        language: "en-gb",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "bf_isabella",
        name: "Isabella",
        language: "en-gb",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "bm_george",
        name: "George",
        language: "en-gb",
        gender: Gender::Male,
    },
    VoiceDescriptor {
        id: "bm_lewis",
        name: "Lewis",
        language: "en-gb",
        gender: Gender::Male,
    },
];

const KOKORO_LANGUAGES: &[&str] = &["en"];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: false,
    voice_cloning: VoiceCloningSupport::None,
    dialogue_multispeaker: None,
    sound_effects: false,
    realtime_bidirectional: false,
    streaming_output: true,
    voice_library: VoiceCatalog::Static {
        voices: KOKORO_VOICES,
    },
    max_concurrent_streams: None,
    languages: Languages::Subset(KOKORO_LANGUAGES),
    style_control: false,
    ssml: false,
    prosody_control: true,
    word_timestamps: false,
    max_chars_per_request: None,
    real_time_factor: Some(0.1),
    typical_ttfb_ms: Some(120),
    requires_network: false,
    supported_output_formats: &[AudioFormat::Pcm {
        sample_rate: 24_000,
        channels: 1,
        sample: SampleType::I16,
    }],
    partial_results: true,
    cost_per_1k_chars_usd: None,
    cost_per_audio_min_usd: None,
};
