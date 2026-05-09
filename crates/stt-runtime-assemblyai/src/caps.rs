use atomr_agents_stt_core::{AudioFormat, Capabilities, DiarizationSupport, Languages};

const ASSEMBLY_LANGUAGES: &[&str] = &[
    "en", "en-US", "en-AU", "en-GB",
    "es", "fr", "de", "it", "pt", "nl", "ja", "zh", "ko", "hi", "uk",
    "ru", "tr", "fi", "pl", "no", "sv", "da", "id", "ms", "vi", "th",
];

pub const CAPS: Capabilities = Capabilities {
    batch: true,
    streaming_push: true,
    realtime_microphone: true,
    diarization: DiarizationSupport::NamedSpeakers,
    word_timestamps: true,
    utterance_timestamps: true,
    language_detection: true,
    languages: Languages::Subset(ASSEMBLY_LANGUAGES),
    punctuation: true,
    profanity_filter: true,
    max_audio_secs: None,
    max_concurrent_streams: Some(32),
    real_time_factor: None,
    requires_network: true,
    supported_audio_formats: &[
        AudioFormat::Wav,
        AudioFormat::Mp3,
        AudioFormat::Flac,
        AudioFormat::Mp4,
        AudioFormat::Webm,
        AudioFormat::Ogg,
    ],
    min_chunk_ms: Some(50),
    partial_results: true,
    redaction: true,
    vad_endpointing: true,
    custom_vocabulary: true,
    cost_per_audio_min_usd: Some(0.0062),
};
