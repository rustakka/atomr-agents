//! Source-agnostic audio feed.
//!
//! [`AudioSource`] abstracts where the harness gets audio — a live
//! microphone, a file on disk, in-memory container bytes, or
//! pre-decoded PCM — so the loop never cares. Each source builds an
//! [`AudioPump`] that yields fixed-size [`AudioChunk`]s: wire bytes to
//! push to the streaming session, plus (when available) the decoded
//! mono PCM slice that layered diarization needs.

use async_trait::async_trait;
use bytes::Bytes;

use atomr_agents_stt_core::{AudioFormat, PcmBuffer, SampleType};

use crate::error::Result;
#[allow(unused_imports)]
use crate::error::SttHarnessError;

/// Default chunk size, in milliseconds of audio, for the file/PCM
/// pumps. Small enough for responsive streaming, large enough to avoid
/// per-sample overhead.
const DEFAULT_CHUNK_MS: u32 = 100;

/// Where the harness gets its audio.
#[derive(Debug)]
pub enum AudioSource {
    /// Live microphone capture. Requires the `mic` feature.
    #[cfg(feature = "mic")]
    Mic(atomr_agents_stt_audio::mic::MicOptions),
    /// A file on disk, decoded then re-chunked. Requires `decode`.
    File(std::path::PathBuf),
    /// In-memory container bytes, decoded then re-chunked. Requires
    /// `decode`.
    Bytes { data: Bytes, format: AudioFormat },
    /// Already-decoded PCM. The zero-I/O path used by tests.
    Pcm(PcmBuffer),
}

impl From<atomr_agents_stt_core::AudioInput> for AudioSource {
    /// Adapt an [`AudioInput`](atomr_agents_stt_core::AudioInput) — the
    /// STT stack's batch input type — into a streaming
    /// [`AudioSource`]. `File` and `Bytes` still require the `decode`
    /// feature to actually pump (checked in [`AudioSource::into_pump`]).
    fn from(input: atomr_agents_stt_core::AudioInput) -> Self {
        use atomr_agents_stt_core::AudioInput;
        match input {
            AudioInput::File(path) => AudioSource::File(path),
            AudioInput::Bytes { data, format } => AudioSource::Bytes { data, format },
            AudioInput::Pcm(buf) => AudioSource::Pcm(buf),
        }
    }
}

impl AudioSource {
    /// Build the [`AudioPump`] for this source. Fails with a clear
    /// message when the source needs a cargo feature that is off.
    pub(crate) fn into_pump(self) -> Result<Box<dyn AudioPump>> {
        match self {
            AudioSource::Pcm(buf) => Ok(Box::new(PcmPump::new(buf, DEFAULT_CHUNK_MS))),
            #[cfg(feature = "decode")]
            AudioSource::File(path) => {
                let pcm = atomr_agents_stt_audio::decode::decode_to_pcm(
                    atomr_agents_stt_core::AudioInput::File(path),
                )?;
                Ok(Box::new(PcmPump::new(pcm, DEFAULT_CHUNK_MS)))
            }
            #[cfg(feature = "decode")]
            AudioSource::Bytes { data, format } => {
                let pcm = atomr_agents_stt_audio::decode::decode_to_pcm(
                    atomr_agents_stt_core::AudioInput::Bytes { data, format },
                )?;
                Ok(Box::new(PcmPump::new(pcm, DEFAULT_CHUNK_MS)))
            }
            #[cfg(not(feature = "decode"))]
            AudioSource::File(_) | AudioSource::Bytes { .. } => Err(SttHarnessError::config(
                "File / Bytes audio sources require the `decode` feature",
            )),
            #[cfg(feature = "mic")]
            AudioSource::Mic(opts) => Ok(Box::new(MicPump::open(opts)?)),
        }
    }
}

/// One unit of audio pulled from a source.
pub(crate) struct AudioChunk {
    /// Wire-format bytes to push to the `StreamingSession`.
    pub bytes: Bytes,
    /// Decoded mono f32 PCM for this chunk, when the source has it
    /// (file / bytes / pcm / mic). Used only for layered diarization.
    pub pcm: Option<PcmBuffer>,
}

/// A uniform "give me the next chunk" surface the session task pumps
/// from. One implementation per [`AudioSource`] variant.
#[async_trait]
pub(crate) trait AudioPump: Send {
    /// The next chunk, or `None` once the source is exhausted.
    async fn next_chunk(&mut self) -> Result<Option<AudioChunk>>;
    /// The wire format the pump produces (used as the `open_stream`
    /// format hint when the caller did not set one).
    fn format(&self) -> AudioFormat;
}

/// Pack mono f32 samples into little-endian PCM-16 wire bytes.
fn pcm_f32_to_pcm16_le(samples: &[f32]) -> Bytes {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    Bytes::from(out)
}

/// Average a (possibly multi-channel) PCM buffer down to mono.
fn to_mono(pcm: &PcmBuffer) -> Vec<f32> {
    if pcm.channels <= 1 {
        return pcm.samples.clone();
    }
    let chs = pcm.channels as usize;
    let frames = pcm.samples.len() / chs;
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..chs {
            acc += pcm.samples[f * chs + c];
        }
        mono.push(acc / chs as f32);
    }
    mono
}

/// Pump over an in-memory mono PCM buffer. Files and byte buffers are
/// decoded eagerly into this same pump.
pub(crate) struct PcmPump {
    samples: Vec<f32>,
    sample_rate: u32,
    pos: usize,
    chunk_frames: usize,
}

