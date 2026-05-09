//! Deterministic mock TTS backend used by every downstream test
//! that needs a `TextToSpeech` instance without network or model
//! load. Mirrors `MockSpeechToText` in `stt-core`.

use std::pin::Pin;

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, PcmBuffer, Result, SampleType, SttError};
use bytes::Bytes;
use futures::Stream;

use crate::capabilities::{
    Capabilities, Gender, VoiceCatalog, VoiceCloningSupport, VoiceDescriptor,
};
use crate::kinds::BackendKind;
use crate::realtime::{RealtimeEvent, RealtimeOptions, RealtimeSession};
use crate::request::SynthesisRequest;
use crate::stream::{AudioChunk, SynthesisStream};
use crate::trait_::{AudioOutput, TextToSpeech};
use atomr_agents_stt_core::Languages;
use atomr_agents_stt_core::TransportKind;

const MOCK_VOICES: &[VoiceDescriptor] = &[
    VoiceDescriptor {
        id: "alpha",
        name: "Alpha",
        language: "en",
        gender: Gender::Female,
    },
    VoiceDescriptor {
        id: "beta",
        name: "Beta",
        language: "en",
        gender: Gender::Male,
    },
];

pub const MOCK_CAPS: Capabilities = Capabilities {
    plain_tts: true,
    voicegen_from_text: true,
    voice_cloning: VoiceCloningSupport::ZeroShot {
        min_sample_secs: 3.0,
    },
    dialogue_multispeaker: Some(5),
    sound_effects: true,
    realtime_bidirectional: true,
    streaming_output: true,
    voice_library: VoiceCatalog::Static { voices: MOCK_VOICES },
    max_concurrent_streams: Some(8),
    languages: Languages::All,
    style_control: true,
    ssml: false,
    prosody_control: true,
    word_timestamps: true,
    max_chars_per_request: None,
    real_time_factor: Some(0.0),
    typical_ttfb_ms: Some(0),
    requires_network: false,
    supported_output_formats: &[AudioFormat::Wav, AudioFormat::Mp3, AudioFormat::Opus],
    partial_results: true,
    cost_per_1k_chars_usd: Some(0.0),
    cost_per_audio_min_usd: Some(0.0),
};

/// Deterministic mock TTS. `synthesize` returns a constant-length
/// PCM buffer (silence) sized proportionally to the input character
/// count so callers can verify shape without caring about audio.
pub struct MockTextToSpeech {
    sample_rate: u32,
    pre_pad_ms: u32,
}

impl Default for MockTextToSpeech {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTextToSpeech {
    pub fn new() -> Self {
        Self {
            sample_rate: 16_000,
            pre_pad_ms: 0,
        }
    }

    pub fn with_sample_rate(mut self, sr: u32) -> Self {
        self.sample_rate = sr;
        self
    }

    fn render_silence(&self, char_count: u32) -> PcmBuffer {
        // ~80 chars/sec at typical narration speed.
        let secs = (char_count as f32 / 80.0).max(0.1);
        let pad_secs = self.pre_pad_ms as f32 / 1000.0;
        let n = ((secs + pad_secs) * self.sample_rate as f32) as usize;
        PcmBuffer::new(vec![0.0; n], self.sample_rate, 1)
    }
}

fn char_count(req: &SynthesisRequest) -> u32 {
    let n = match req {
        SynthesisRequest::Tts { text, .. } => text.chars().count(),
        SynthesisRequest::SoundEffect { prompt, .. } => prompt.chars().count(),
        SynthesisRequest::Dialogue { script, .. } => {
            script.iter().map(|t| t.text.chars().count()).sum::<usize>()
        }
    };
    n as u32
}

#[async_trait]
impl TextToSpeech for MockTextToSpeech {
    fn capabilities(&self) -> &'static Capabilities {
        &MOCK_CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::Custom(std::borrow::Cow::Borrowed("mock"))
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::LocalModel
    }

    async fn synthesize(&self, request: SynthesisRequest) -> Result<AudioOutput> {
        let chars = char_count(&request);
        let pcm = self.render_silence(chars);
        let mut out = AudioOutput::from_pcm(
            pcm,
            BackendKind::Custom(std::borrow::Cow::Borrowed("mock")),
            chars,
        );
        out.model_id = Some("mock-tts-1".into());
        out.cost_usd = Some(0.0);
        if let SynthesisRequest::Tts { voice, .. } = &request {
            if let crate::voice::VoiceRef::Library { id } = voice {
                out.voice_id_used = Some(id.clone());
            }
        }
        Ok(out)
    }

    async fn synthesize_stream(
        &self,
        request: SynthesisRequest,
    ) -> Result<Box<dyn SynthesisStream>> {
        let chars = char_count(&request);
        let pcm = self.render_silence(chars);
        Ok(Box::new(MockSynthesisStream::new(pcm, self.sample_rate)))
    }

    async fn open_realtime(
        &self,
        _opts: RealtimeOptions,
    ) -> Result<Box<dyn RealtimeSession>> {
        Ok(Box::new(MockRealtimeSession::new(self.sample_rate)))
    }
}

pub struct MockSynthesisStream {
    sample_rate: u32,
    queue: tokio::sync::mpsc::UnboundedReceiver<std::result::Result<AudioChunk, SttError>>,
    format: AudioFormat,
}

