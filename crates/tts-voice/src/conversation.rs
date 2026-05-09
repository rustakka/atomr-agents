use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError};
use atomr_agents_tts_core::{
    AudioChunk, DynTextToSpeech, RealtimeEvent, RealtimeOptions, RealtimeSession, SynthesisRequest,
    VoiceRef,
};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationMode {
    UnifiedRealtime,
    TurnBased,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationOptions {
    #[serde(default)]
    pub voice_id: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

#[async_trait]
pub trait ConversationAgent: Send + Sync {
    async fn respond(&self, user_turn: &str) -> Result<String>;
}

pub struct NoopAgent;

#[async_trait]
impl ConversationAgent for NoopAgent {
    async fn respond(&self, user_turn: &str) -> Result<String> {
        Ok(format!("(echo) {user_turn}"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationEvent {
    UserSpoke { text: String, is_final: bool },
    AssistantText { text: String, is_final: bool },
    AssistantAudio { chunk: AudioChunk },
    Interrupted,
    Done,
}

/// Bidirectional voice conversation.
///
/// - `open_turn_based(tts, agent)` — caller pushes finalised user
///   turns via [`Conversation::push_text`] (typically driven by an
///   external STT pipeline). The session runs them through the agent
///   and synthesises the reply via the TTS backend.
/// - `open_unified_realtime(tts, agent, opts)` — opens a realtime
///   session against a backend that handles both directions
///   (OpenAI Realtime, Gemini Live, ElevenLabs ConvAI). Caller drives
///   audio in via [`Conversation::push_audio`]; the session forwards
///   transcripts and assistant audio to [`Conversation::events`].
pub struct Conversation {
    mode: ConversationMode,
    realtime: Option<Arc<tokio::sync::Mutex<Box<dyn RealtimeSession>>>>,
    turn: Option<TurnState>,
    events_rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<ConversationEvent, SttError>>>>>,
    events_tx: mpsc::Sender<std::result::Result<ConversationEvent, SttError>>,
    _forward_task: Option<tokio::task::JoinHandle<()>>,
    closed: bool,
}

struct TurnState {
    tts: DynTextToSpeech,
    voice_id: Option<String>,
    agent: Arc<dyn ConversationAgent>,
}

impl Conversation {
    pub async fn open_unified_realtime(
        tts: DynTextToSpeech,
        agent: Arc<dyn ConversationAgent>,
        opts: ConversationOptions,
    ) -> Result<Self> {
        let realtime_opts = RealtimeOptions {
            voice_id: opts.voice_id.clone(),
            instructions: opts.instructions.clone(),
            language: opts.language.clone(),
            temperature: None,
            extra: opts.extra.clone(),
        };
        let session = tts.open_realtime(realtime_opts).await?;
        let realtime = Arc::new(tokio::sync::Mutex::new(session));
        let (events_tx, events_rx) =
            mpsc::channel::<std::result::Result<ConversationEvent, SttError>>(64);

        let realtime_for_task = realtime.clone();
        let events_tx_for_task = events_tx.clone();
        let agent_for_task = agent.clone();
        let forward_task = tokio::spawn(async move {
            forward_realtime(realtime_for_task, events_tx_for_task, agent_for_task).await;
        });

        Ok(Self {
            mode: ConversationMode::UnifiedRealtime,
            realtime: Some(realtime),
            turn: None,
            events_rx: Arc::new(Mutex::new(Some(events_rx))),
            events_tx,
            _forward_task: Some(forward_task),
            closed: false,
        })
    }

    pub fn open_turn_based(
        tts: DynTextToSpeech,
        agent: Arc<dyn ConversationAgent>,
        opts: ConversationOptions,
    ) -> Self {
        let (events_tx, events_rx) =
            mpsc::channel::<std::result::Result<ConversationEvent, SttError>>(64);
        Self {
            mode: ConversationMode::TurnBased,
            realtime: None,
            turn: Some(TurnState {
                tts,
                voice_id: opts.voice_id,
                agent,
            }),
            events_rx: Arc::new(Mutex::new(Some(events_rx))),
            events_tx,
            _forward_task: None,
            closed: false,
        }
    }

    pub fn mode(&self) -> ConversationMode { self.mode }

    pub async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        if let Some(rt) = &self.realtime {
            let mut guard = rt.lock().await;
            guard.push_audio(chunk).await
        } else {
            Ok(())
        }
    }

    pub async fn push_text(&mut self, text: &str) -> Result<()> {
        if let Some(rt) = &self.realtime {
            let mut guard = rt.lock().await;
            return guard.push_text(text).await;
        }
        if let Some(turn) = &self.turn {
            let _ = self
                .events_tx
                .send(Ok(ConversationEvent::UserSpoke {
                    text: text.to_string(),
                    is_final: true,
                }))
                .await;
            let reply = turn.agent.respond(text).await?;
            let _ = self
                .events_tx
                .send(Ok(ConversationEvent::AssistantText {
                    text: reply.clone(),
                    is_final: true,
                }))
                .await;
            let voice = turn
                .voice_id
                .clone()
                .map(|id| VoiceRef::Library { id })
                .unwrap_or(VoiceRef::Library { id: "default".to_string() });
            let req = SynthesisRequest::tts(reply, voice);
            let out = turn.tts.synthesize(req).await?;
            let chunk_bytes = out
                .container_bytes
                .clone()
                .unwrap_or_else(|| {
                    // Pack the f32 PCM samples as raw little-endian
                    // i16 so downstream consumers have a portable
                    // wire shape. The underlying format is on
                    // out.format.
                    let mut buf = Vec::with_capacity(out.audio.samples.len() * 2);
                    for s in &out.audio.samples {
                        let q = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        buf.extend_from_slice(&q.to_le_bytes());
                    }
                    Bytes::from(buf)
                });
            let _ = self
                .events_tx
                .send(Ok(ConversationEvent::AssistantAudio {
                    chunk: AudioChunk {
                        bytes: chunk_bytes,
                        seq: 0,
                        is_final: true,
                        words: Vec::new(),
                    },
                }))
                .await;
            let _ = self.events_tx.send(Ok(ConversationEvent::Done)).await;
        }
        Ok(())
    }

    pub async fn interrupt(&mut self) -> Result<()> {
        if let Some(rt) = &self.realtime {
            let mut guard = rt.lock().await;
            return guard.interrupt().await;
        }
        Ok(())
    }

    pub async fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            if let Some(rt) = &self.realtime {
                let mut guard = rt.lock().await;
                let _ = guard.close().await;
            }
        }
        Ok(())
    }

    pub fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<ConversationEvent, SttError>> + Send + '_>>
    {
        let mut guard = self.events_rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx))),
            None => Box::pin(futures::stream::empty()),
        }
    }
}

