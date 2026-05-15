//! ElevenLabs streaming sessions:
//!
//! - `ElevenLabsHttpStream` wraps the chunked HTTP response from
//!   `POST /v1/text-to-speech/{voice}/stream`.
//! - `ElevenLabsConvaiSession` wraps the Conversational AI WS for
//!   `open_realtime`.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, Result, SttError};
use atomr_agents_tts_core::{AudioChunk, Capabilities, RealtimeEvent, RealtimeSession, SynthesisStream};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::caps::CAPS;

// ----- HTTP-chunked stream --------------------------------------------------

pub(crate) struct ElevenLabsHttpStream {
    rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<AudioChunk, SttError>>>>>,
    format: AudioFormat,
}

impl ElevenLabsHttpStream {
    pub(crate) fn spawn(
        body_stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + 'static,
        format: AudioFormat,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<std::result::Result<AudioChunk, SttError>>(64);
        tokio::spawn(async move {
            use futures_util::StreamExt;
            futures::pin_mut!(body_stream);
            let mut seq = 0u64;
            let mut last: Option<AudioChunk> = None;
            while let Some(res) = body_stream.next().await {
                match res {
                    Ok(bytes) => {
                        if bytes.is_empty() {
                            continue;
                        }
                        if let Some(prev) = last.take() {
                            let _ = tx.send(Ok(prev)).await;
                        }
                        last = Some(AudioChunk {
                            bytes,
                            seq,
                            is_final: false,
                            words: Vec::new(),
                        });
                        seq += 1;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(SttError::transport(format!("elevenlabs stream: {e}"))))
                            .await;
                        return;
                    }
                }
            }
            if let Some(mut c) = last {
                c.is_final = true;
                let _ = tx.send(Ok(c)).await;
            } else {
                let _ = tx
                    .send(Ok(AudioChunk {
                        bytes: Bytes::new(),
                        seq,
                        is_final: true,
                        words: Vec::new(),
                    }))
                    .await;
            }
        });
        Self {
            rx: Arc::new(Mutex::new(Some(rx))),
            format,
        }
    }
}

#[async_trait]
impl SynthesisStream for ElevenLabsHttpStream {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }
    fn format(&self) -> &AudioFormat {
        &self.format
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<AudioChunk, SttError>> + Send + '_>> {
        let mut guard = self.rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx))),
            None => Box::pin(futures::stream::empty()),
        }
    }

    async fn close(&mut self) -> Result<()> {
        let mut g = self.rx.lock();
        *g = None;
        Ok(())
    }
}