impl MockSynthesisStream {
    fn new(pcm: PcmBuffer, sample_rate: u32) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // Slice the PCM into ~100ms chunks and emit one Final at the end.
        let chunk_samples = (sample_rate / 10) as usize;
        let total = pcm.samples.len();
        let mut seq = 0u64;
        let mut idx = 0;
        while idx < total {
            let end = (idx + chunk_samples).min(total);
            let slice = &pcm.samples[idx..end];
            let bytes = pcm_f32_to_bytes(slice);
            let _ = tx.send(Ok(AudioChunk {
                bytes: Bytes::from(bytes),
                seq,
                is_final: end == total,
                words: Vec::new(),
            }));
            seq += 1;
            idx = end;
        }
        if total == 0 {
            let _ = tx.send(Ok(AudioChunk {
                bytes: Bytes::new(),
                seq: 0,
                is_final: true,
                words: Vec::new(),
            }));
        }
        Self {
            sample_rate,
            queue: rx,
            format: AudioFormat::Pcm {
                sample_rate,
                channels: 1,
                sample: SampleType::F32,
            },
        }
    }
}

#[async_trait]
impl SynthesisStream for MockSynthesisStream {
    fn capabilities(&self) -> &'static Capabilities {
        &MOCK_CAPS
    }

    fn format(&self) -> &AudioFormat {
        &self.format
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<AudioChunk, SttError>> + Send + '_>>
    {
        Box::pin(futures::stream::poll_fn(move |cx| self.queue.poll_recv(cx)))
    }

    async fn close(&mut self) -> Result<()> {
        // Drop the receiver — done.
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.queue = rx;
        let _ = self.sample_rate;
        Ok(())
    }
}

pub struct MockRealtimeSession {
    sample_rate: u32,
    tx: tokio::sync::mpsc::UnboundedSender<std::result::Result<RealtimeEvent, SttError>>,
    rx: tokio::sync::mpsc::UnboundedReceiver<std::result::Result<RealtimeEvent, SttError>>,
}

impl MockRealtimeSession {
    fn new(sample_rate: u32) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self { sample_rate, tx, rx }
    }
}

#[async_trait]
impl RealtimeSession for MockRealtimeSession {
    fn capabilities(&self) -> &'static Capabilities {
        &MOCK_CAPS
    }

    async fn push_text(&mut self, text: &str) -> Result<()> {
        let _ = self.tx.send(Ok(RealtimeEvent::OutboundText {
            text: text.to_string(),
            is_final: true,
        }));
        // Render some silence as the "response".
        let pcm = vec![0.0f32; (self.sample_rate / 4) as usize];
        let bytes = pcm_f32_to_bytes(&pcm);
        let _ = self.tx.send(Ok(RealtimeEvent::AudioOut {
            chunk: AudioChunk {
                bytes: Bytes::from(bytes),
                seq: 0,
                is_final: true,
                words: Vec::new(),
            },
        }));
        let _ = self.tx.send(Ok(RealtimeEvent::Done));
        Ok(())
    }

    async fn push_audio(&mut self, _chunk: Bytes) -> Result<()> {
        let _ = self.tx.send(Ok(RealtimeEvent::InboundTranscript {
            text: "(mock transcript)".to_string(),
            is_final: false,
        }));
        Ok(())
    }

    async fn commit_input(&mut self) -> Result<()> {
        let _ = self.tx.send(Ok(RealtimeEvent::UserSpeechEnded));
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<()> {
        let _ = self.tx.send(Ok(RealtimeEvent::BargeIn));
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        self.tx = tx;
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<RealtimeEvent, SttError>> + Send + '_>>
    {
        Box::pin(futures::stream::poll_fn(move |cx| self.rx.poll_recv(cx)))
    }
}

fn pcm_f32_to_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::VoiceRef;

    #[tokio::test]
    async fn batch_returns_silence_proportional_to_text() {
        let m = MockTextToSpeech::new();
        let req = SynthesisRequest::tts("Hello world", VoiceRef::library("alpha"));
        let out = m.synthesize(req).await.unwrap();
        assert_eq!(out.backend.as_str(), "mock");
        assert_eq!(out.voice_id_used.as_deref(), Some("alpha"));
        assert!(out.duration_secs > 0.0);
        assert!(out.characters_processed > 0);
    }

    #[tokio::test]
    async fn stream_emits_chunks_then_final() {
        use futures::StreamExt;
        let m = MockTextToSpeech::new();
        let req = SynthesisRequest::tts(
            "This is a streaming test sentence",
            VoiceRef::library("alpha"),
        );
        let mut s = m.synthesize_stream(req).await.unwrap();
        let mut chunks = 0;
        let mut got_final = false;
        let mut stream = s.events();
        while let Some(item) = stream.next().await {
            let chunk = item.unwrap();
            chunks += 1;
            if chunk.is_final {
                got_final = true;
                break;
            }
        }
        assert!(chunks > 0);
        assert!(got_final);
    }

    #[tokio::test]
    async fn realtime_session_emits_audio_and_done() {
        use futures::StreamExt;
        let m = MockTextToSpeech::new();
        let mut rs = m.open_realtime(RealtimeOptions::default()).await.unwrap();
        rs.push_text("hello there").await.unwrap();
        let mut stream = rs.events();
        let first = stream.next().await.unwrap().unwrap();
        assert!(matches!(first, RealtimeEvent::OutboundText { .. }));
        let second = stream.next().await.unwrap().unwrap();
        assert!(matches!(second, RealtimeEvent::AudioOut { .. }));
        let third = stream.next().await.unwrap().unwrap();
        assert!(matches!(third, RealtimeEvent::Done));
    }

    #[test]
    fn caps_serialize_to_json() {
        let v = serde_json::to_value(&MOCK_CAPS).unwrap();
        assert_eq!(v["plain_tts"], true);
        assert_eq!(v["realtime_bidirectional"], true);
        assert_eq!(v["voice_cloning"]["kind"], "zero_shot");
    }
}
