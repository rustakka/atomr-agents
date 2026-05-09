use atomr_agents_stt_core::{Capabilities, DiarizationSupport, Languages};

pub const CAPS: Capabilities = Capabilities {
    batch: true,
    streaming_push: false,
    realtime_microphone: false,
    // whisper.cpp doesn't ship diarization. Layer
    // `atomr-agents-stt-diarize-sherpa` on top.
    diarization: DiarizationSupport::None,
    word_timestamps: true,
    utterance_timestamps: true,
    language_detection: true,
    languages: Languages::All,
    punctuation: true,
    profanity_filter: false,
    max_audio_secs: None,
    max_concurrent_streams: Some(1),
    // Reference: base.en model, single-thread CPU on a M1 — actual
    // value varies wildly with model size and hardware.
    real_time_factor: Some(0.4),
    requires_network: false,
    supported_audio_formats: &[
        atomr_agents_stt_core::AudioFormat::Wav,
        atomr_agents_stt_core::AudioFormat::Mp3,
        atomr_agents_stt_core::AudioFormat::Flac,
        atomr_agents_stt_core::AudioFormat::Ogg,
        atomr_agents_stt_core::AudioFormat::Mp4,
        atomr_agents_stt_core::AudioFormat::Webm,
    ],
    min_chunk_ms: None,
    partial_results: false,
    redaction: false,
    vad_endpointing: false,
    custom_vocabulary: false,
    cost_per_audio_min_usd: Some(0.0),
};
