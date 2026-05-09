//! The central [`TextToSpeech`] trait.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, PcmBuffer, Result};
use serde::Serialize;

use crate::capabilities::Capabilities;
use crate::kinds::BackendKind;
use crate::realtime::{RealtimeOptions, RealtimeSession};
use crate::request::SynthesisRequest;
use crate::stream::SynthesisStream;
use atomr_agents_stt_core::TransportKind;

#[async_trait]
pub trait TextToSpeech: Send + Sync + 'static {
    fn capabilities(&self) -> &'static Capabilities;
    fn backend_kind(&self) -> BackendKind;
    fn transport_kind(&self) -> TransportKind;

    /// Batch synthesis: full request → full audio output.
    async fn synthesize(&self, request: SynthesisRequest) -> Result<AudioOutput>;

    /// Streaming output. Backends without streaming should return
    /// [`atomr_agents_stt_core::SttError::UnsupportedCapability`].
    async fn synthesize_stream(
        &self,
        request: SynthesisRequest,
    ) -> Result<Box<dyn SynthesisStream>>;

    /// Open a bidirectional realtime session. Backends without
    /// realtime return UnsupportedCapability.
    async fn open_realtime(
        &self,
        opts: RealtimeOptions,
    ) -> Result<Box<dyn RealtimeSession>>;
}

pub type DynTextToSpeech = Arc<dyn TextToSpeech>;

#[derive(Debug, Clone, Serialize)]
pub struct AudioOutput {
    /// Decoded PCM samples. Backends that emit a container directly
    /// (MP3 / Opus) are responsible for decoding before constructing
    /// this struct, OR for setting `container_bytes` instead.
    #[serde(skip)]
    pub audio: PcmBuffer,
    /// The format the backend produced. For PCM-decoded results this
    /// will be `AudioFormat::Pcm{...}`. For container-only results it
    /// describes the container.
    pub format: AudioFormat,
    /// Optional container bytes (when the backend returned compressed
    /// audio and we don't want to decode eagerly).
    #[serde(skip)]
    pub container_bytes: Option<bytes::Bytes>,
    pub duration_secs: f32,
    pub characters_processed: u32,
    pub backend: BackendKind,
    pub model_id: Option<String>,
    pub voice_id_used: Option<String>,
    pub cost_usd: Option<f32>,
}

impl AudioOutput {
    /// Construct from a PCM buffer. Sets `format` to a matching
    /// `AudioFormat::Pcm` and computes `duration_secs` from the
    /// buffer.
    pub fn from_pcm(
        pcm: PcmBuffer,
        backend: BackendKind,
        characters_processed: u32,
    ) -> Self {
        use atomr_agents_stt_core::SampleType;
        let format = AudioFormat::Pcm {
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
            sample: SampleType::F32,
        };
        let duration_secs = pcm.duration_secs();
        Self {
            audio: pcm,
            format,
            container_bytes: None,
            duration_secs,
            characters_processed,
            backend,
            model_id: None,
            voice_id_used: None,
            cost_usd: None,
        }
    }

    /// Construct from container bytes (e.g. an MP3 the backend
    /// returned that we don't want to decode immediately). Sets
    /// `audio` to an empty PCM buffer; callers can decode lazily
    /// via `stt-audio::decode::decode_to_pcm` on `container_bytes`.
    pub fn from_container(
        bytes: bytes::Bytes,
        format: AudioFormat,
        duration_secs: f32,
        backend: BackendKind,
        characters_processed: u32,
    ) -> Self {
        Self {
            audio: PcmBuffer::new(Vec::new(), 0, 0),
            format,
            container_bytes: Some(bytes),
            duration_secs,
            characters_processed,
            backend,
            model_id: None,
            voice_id_used: None,
            cost_usd: None,
        }
    }
}
