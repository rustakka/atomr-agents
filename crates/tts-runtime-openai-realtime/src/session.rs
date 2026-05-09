//! `OpenAiRealtimeSession` — bidirectional realtime over WebSocket.
//!
//! Wire format reference:
//! <https://platform.openai.com/docs/api-reference/realtime>.
//!
//! Client→Server events used here:
//! - `session.update`            — set voice/instructions/modalities.
//! - `input_audio_buffer.append` — `{audio: <base64 PCM>}`.
//! - `input_audio_buffer.commit` — close current user turn.
//! - `conversation.item.create` — inject a text turn.
//! - `response.create`           — request the assistant respond.
//! - `response.cancel`           — barge-in.
//!
//! Server→Client events parsed here:
//! - `response.audio.delta`           — base64 PCM chunk.
//! - `response.audio_transcript.delta`— assistant transcript.
//! - `response.audio.done` / `response.done`.
//! - `input_audio_buffer.speech_started`/`stopped`.
//! - `conversation.item.input_audio_transcription.completed`.
//! - `error`.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{Result, SttError};
use atomr_agents_tts_core::{
    AudioChunk, Capabilities, RealtimeEvent, RealtimeSession,
};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::caps::CAPS;

pub struct OpenAiRealtimeSession {
    out_tx: mpsc::Sender<OutgoingMsg>,
    events_rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<RealtimeEvent, SttError>>>>>,
    _task: tokio::task::JoinHandle<()>,
    closed: bool,
}

#[derive(Debug)]
enum OutgoingMsg {
    Text(String),
    Close,
}

impl OpenAiRealtimeSession {
    pub fn spawn(
        ws: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        voice: String,
        instructions: Option<String>,
        modalities: Vec<String>,
    ) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (out_tx, mut out_rx) = mpsc::channel::<OutgoingMsg>(64);
        let (events_tx, events_rx) =
            mpsc::channel::<std::result::Result<RealtimeEvent, SttError>>(64);
        let (mut sink, mut source) = ws.split();

        // Seed session.update on connect.
        let session_update = serde_json::json!({
            "type": "session.update",
            "session": {
                "voice": voice,
                "modalities": modalities,
                "instructions": instructions.unwrap_or_default(),
            }
        });
        let out_tx_seed = out_tx.clone();
        tokio::spawn(async move {
            let _ = out_tx_seed
                .send(OutgoingMsg::Text(session_update.to_string()))
                .await;
        });

        let task = tokio::spawn(async move {
            let send = tokio::spawn(async move {
                while let Some(msg) = out_rx.recv().await {
                    match msg {
                        OutgoingMsg::Text(t) => {
                            if sink.send(Message::Text(t)).await.is_err() {
                                return;
                            }
                        }
                        OutgoingMsg::Close => {
                            let _ = sink.close().await;
                            return;
                        }
                    }
                }
                let _ = sink.close().await;
            });

            let mut seq = 0u64;
            while let Some(msg) = source.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<ServerEvent>(&text) {
                            Ok(ServerEvent::AudioDelta { delta, .. }) => {
                                let bytes = base64_decode(&delta).unwrap_or_default();
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::AudioOut {
                                        chunk: AudioChunk {
                                            bytes: Bytes::from(bytes),
                                            seq,
                                            is_final: false,
                                            words: Vec::new(),
                                        },
                                    }))
                                    .await;
                                seq += 1;
                            }
                            Ok(ServerEvent::AudioTranscriptDelta { delta, .. }) => {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::OutboundText {
                                        text: delta,
                                        is_final: false,
                                    }))
                                    .await;
                            }
                            Ok(ServerEvent::InputTranscriptCompleted { transcript, .. }) => {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::InboundTranscript {
                                        text: transcript,
                                        is_final: true,
                                    }))
                                    .await;
                            }
                            Ok(ServerEvent::SpeechStarted { .. }) => {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::UserSpeechStarted))
                                    .await;
                            }
                            Ok(ServerEvent::SpeechStopped { .. }) => {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::UserSpeechEnded))
                                    .await;
                            }
                            Ok(ServerEvent::ResponseCancelled { .. }) => {
                                let _ = events_tx.send(Ok(RealtimeEvent::BargeIn)).await;
                            }
                            Ok(ServerEvent::ResponseDone { .. })
                            | Ok(ServerEvent::AudioDone { .. }) => {
                                // Per-turn done; we surface the global Done
                                // when the WS closes.
                            }
                            Ok(ServerEvent::Error { error }) => {
                                let _ = events_tx
                                    .send(Err(SttError::Backend {
                                        status: 0,
                                        message: format!("openai realtime: {}", error.message),
                                    }))
                                    .await;
                            }
                            Ok(ServerEvent::Other) => {}
                            Err(e) => {
                                let _ = events_tx
                                    .send(Err(SttError::transport(format!(
                                        "openai realtime parse: {e}"
                                    ))))
                                    .await;
                            }
                        }
                    }
                    Ok(Message::Binary(_))
                    | Ok(Message::Ping(_))
                    | Ok(Message::Pong(_))
                    | Ok(Message::Frame(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        let _ = events_tx
                            .send(Err(SttError::transport(format!("openai realtime recv: {e}"))))
                            .await;
                        break;
                    }
                }
            }
            let _ = events_tx.send(Ok(RealtimeEvent::Done)).await;
            let _ = send.await;
        });

        Self {
            out_tx,
            events_rx: Arc::new(Mutex::new(Some(events_rx))),
            _task: task,
            closed: false,
        }
    }
}