async fn forward_realtime(
    realtime: Arc<tokio::sync::Mutex<Box<dyn RealtimeSession>>>,
    events_tx: mpsc::Sender<std::result::Result<ConversationEvent, SttError>>,
    agent: Arc<dyn ConversationAgent>,
) {
    use futures::StreamExt;
    // Pull the inbound event stream once and drain it, holding the
    // session lock for the lifetime of the iterator.
    let mut guard = realtime.lock().await;
    let mut events = guard.events();
    while let Some(item) = events.next().await {
        match item {
            Ok(RealtimeEvent::AudioOut { chunk }) => {
                if events_tx
                    .send(Ok(ConversationEvent::AssistantAudio { chunk }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(RealtimeEvent::OutboundText { text, is_final }) => {
                if events_tx
                    .send(Ok(ConversationEvent::AssistantText { text, is_final }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(RealtimeEvent::InboundTranscript { text, is_final }) => {
                if events_tx
                    .send(Ok(ConversationEvent::UserSpoke {
                        text: text.clone(),
                        is_final,
                    }))
                    .await
                    .is_err()
                {
                    break;
                }
                if is_final {
                    let _ = agent.respond(&text).await;
                }
            }
            Ok(RealtimeEvent::BargeIn) => {
                let _ = events_tx.send(Ok(ConversationEvent::Interrupted)).await;
            }
            Ok(RealtimeEvent::Done) => {
                let _ = events_tx.send(Ok(ConversationEvent::Done)).await;
                break;
            }
            Ok(_) => {}
            Err(e) => {
                let _ = events_tx.send(Err(e)).await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_tts_core::MockTextToSpeech;
    use std::sync::Arc;

    #[tokio::test]
    async fn turn_based_runs_user_turn_through_agent_and_tts() {
        use futures::StreamExt;
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let agent: Arc<dyn ConversationAgent> = Arc::new(NoopAgent);
        let mut conv = Conversation::open_turn_based(tts, agent, ConversationOptions::default());

        conv.push_text("hi there").await.unwrap();

        let mut events = conv.events();
        let mut got_user = false;
        let mut got_text = false;
        let mut got_audio = false;
        let mut got_done = false;
        while let Some(item) = events.next().await {
            match item.unwrap() {
                ConversationEvent::UserSpoke { text, .. } => {
                    assert_eq!(text, "hi there");
                    got_user = true;
                }
                ConversationEvent::AssistantText { text, .. } => {
                    assert!(text.starts_with("(echo)"));
                    got_text = true;
                }
                ConversationEvent::AssistantAudio { .. } => got_audio = true,
                ConversationEvent::Done => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_user && got_text && got_audio && got_done);
    }

    #[tokio::test]
    async fn mode_introspection_works() {
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let conv = Conversation::open_turn_based(
            tts,
            Arc::new(NoopAgent),
            ConversationOptions::default(),
        );
        assert_eq!(conv.mode(), ConversationMode::TurnBased);
    }
}
