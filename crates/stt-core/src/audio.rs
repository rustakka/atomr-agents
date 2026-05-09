//! Audio input shape. The trait is decode-agnostic — `AudioInput`
//! carries either a path, a byte buffer (with a declared format),
//! already-decoded PCM, or an async reader. `atomr-agents-stt-audio`
//! provides the actual symphonia-based decoder used by local backends.

use std::path::PathBuf;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum AudioInput {
    /// File on disk. Format is inferred from the extension by
    /// `stt-audio::decode`, or supplied alongside via the cloud
    /// backend's multipart filename header.
    File(PathBuf),
    /// In-memory buffer with an explicit format hint.
    Bytes { data: Bytes, format: AudioFormat },
    /// Already-decoded PCM samples. Backends that do their own
    /// decode (whisper-rs) avoid the symphonia round-trip.
    Pcm(PcmBuffer),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioFormat {
    Pcm {
        sample_rate: u32,
        channels: u16,
        sample: SampleType,
    },
    Wav,
    Mp3,
    Flac,
    Ogg,
    Opus,
    Webm,
    Mp4,
    Aac,
    /// 8 kHz µ-law (telephony).
    Mulaw {
        sample_rate: u32,
    },
}

impl AudioFormat {
    /// Conventional MIME type for HTTP uploads.
    pub fn mime(&self) -> &'static str {
        match self {
            AudioFormat::Pcm { .. } => "audio/wav",
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Opus => "audio/opus",
            AudioFormat::Webm => "audio/webm",
            AudioFormat::Mp4 => "audio/mp4",
            AudioFormat::Aac => "audio/aac",
            AudioFormat::Mulaw { .. } => "audio/basic",
        }
    }

    /// Conventional file extension (no leading dot).
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Pcm { .. } | AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::Webm => "webm",
            AudioFormat::Mp4 => "mp4",
            AudioFormat::Aac => "aac",
            AudioFormat::Mulaw { .. } => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleType {
    I16,
    I32,
    F32,
}

/// Decoded PCM. Backends that resample (e.g. whisper-rs needs 16 kHz
/// mono f32) take this and convert.
#[derive(Debug, Clone)]
pub struct PcmBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmBuffer {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
        }
    }

    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            0.0
        } else {
            (self.samples.len() as f32) / (self.sample_rate as f32 * self.channels as f32)
        }
    }
}
