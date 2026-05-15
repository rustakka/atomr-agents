//! Deterministic mock backend used by every downstream test that
//! needs a `SpeechToText` instance without network or model load.
//! Mirrors `MockEmbedder` in `atomr-agents-embed`.

use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use crate::audio::{AudioFormat, AudioInput};
use crate::capabilities::{Capabilities, DiarizationSupport, Languages};
use crate::error::{Result, SttError};
use crate::kinds::{BackendKind, TransportKind};
use crate::stream::{StreamEvent, StreamOptions, StreamingSession};
use crate::trait_::{SpeechToText, TranscribeOptions};
use crate::transcript::{Segment, Transcript};

/// CAPS const for the mock backend. Mirrors what a "fully featured"
/// cloud backend looks like, so tests that gate on capabilities still
/// exercise the gating logic.
pub const MOCK_CAPS: Capabilities = Capabilities {
    batch: true,
    streaming_push: true,
    realtime_microphone: true,
    diarization: DiarizationSupport::SpeakerCount,
    word_timestamps: true,
    utterance_timestamps: true,
    language_detection: true,
    languages: Languages::All,
    punctuation: true,
    profanity_filter: false,
    max_audio_secs: None,
    max_concurrent_streams: Some(8),
    real_time_factor: Some(0.0),
    requires_network: false,
    supported_audio_formats: &[
        AudioFormat::Wav,
        AudioFormat::Mp3,
        AudioFormat::Flac,
        AudioFormat::Ogg,
    ],
    min_chunk_ms: Some(20),
    partial_results: true,
    redaction: false,
    vad_endpointing: true,
    custom_vocabulary: false,
    cost_per_audio_min_usd: Some(0.0),
};

/// Deterministic mock STT. `transcribe` returns a transcript whose
/// text is a hash digest of the input length so tests can assert on
/// stability without caring about the exact string.
pub struct MockSpeechToText {
    fixed_text: Option<String>,
    detected_language: Option<String>,
}

impl Default for MockSpeechToText {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSpeechToText {
    pub fn new() -> Self {
        Self {
            fixed_text: None,
            detected_language: Some("en".into()),
        }
    }

    /// Pin the returned transcript text. Used by integration tests
    /// that want to assert on a known string.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.fixed_text = Some(text.into());
        self
    }

    pub fn with_language(mut self, bcp47: impl Into<String>) -> Self {
        self.detected_language = Some(bcp47.into());
        self
    }

    fn text_for(&self, input_len: usize) -> String {
        if let Some(t) = &self.fixed_text {
            return t.clone();
        }
        format!("mock transcript ({input_len} bytes)")
    }
}

#[async_trait]
impl SpeechToText for MockSpeechToText {
    fn capabilities(&self) -> &'static Capabilities {
        &MOCK_CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::Custom(std::borrow::Cow::Borrowed("mock"))
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::LocalModel
    }

    async fn transcribe(&self, input: AudioInput, opts: TranscribeOptions) -> Result<Transcript> {
        let (input_bytes, dur) = match &input {
            AudioInput::File(path) => {
                let meta = tokio::fs::metadata(path).await?;
                (meta.len() as usize, 0.5)
            }
            AudioInput::Bytes { data, .. } => (data.len(), 0.5),
            AudioInput::Pcm(p) => (p.samples.len(), p.duration_secs()),
        };
        let text = self.text_for(input_bytes);
        let mut t = Transcript::from_text(text, BackendKind::Custom("mock".into()), dur);
        t.language = opts.language.or_else(|| self.detected_language.clone());
        t.model_id = Some("mock-stt-1".into());
        Ok(t)
    }

    async fn open_stream(&self, _opts: StreamOptions) -> Result<Box<dyn StreamingSession>> {
        Ok(Box::new(MockStreamingSession::new(self.text_for(0))))
    }
}

/// In-memory streaming session that emits a single Final segment on
/// `finish()`. Useful for FFI shape tests.
pub struct MockStreamingSession {
    text: String,
    pushed: usize,
    finished: bool,
    queue: tokio::sync::mpsc::UnboundedReceiver<std::result::Result<StreamEvent, SttError>>,
    tx: tokio::sync::mpsc::UnboundedSender<std::result::Result<StreamEvent, SttError>>,
}

impl MockStreamingSession {
    fn new(text: String) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            text,
            pushed: 0,
            finished: false,
            queue: rx,
            tx,
        }
    }
}

#[async_trait]
impl StreamingSession for MockStreamingSession {
    fn capabilities(&self) -> &'static Capabilities {
        &MOCK_CAPS
    }

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        self.pushed += chunk.len();
        // Emit a partial every push so consumers can verify the
        // event channel works.
        let _ = self.tx.send(Ok(StreamEvent::Partial {
            text: format!("(partial {})", self.pushed),
            start_ms: 0,
            end_ms: 0,
            words: Vec::new(),
        }));
        Ok(())
    }

    async fn finish(&mut self) -> Result<()> {
        if !self.finished {
            self.finished = true;
            let segment = Segment {
                text: self.text.clone(),
                start_ms: 0,
                end_ms: 0,
                words: Vec::new(),
                speaker: None,
                confidence: Some(1.0),
            };
            let _ = self.tx.send(Ok(StreamEvent::Final { segment }));
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        // Drop the sender by replacing with a fresh disconnected one.
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        self.tx = tx;
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<StreamEvent, SttError>> + Send + '_>> {
        let stream = futures::stream::poll_fn(move |cx| self.queue.poll_recv(cx));
        Box::pin(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn batch_returns_deterministic_text() {
        let m = MockSpeechToText::new();
        let bytes = Bytes::from_static(b"abcdef");
        let t = m
            .transcribe(
                AudioInput::Bytes {
                    data: bytes,
                    format: AudioFormat::Wav,
                },
                TranscribeOptions::default(),
            )
            .await
            .unwrap();
        assert!(t.text.contains("6 bytes"));
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.backend.as_str(), "mock");
    }

    #[tokio::test]
    async fn stream_emits_partial_then_final() {
        use futures::StreamExt;
        let m = MockSpeechToText::new().with_text("hello world");
        let mut s = m.open_stream(StreamOptions::default()).await.unwrap();
        s.push_audio(Bytes::from_static(b"chunk1")).await.unwrap();
        s.finish().await.unwrap();
        let mut stream = s.events();
        let first = stream.next().await.unwrap().unwrap();
        match first {
            StreamEvent::Partial { text, .. } => assert!(text.contains("partial")),
            other => panic!("expected partial, got {other:?}"),
        }
        let second = stream.next().await.unwrap().unwrap();
        match second {
            StreamEvent::Final { segment } => assert_eq!(segment.text, "hello world"),
            other => panic!("expected final, got {other:?}"),
        }
    }

    #[test]
    fn caps_serialize_to_json() {
        let v = serde_json::to_value(&MOCK_CAPS).unwrap();
        assert_eq!(v["batch"], true);
        assert_eq!(v["streaming_push"], true);
        assert_eq!(v["diarization"], "speaker_count");
    }
}