// ----- Conversational AI WS -------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub(crate) enum ConvaiMessage {
    #[serde(rename = "conversation_initiation_metadata")]
    InitMeta { conversation_id: Option<String> },
    #[serde(rename = "audio")]
    Audio { audio_event: AudioEvent },
    #[serde(rename = "user_transcript")]
    UserTranscript {
        user_transcription_event: TranscriptEvent,
    },
    #[serde(rename = "agent_response")]
    AgentResponse { agent_response_event: TextEvent },
    #[serde(rename = "agent_response_correction")]
    AgentCorrection {
        agent_response_correction_event: TextEvent,
    },
    #[serde(rename = "interruption")]
    Interruption,
    #[serde(rename = "ping")]
    Ping { ping_event: serde_json::Value },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AudioEvent {
    pub audio_base_64: String,
    #[allow(dead_code)]
    pub event_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptEvent {
    pub user_transcript: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextEvent {
    pub agent_response: Option<String>,
    pub corrected_response: Option<String>,
}

pub(crate) struct ElevenLabsConvaiSession {
    audio_tx: mpsc::Sender<Bytes>,
    events_rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<RealtimeEvent, SttError>>>>>,
    _task: tokio::task::JoinHandle<()>,
    closed: bool,
}

impl ElevenLabsConvaiSession {
    pub(crate) fn spawn(
        ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (audio_tx, mut audio_rx) = mpsc::channel::<Bytes>(64);
        let (events_tx, events_rx) = mpsc::channel::<std::result::Result<RealtimeEvent, SttError>>(64);
        let (mut sink, mut source) = ws.split();

        let task = tokio::spawn(async move {
            let send = tokio::spawn(async move {
                while let Some(chunk) = audio_rx.recv().await {
                    if chunk.is_empty() {
                        let _ = sink.close().await;
                        return;
                    }
                    // ConvAI expects {"user_audio_chunk": "<b64>"} text frames.
                    let b64 = base64_encode(&chunk);
                    let payload = serde_json::json!({"user_audio_chunk": b64});
                    if sink.send(Message::Text(payload.to_string())).await.is_err() {
                        return;
                    }
                }
                let _ = sink.close().await;
            });

            let mut seq = 0u64;
            while let Some(msg) = source.next().await {
                match msg {
                    Ok(Message::Text(text)) => match serde_json::from_str::<ConvaiMessage>(&text) {
                        Ok(ConvaiMessage::Audio { audio_event }) => {
                            let bytes = base64_decode(&audio_event.audio_base_64).unwrap_or_default();
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
                        Ok(ConvaiMessage::UserTranscript {
                            user_transcription_event,
                        }) => {
                            let _ = events_tx
                                .send(Ok(RealtimeEvent::InboundTranscript {
                                    text: user_transcription_event.user_transcript,
                                    is_final: true,
                                }))
                                .await;
                        }
                        Ok(ConvaiMessage::AgentResponse { agent_response_event }) => {
                            if let Some(text) = agent_response_event.agent_response {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::OutboundText { text, is_final: true }))
                                    .await;
                            }
                        }
                        Ok(ConvaiMessage::AgentCorrection {
                            agent_response_correction_event,
                        }) => {
                            if let Some(text) = agent_response_correction_event.corrected_response {
                                let _ = events_tx
                                    .send(Ok(RealtimeEvent::OutboundText { text, is_final: true }))
                                    .await;
                            }
                        }
                        Ok(ConvaiMessage::Interruption) => {
                            let _ = events_tx.send(Ok(RealtimeEvent::BargeIn)).await;
                        }
                        Ok(ConvaiMessage::InitMeta { .. })
                        | Ok(ConvaiMessage::Ping { .. })
                        | Ok(ConvaiMessage::Other) => {}
                        Err(e) => {
                            let _ = events_tx
                                .send(Err(SttError::transport(format!("convai parse: {e}"))))
                                .await;
                        }
                    },
                    Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Frame(_)) => {}
                    Err(e) => {
                        let _ = events_tx
                            .send(Err(SttError::transport(format!("convai recv: {e}"))))
                            .await;
                        break;
                    }
                }
            }
            let _ = events_tx.send(Ok(RealtimeEvent::Done)).await;
            let _ = send.await;
        });

        Self {
            audio_tx,
            events_rx: Arc::new(Mutex::new(Some(events_rx))),
            _task: task,
            closed: false,
        }
    }
}

#[async_trait]
impl RealtimeSession for ElevenLabsConvaiSession {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    async fn push_text(&mut self, _text: &str) -> Result<()> {
        // ConvAI is mic-driven; the agent responds to user audio.
        // To inject text-only turns, use the user_message API. We
        // intentionally surface the limitation via a typed error so
        // callers know to push audio instead.
        Err(SttError::UnsupportedCapability(
            "convai expects audio input; use push_audio (or convai's user_message API)",
        ))
    }

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.audio_tx
            .send(chunk)
            .await
            .map_err(|_| SttError::transport("convai: stream closed"))
    }

    async fn commit_input(&mut self) -> Result<()> {
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<()> {
        // No client-side interrupt frame in convai; the server
        // detects barge-in on incoming audio.
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            let _ = self.audio_tx.send(Bytes::new()).await;
        }
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<RealtimeEvent, SttError>> + Send + '_>> {
        let mut guard = self.events_rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx))),
            None => Box::pin(futures::stream::empty()),
        }
    }
}

// ----- base64 helpers -------------------------------------------------------

fn base64_encode(b: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((b.len() + 2) / 3 * 4);
    for chunk in b.chunks(3) {
        let n = chunk.len();
        let b0 = chunk[0] as u32;
        let b1 = if n > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if n > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if n > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if n > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
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
            buf[i] = val(*c).ok_or_else(|| SttError::decode("invalid base64 character"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_round_trip() {
        let cases: &[&[u8]] = &[b"", b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"];
        for case in cases {
            let enc = base64_encode(case);
            let dec = base64_decode(&enc).unwrap();
            assert_eq!(dec, *case, "round-trip mismatch for {:?}", case);
        }
    }
}
