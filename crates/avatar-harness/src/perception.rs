//! Perception actor — surfaces transcribed utterances to the harness.
//!
//! Two ingestion modes are supported:
//!
//! 1. **Manual text** — callers push pre-transcribed text via
//!    [`PerceptionActor::push_text`]. Useful when the avatar is driven
//!    from a chat UI or webhook.
//! 2. **STT-driven** — when wired to an
//!    [`atomr_agents_stt_core::SpeechToText`] backend, the harness
//!    forwards mic audio chunks to it and pushes transcripts onto the
//!    same outbound channel. The STT wiring lives in the *caller*
//!    (the harness builder accepts a `DynSpeechToText`); this actor
//!    just multiplexes everyone onto the same `mpsc::Sender<String>`.

use tokio::sync::mpsc;

/// One transcribed user utterance the harness should react to.
#[derive(Debug, Clone)]
pub struct Utterance {
    pub text: String,
    /// Optional per-speaker label (useful for diarized streams).
    pub speaker: Option<String>,
}

impl Utterance {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            speaker: None,
        }
    }
}

/// Thin wrapper around an mpsc sender so the public API is clearer
/// than a raw channel handle.
#[derive(Clone)]
pub struct PerceptionActor {
    tx: mpsc::Sender<Utterance>,
}

impl PerceptionActor {
    pub fn new(tx: mpsc::Sender<Utterance>) -> Self {
        Self { tx }
    }

    /// Push a pre-transcribed utterance onto the perception channel.
    /// Returns an error if the channel is closed (harness shut down).
    pub async fn push_text(&self, text: impl Into<String>) -> Result<(), &'static str> {
        self.tx
            .send(Utterance::from_text(text))
            .await
            .map_err(|_| "perception channel closed")
    }

    pub async fn push(&self, utterance: Utterance) -> Result<(), &'static str> {
        self.tx
            .send(utterance)
            .await
            .map_err(|_| "perception channel closed")
    }
}
