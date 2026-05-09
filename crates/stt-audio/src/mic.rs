//! Microphone capture via `cpal`. Gated on the `mic` feature.
//!
//! [`MicCaptureSession::open`] negotiates a stream from the default
//! input device and pumps PCM frames into a bounded
//! `tokio::sync::mpsc` channel. Callers consume frames via
//! [`MicCaptureSession::frames`] and forward them to a
//! [`StreamingSession`](atomr_agents_stt_core::StreamingSession) (or
//! a UI waveform, or a recorder). Composability over coupling: this
//! type does not transcribe — it only produces audio.

use atomr_agents_stt_core::{AudioFormat, PcmBuffer, SampleType, SttError};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// One PCM frame from the microphone.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Interleaved PCM samples in the negotiated format.
    pub samples: Vec<f32>,
    /// Monotonic frame index (callback-based; useful for ordering).
    pub seq: u64,
}

#[derive(Debug, Clone)]
pub struct MicOptions {
    /// Channel capacity for the producer→consumer queue. When the
    /// consumer falls behind we drop frames rather than block the
    /// callback (which can't be allowed to block).
    pub queue_capacity: usize,
    /// Preferred sample rate. Falls back to the device default if
    /// the device doesn't support it.
    pub preferred_sample_rate: Option<u32>,
    /// Preferred channel count.
    pub preferred_channels: Option<u16>,
}

impl Default for MicOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 64,
            preferred_sample_rate: Some(16_000),
            preferred_channels: Some(1),
        }
    }
}

pub struct MicCaptureSession {
    rx: mpsc::Receiver<AudioFrame>,
    /// Negotiated format: callers feeding this into a `StreamingSession`
    /// pass it as the format hint.
    pub format: AudioFormat,
    /// Total frames the cpal callback dropped due to queue
    /// backpressure. Surface alongside `EventBus::Backpressure`.
    pub dropped: Arc<AtomicU64>,
    // Drop order: rx, then _stream — dropping the stream stops cpal.
    _stream: cpal::Stream,
}

impl MicCaptureSession {
    /// Open the default input device.
    pub fn open(opts: MicOptions) -> Result<Self, SttError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| SttError::internal("cpal: no default input device"))?;

        let supported = device
            .default_input_config()
            .map_err(|e| SttError::internal(format!("cpal default config: {e}")))?;

        // Negotiate sample rate / channels from preferred → device default.
        let device_sr = supported.sample_rate().0;
        let device_ch = supported.channels();
        let sr = opts.preferred_sample_rate.unwrap_or(device_sr);
        let ch = opts.preferred_channels.unwrap_or(device_ch);
        let sample_format = supported.sample_format();

        let stream_config = cpal::StreamConfig {
            channels: ch,
            sample_rate: cpal::SampleRate(sr),
            buffer_size: cpal::BufferSize::Default,
        };

        let (tx, rx) = mpsc::channel::<AudioFrame>(opts.queue_capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_cb = dropped.clone();
        let seq = Arc::new(AtomicU64::new(0));
        let seq_cb = seq.clone();

        let err_fn = |e| tracing::warn!(error = ?e, "cpal stream error");

        // Build the stream. cpal needs the right callback type per
        // sample format; we map all of them to f32 frames.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        push(&tx, &dropped_cb, &seq_cb, data.to_vec());
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| SttError::internal(format!("cpal build f32: {e}")))?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        let v: Vec<f32> = data.iter().map(|s| (*s as f32) / i16::MAX as f32).collect();
                        push(&tx, &dropped_cb, &seq_cb, v);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| SttError::internal(format!("cpal build i16: {e}")))?,
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        let v: Vec<f32> = data
                            .iter()
                            .map(|s| ((*s as f32) - 32_768.0) / 32_768.0)
                            .collect();
                        push(&tx, &dropped_cb, &seq_cb, v);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| SttError::internal(format!("cpal build u16: {e}")))?,
            other => {
                return Err(SttError::internal(format!(
                    "cpal sample format {other:?} not supported",
                )))
            }
        };
        stream
            .play()
            .map_err(|e| SttError::internal(format!("cpal play: {e}")))?;

        let format = AudioFormat::Pcm {
            sample_rate: sr,
            channels: ch,
            sample: SampleType::F32,
        };

        Ok(Self {
            rx,
            format,
            dropped,
            _stream: stream,
        })
    }

    /// Receive the next captured frame. Returns `None` when the
    /// stream is closed (device disconnected, etc.).
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.rx.recv().await
    }

    /// Mutable access to the underlying mpsc receiver, for callers
    /// that want to integrate it into a tokio `select!`.
    pub fn frames(&mut self) -> &mut mpsc::Receiver<AudioFrame> {
        &mut self.rx
    }

    /// Convert a captured frame into a [`PcmBuffer`].
    pub fn frame_to_pcm(&self, frame: AudioFrame) -> PcmBuffer {
        let (sr, ch) = match self.format {
            AudioFormat::Pcm {
                sample_rate,
                channels,
                ..
            } => (sample_rate, channels),
            _ => unreachable!("MicCaptureSession always negotiates AudioFormat::Pcm"),
        };
        PcmBuffer::new(frame.samples, sr, ch)
    }
}

fn push(
    tx: &mpsc::Sender<AudioFrame>,
    dropped: &Arc<AtomicU64>,
    seq: &Arc<AtomicU64>,
    samples: Vec<f32>,
) {
    let n = seq.fetch_add(1, Ordering::Relaxed);
    let frame = AudioFrame { samples, seq: n };
    if tx.try_send(frame).is_err() {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}
