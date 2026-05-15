use atomr_agents_stt_core::{AudioFormat, Languages};
use atomr_agents_tts_core::{Capabilities, VoiceCatalog, VoiceCloningSupport};

const ELEVENLABS_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "it", "pt", "pl", "tr", "ru", "nl", "cs", "ar", "zh", "ja", "hu", "ko", "hi",
    "id", "fi", "fil", "uk", "el", "vi", "no", "ro", "da", "sk", "sv", "ta", "ms",
];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: true,
    voice_cloning: VoiceCloningSupport::Both {
        min_sample_secs: 60.0,
    },
    dialogue_multispeaker: None,
    sound_effects: true,
    realtime_bidirectional: true,
    streaming_output: true,
    voice_library: VoiceCatalog::Dynamic,
    max_concurrent_streams: Some(10),
    languages: Languages::Subset(ELEVENLABS_LANGUAGES),
    style_control: true,
    ssml: false,
    prosody_control: false,
    word_timestamps: true,
    max_chars_per_request: Some(5_000),
    real_time_factor: None,
    typical_ttfb_ms: Some(150),
    requires_network: true,
    supported_output_formats: &[
        AudioFormat::Mp3,
        AudioFormat::Pcm {
            sample_rate: 16_000,
            channels: 1,
            sample: atomr_agents_stt_core::SampleType::I16,
        },
        AudioFormat::Pcm {
            sample_rate: 22_050,
            channels: 1,
            sample: atomr_agents_stt_core::SampleType::I16,
        },
        AudioFormat::Pcm {
            sample_rate: 24_000,
            channels: 1,
            sample: atomr_agents_stt_core::SampleType::I16,
        },
        AudioFormat::Pcm {
            sample_rate: 44_100,
            channels: 1,
            sample: atomr_agents_stt_core::SampleType::I16,
        },
        AudioFormat::Mulaw { sample_rate: 8_000 },
    ],
    partial_results: true,
    cost_per_1k_chars_usd: Some(0.30),
    cost_per_audio_min_usd: None,
};