#[async_trait]
impl RealtimeSession for OpenAiRealtimeSession {
    fn capabilities(&self) -> &'static Capabilities { &CAPS }

    async fn push_text(&mut self, text: &str) -> Result<()> {
        let item = serde_json::json!({
            "type": "conversation.item.create",
            "item": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            }
        });
        self.out_tx
            .send(OutgoingMsg::Text(item.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))?;
        let resp = serde_json::json!({"type": "response.create"});
        self.out_tx
            .send(OutgoingMsg::Text(resp.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))
    }

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        let payload = serde_json::json!({
            "type": "input_audio_buffer.append",
            "audio": base64_encode(&chunk),
        });
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))
    }

    async fn commit_input(&mut self) -> Result<()> {
        let payload = serde_json::json!({"type": "input_audio_buffer.commit"});
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))?;
        let resp = serde_json::json!({"type": "response.create"});
        self.out_tx
            .send(OutgoingMsg::Text(resp.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))
    }

    async fn interrupt(&mut self) -> Result<()> {
        let payload = serde_json::json!({"type": "response.cancel"});
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("openai realtime: stream closed"))
    }

    async fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            let _ = self.out_tx.send(OutgoingMsg::Close).await;
        }
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<RealtimeEvent, SttError>> + Send + '_>>
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

// ----- wire shapes ----------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ServerEvent {
    #[serde(rename = "response.audio.delta")]
    AudioDelta {
        delta: String,
        #[allow(dead_code)]
        response_id: Option<String>,
    },
    #[serde(rename = "response.audio_transcript.delta")]
    AudioTranscriptDelta {
        delta: String,
        #[allow(dead_code)]
        response_id: Option<String>,
    },
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    InputTranscriptCompleted {
        transcript: String,
        #[allow(dead_code)]
        item_id: Option<String>,
    },
    #[serde(rename = "input_audio_buffer.speech_started")]
    SpeechStarted {
        #[allow(dead_code)]
        item_id: Option<String>,
    },
    #[serde(rename = "input_audio_buffer.speech_stopped")]
    SpeechStopped {
        #[allow(dead_code)]
        item_id: Option<String>,
    },
    #[serde(rename = "response.cancelled")]
    ResponseCancelled {
        #[allow(dead_code)]
        response_id: Option<String>,
    },
    #[serde(rename = "response.done")]
    ResponseDone {
        #[allow(dead_code)]
        response: Option<serde_json::Value>,
    },
    #[serde(rename = "response.audio.done")]
    AudioDone {
        #[allow(dead_code)]
        response_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error { error: ErrorBody },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: String,
    #[allow(dead_code)]
    #[serde(default)]
    code: Option<String>,
}

// ----- base64 helpers (copy-local to avoid cross-crate coupling) ------------

fn base64_encode(b: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((b.len() + 2) / 3 * 4);
    for chunk in b.chunks(3) {
        let n = chunk.len();
        let b0 = chunk[0] as u32;
        let b1 = if n > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if n > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if n > 1 { out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char); } else { out.push('='); }
        if n > 2 { out.push(ALPHABET[(triple & 0x3F) as usize] as char); } else { out.push('='); }
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut buf = [0u32; 4];
        let mut pad = 0;
        for (i, c) in chunk.iter().enumerate() {
            if *c == b'=' {
                pad += 1;
                continue;
            }
            buf[i] = val(*c)
                .ok_or_else(|| SttError::decode("invalid base64 character"))?;
        }
        let triple = (buf[0] << 18) | (buf[1] << 12) | (buf[2] << 6) | buf[3];
        out.push(((triple >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((triple & 0xFF) as u8);
        }
    }
    Ok(out)
}
