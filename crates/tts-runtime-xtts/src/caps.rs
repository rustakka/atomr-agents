use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, VoiceCatalog, VoiceCloningSupport};

const XTTS_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "it", "pt", "pl", "tr", "ru", "nl", "cs", "ar", "zh", "ja", "ko",
    "hu", "hi",
];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: false,
    voice_cloning: VoiceCloningSupport::ZeroShot { min_sample_secs: 6.0 },
    dialogue_multispeaker: None,
    sound_effects: false,
    realtime_bidirectional: false,
    streaming_output: true,
    voice_library: VoiceCatalog::Dynamic,
    max_concurrent_streams: Some(2),
    languages: Languages::Subset(XTTS_LANGUAGES),
    style_control: false,
    ssml: false,
    prosody_control: true,
    word_timestamps: false,
    max_chars_per_request: Some(400),
    real_time_factor: Some(0.4),
    typical_ttfb_ms: Some(350),
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
