use atomr_agents_stt_core::{AudioFormat, Capabilities, DiarizationSupport, Languages};

const DEEPGRAM_LANGUAGES: &[&str] = &[
    "en", "en-US", "en-GB", "en-AU", "en-NZ", "en-IN",
    "es", "es-419",
    "fr", "fr-CA",
    "de", "it", "pt", "pt-BR", "pt-PT",
    "nl", "ru", "tr", "pl", "uk", "sv", "no", "da", "fi",
    "ja", "zh", "zh-CN", "zh-TW", "ko", "hi", "id", "th", "vi", "ms",
];

pub const CAPS: Capabilities = Capabilities {
    batch: true,
    streaming_push: true,
    realtime_microphone: true,
    diarization: DiarizationSupport::SpeakerCount,
    word_timestamps: true,
    utterance_timestamps: true,
    language_detection: true,
    languages: Languages::Subset(DEEPGRAM_LANGUAGES),
    punctuation: true,
    profanity_filter: true,
    max_audio_secs: None,
    max_concurrent_streams: Some(100),
    real_time_factor: None,
    requires_network: true,
    supported_audio_formats: &[
        AudioFormat::Wav,
        AudioFormat::Mp3,
        AudioFormat::Flac,
        AudioFormat::Ogg,
        AudioFormat::Opus,
        AudioFormat::Mulaw { sample_rate: 8_000 },
    ],
    min_chunk_ms: Some(20),
    partial_results: true,
    redaction: true,
    vad_endpointing: true,
    custom_vocabulary: true,
    cost_per_audio_min_usd: Some(0.0043),
};