impl PcmPump {
    pub(crate) fn new(pcm: PcmBuffer, chunk_ms: u32) -> Self {
        let sample_rate = pcm.sample_rate.max(1);
        let samples = to_mono(&pcm);
        let chunk_frames = ((sample_rate as u64 * chunk_ms as u64) / 1000).max(1) as usize;
        Self {
            samples,
            sample_rate,
            pos: 0,
            chunk_frames,
        }
    }
}

#[async_trait]
impl AudioPump for PcmPump {
    async fn next_chunk(&mut self) -> Result<Option<AudioChunk>> {
        if self.pos >= self.samples.len() {
            return Ok(None);
        }
        let end = (self.pos + self.chunk_frames).min(self.samples.len());
        let slice = &self.samples[self.pos..end];
        self.pos = end;
        let bytes = pcm_f32_to_pcm16_le(slice);
        let pcm = PcmBuffer::new(slice.to_vec(), self.sample_rate, 1);
        Ok(Some(AudioChunk {
            bytes,
            pcm: Some(pcm),
        }))
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::Pcm {
            sample_rate: self.sample_rate,
            channels: 1,
            sample: SampleType::I16,
        }
    }
}

/// Pump over a live microphone capture session.
///
/// `cpal`'s stream handle is `!Send`, so the [`MicCaptureSession`]
/// cannot move across threads — but the session task does run on the
/// tokio thread pool. To bridge that, capture lives on a dedicated OS
/// thread that owns the session and forwards `AudioFrame`s through a
/// `Send` channel; `MicPump` holds only the receiving end.
///
/// [`MicCaptureSession`]: atomr_agents_stt_audio::mic::MicCaptureSession
#[cfg(feature = "mic")]
pub(crate) struct MicPump {
    rx: tokio::sync::mpsc::UnboundedReceiver<atomr_agents_stt_audio::mic::AudioFrame>,
    sample_rate: u32,
    channels: u16,
    _capture_thread: std::thread::JoinHandle<()>,
}

#[cfg(feature = "mic")]
impl MicPump {
    pub(crate) fn open(opts: atomr_agents_stt_audio::mic::MicOptions) -> Result<Self> {
        use atomr_agents_stt_audio::mic::MicCaptureSession;

        let (frame_tx, frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<atomr_agents_stt_audio::mic::AudioFrame>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(u32, u16)>>();

        let capture_thread = std::thread::spawn(move || {
            let mut session = match MicCaptureSession::open(opts) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(SttHarnessError::from(e)));
                    return;
                }
            };
            let (sr, ch) = match session.format {
                AudioFormat::Pcm {
                    sample_rate,
                    channels,
                    ..
                } => (sample_rate, channels),
                _ => (16_000, 1),
            };
            if ready_tx.send(Ok((sr, ch))).is_err() {
                return;
            }
            // A current-thread runtime drives `recv()` on the thread
            // that owns the `cpal::Stream`.
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            rt.block_on(async move {
                while let Some(frame) = session.recv().await {
                    if frame_tx.send(frame).is_err() {
                        break;
                    }
                }
            });
            // `session` (and the `cpal::Stream`) is dropped here, on
            // the thread that created it.
        });

        let (sample_rate, channels) = ready_rx
            .recv()
            .map_err(|_| SttHarnessError::audio("mic capture thread exited before init"))??;

        Ok(Self {
            rx: frame_rx,
            sample_rate,
            channels,
            _capture_thread: capture_thread,
        })
    }
}

#[cfg(feature = "mic")]
#[async_trait]
impl AudioPump for MicPump {
    async fn next_chunk(&mut self) -> Result<Option<AudioChunk>> {
        match self.rx.recv().await {
            None => Ok(None),
            Some(frame) => {
                let pcm = PcmBuffer::new(frame.samples, self.sample_rate, self.channels);
                let mono = to_mono(&pcm);
                let bytes = pcm_f32_to_pcm16_le(&mono);
                let mono_pcm = PcmBuffer::new(mono, self.sample_rate, 1);
                Ok(Some(AudioChunk {
                    bytes,
                    pcm: Some(mono_pcm),
                }))
            }
        }
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::Pcm {
            sample_rate: self.sample_rate,
            channels: 1,
            sample: SampleType::I16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pcm_pump_chunks_and_drains() {
        // 1 second of silence at 16 kHz mono → 10 chunks of 100 ms.
        let pcm = PcmBuffer::new(vec![0.0; 16_000], 16_000, 1);
        let mut pump = PcmPump::new(pcm, 100);
        let mut chunks = 0;
        let mut total_samples = 0;
        while let Some(chunk) = pump.next_chunk().await.unwrap() {
            chunks += 1;
            total_samples += chunk.pcm.as_ref().unwrap().samples.len();
            // PCM-16 wire bytes are 2 per sample.
            assert_eq!(chunk.bytes.len(), chunk.pcm.as_ref().unwrap().samples.len() * 2);
        }
        assert_eq!(chunks, 10);
        assert_eq!(total_samples, 16_000);
    }

    #[tokio::test]
    async fn pcm_pump_mixes_stereo_to_mono() {
        // 2 stereo frames: averages to [0.5, 0.5].
        let pcm = PcmBuffer::new(vec![1.0, 0.0, 0.0, 1.0], 16_000, 2);
        let mut pump = PcmPump::new(pcm, 1000);
        let chunk = pump.next_chunk().await.unwrap().unwrap();
        assert_eq!(chunk.pcm.unwrap().samples, vec![0.5, 0.5]);
    }
}
