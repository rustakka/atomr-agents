use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, VoiceCatalog, VoiceCloningSupport};

const MOSS_LANGUAGES: &[&str] = &["en", "zh", "ja", "ko", "fr", "de", "es", "ar", "ru", "hi"];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: true,
    voice_cloning: VoiceCloningSupport::ZeroShot { min_sample_secs: 3.0 },
    dialogue_multispeaker: Some(5),
    sound_effects: true,
    realtime_bidirectional: true,
    streaming_output: true,
    voice_library: VoiceCatalog::Dynamic,
    max_concurrent_streams: Some(4),
    languages: Languages::Subset(MOSS_LANGUAGES),
    style_control: true,
    ssml: false,
    prosody_control: true,
    word_timestamps: false,
    max_chars_per_request: None,
    real_time_factor: Some(0.51),
    typical_ttfb_ms: Some(180),
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
