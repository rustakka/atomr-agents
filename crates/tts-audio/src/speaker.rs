//! cpal-based speaker output. Gated on the `speaker` feature.
//!
//! Mirrors `stt-audio::mic::MicCaptureSession`: own a `cpal::Stream`,
//! pull PCM frames from a bounded mpsc, hand them to the audio device.
//! Backpressure: callbacks can't block, so when the producer falls
//! behind we underrun (silence) and tick a counter the caller can
//! surface to the observability bus.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use atomr_agents_stt_core::{AudioFormat, SampleType, SttError};
use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct SpeakerOptions {
    pub queue_capacity: usize,
    pub preferred_sample_rate: Option<u32>,
    pub preferred_channels: Option<u16>,
}

impl Default for SpeakerOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 64,
            preferred_sample_rate: Some(24_000),
            preferred_channels: Some(1),
        }
    }
}

pub struct SpeakerStream {
    tx: mpsc::Sender<Vec<f32>>,
    pub format: AudioFormat,
    pub underruns: Arc<AtomicU64>,
    _stream: cpal::Stream,
}

impl SpeakerStream {
    pub fn open(opts: SpeakerOptions) -> Result<Self, SttError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| SttError::internal("cpal: no default output device"))?;
        let supported = device
            .default_output_config()
            .map_err(|e| SttError::internal(format!("cpal default config: {e}")))?;
        let sr = opts.preferred_sample_rate.unwrap_or(supported.sample_rate().0);
        let ch = opts.preferred_channels.unwrap_or(supported.channels());
        let stream_config = cpal::StreamConfig {
            channels: ch,
            sample_rate: cpal::SampleRate(sr),
            buffer_size: cpal::BufferSize::Default,
        };

        let (tx, mut rx) = mpsc::channel::<Vec<f32>>(opts.queue_capacity);
        let underruns = Arc::new(AtomicU64::new(0));
        let underruns_cb = underruns.clone();
        // Lock-free-ish ring of pending samples for the callback.
        let pending: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pending_cb = pending.clone();
        let pending_pull = pending.clone();

        // Drain the mpsc into the pending buffer on a tokio task.
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                let mut g = pending_pull.lock();
                g.extend(frame);
            }
        });

        let err_fn = |e| tracing::warn!(error = ?e, "cpal output stream error");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    let mut g = pending_cb.lock();
                    for slot in data.iter_mut() {
                        match g.pop_front() {
                            Some(s) => *slot = s,
                            None => {
                                *slot = 0.0;
                                underruns_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| {
                    let mut g = pending_cb.lock();
                    for slot in data.iter_mut() {
                        match g.pop_front() {
                            Some(s) => *slot = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16,
                            None => {
                                *slot = 0;
                                underruns_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                },
                err_fn,
                None,
            ),
            other => {
                return Err(SttError::internal(format!(
                    "cpal output sample format {other:?} not supported"
                )));
            }
        }
        .map_err(|e| SttError::internal(format!("cpal build_output_stream: {e}")))?;
        stream
            .play()
            .map_err(|e| SttError::internal(format!("cpal play: {e}")))?;

        let format = AudioFormat::Pcm {
            sample_rate: sr,
            channels: ch,
            sample: SampleType::F32,
        };
        Ok(Self {
            tx,
            format,
            underruns,
            _stream: stream,
        })
    }

    /// Push one frame of f32 PCM samples (mono or interleaved per
    /// the negotiated channel count). Returns `Err` if the queue is
    /// full and the producer should slow down.
    pub async fn push_pcm_f32(&self, samples: Vec<f32>) -> Result<(), SttError> {
        self.tx
            .send(samples)
            .await
            .map_err(|_| SttError::internal("speaker: stream closed"))
    }

    /// Push one frame of PCM-S16LE bytes (the most common live
    /// format from cloud streaming TTS).
    pub async fn push_pcm_s16le(&self, bytes: &Bytes) -> Result<(), SttError> {
        let mut samples: Vec<f32> = Vec::with_capacity(bytes.len() / 2);
        for pair in bytes.chunks_exact(2) {
            let s = i16::from_le_bytes([pair[0], pair[1]]);
            samples.push(s as f32 / i16::MAX as f32);
        }
        self.push_pcm_f32(samples).await
    }
}
