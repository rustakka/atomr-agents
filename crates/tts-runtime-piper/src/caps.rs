use atomr_agents_stt_core::{AudioFormat, Languages, SampleType};
use atomr_agents_tts_core::{Capabilities, VoiceCatalog, VoiceCloningSupport};

const PIPER_LANGUAGES: &[&str] = &[
    "ar", "ca", "cs", "cy", "da", "de", "el", "en", "es", "fa", "fi", "fr", "hu", "is", "it", "ka", "kk",
    "lb", "ne", "nl", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sr", "sv", "sw", "tr", "uk", "vi", "zh",
];

pub const CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: false,
    voice_cloning: VoiceCloningSupport::None,
    dialogue_multispeaker: None,
    sound_effects: false,
    realtime_bidirectional: false,
    streaming_output: true,
    voice_library: VoiceCatalog::Dynamic,
    max_concurrent_streams: None,
    languages: Languages::Subset(PIPER_LANGUAGES),
    style_control: false,
    ssml: false,
    prosody_control: true,
    word_timestamps: false,
    max_chars_per_request: None,
    real_time_factor: Some(0.05),
    typical_ttfb_ms: Some(80),
    requires_network: false,
    supported_output_formats: &[AudioFormat::Pcm {
        sample_rate: 22_050,
        channels: 1,
        sample: SampleType::I16,
    }],
    partial_results: true,
    cost_per_1k_chars_usd: None,
    cost_per_audio_min_usd: None,
};
