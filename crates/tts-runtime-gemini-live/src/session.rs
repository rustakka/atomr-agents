//! `GeminiLiveSession` — bidirectional realtime over WebSocket.
//!
//! Wire format reference:
//! <https://ai.google.dev/api/live>.
//!
//! Client→Server messages used here:
//! - `setup` (sent once on connect): `{model, generation_config{response_modalities,
//!   speech_config{voice_config{prebuilt_voice_config{voice_name}}}}, system_instruction}`.
//! - `client_content`: `{turns: [{role: "user", parts: [{text}]}], turn_complete: true}`.
//! - `realtime_input`: `{media_chunks: [{mime_type: "audio/pcm;rate=16000", data: <b64>}]}`.
//!
//! Server→Client messages parsed here:
//! - `serverContent.modelTurn.parts[].inlineData.data` (b64 PCM out)
//! - `serverContent.modelTurn.parts[].text` (assistant text)
//! - `serverContent.inputTranscription.text`
//! - `serverContent.outputTranscription.text`
//! - `serverContent.interrupted`
//! - `serverContent.turnComplete`

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

pub struct GeminiLiveSession {
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

impl GeminiLiveSession {
    pub fn spawn(
        ws: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        model: String,
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

        // Setup message (always first).
        let mut setup = serde_json::json!({
            "setup": {
                "model": model,
                "generation_config": {
                    "response_modalities": modalities,
                    "speech_config": {
                        "voice_config": {
                            "prebuilt_voice_config": { "voice_name": voice }
                        }
                    }
                }
            }
        });
        if let Some(instr) = instructions {
            setup["setup"]["system_instruction"] = serde_json::json!({
                "parts": [{"text": instr}]
            });
        }

        let out_tx_seed = out_tx.clone();
        tokio::spawn(async move {
            let _ = out_tx_seed.send(OutgoingMsg::Text(setup.to_string())).await;
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
                        emit_events(&text, &events_tx, &mut seq).await;
                    }
                    Ok(Message::Binary(b)) => {
                        // Some Gemini revisions send setup confirmation as binary; ignore.
                        let _ = String::from_utf8(b);
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        let _ = events_tx
                            .send(Err(SttError::transport(format!("gemini live recv: {e}"))))
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

async fn emit_events(
    text: &str,
    events_tx: &mpsc::Sender<std::result::Result<RealtimeEvent, SttError>>,
    seq: &mut u64,
) {
    let parsed: ServerEnvelope = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(content) = parsed.server_content {
        if let Some(turn) = content.model_turn {
            for part in turn.parts {
                if let Some(inline) = part.inline_data {
                    if inline.mime_type.starts_with("audio/") {
                        let bytes = base64_decode(&inline.data).unwrap_or_default();
                        let _ = events_tx
                            .send(Ok(RealtimeEvent::AudioOut {
                                chunk: AudioChunk {
                                    bytes: Bytes::from(bytes),
                                    seq: *seq,
                                    is_final: false,
                                    words: Vec::new(),
                                },
                            }))
                            .await;
                        *seq += 1;
                    }
                }
                if let Some(text) = part.text {
                    let _ = events_tx
                        .send(Ok(RealtimeEvent::OutboundText {
                            text,
                            is_final: false,
                        }))
                        .await;
                }
            }
        }
        if let Some(input) = content.input_transcription {
            let _ = events_tx
                .send(Ok(RealtimeEvent::InboundTranscript {
                    text: input.text,
                    is_final: true,
                }))
                .await;
        }
        if let Some(output) = content.output_transcription {
            let _ = events_tx
                .send(Ok(RealtimeEvent::OutboundText {
                    text: output.text,
                    is_final: true,
                }))
                .await;
        }
        if content.interrupted == Some(true) {
            let _ = events_tx.send(Ok(RealtimeEvent::BargeIn)).await;
        }
    }
}

#[async_trait]
impl RealtimeSession for GeminiLiveSession {
    fn capabilities(&self) -> &'static Capabilities { &CAPS }

    async fn push_text(&mut self, text: &str) -> Result<()> {
        let payload = serde_json::json!({
            "client_content": {
                "turns": [{"role": "user", "parts": [{"text": text}]}],
                "turn_complete": true,
            }
        });
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("gemini live: stream closed"))
    }

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        let payload = serde_json::json!({
            "realtime_input": {
                "media_chunks": [{
                    "mime_type": "audio/pcm;rate=16000",
                    "data": base64_encode(&chunk),
                }]
            }
        });
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("gemini live: stream closed"))
    }

    async fn commit_input(&mut self) -> Result<()> {
        // Gemini Live infers turn boundaries from VAD; no explicit commit.
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<()> {
        // Sending fresh client_content with turn_complete=true cancels
        // the current model turn server-side.
        let payload = serde_json::json!({
            "client_content": {"turns": [], "turn_complete": true}
        });
        self.out_tx
            .send(OutgoingMsg::Text(payload.to_string()))
            .await
            .map_err(|_| SttError::transport("gemini live: stream closed"))
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
#[serde(rename_all = "camelCase")]
struct ServerEnvelope {
    #[serde(default)]
    server_content: Option<ServerContent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerContent {
    #[serde(default)]
    model_turn: Option<ModelTurn>,
    #[serde(default)]
    input_transcription: Option<TranscriptionPart>,
    #[serde(default)]
    output_transcription: Option<TranscriptionPart>,
    #[serde(default)]
    interrupted: Option<bool>,
    #[allow(dead_code)]
    #[serde(default)]
    turn_complete: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ModelTurn {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    #[serde(default)]
    inline_data: Option<InlineData>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct TranscriptionPart {
    text: String,
}

// ----- base64 helpers (copy-local) ------------------------------------------

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
        if pad < 2 { out.push(((triple >> 8) & 0xFF) as u8); }
        if pad < 1 { out.push((triple & 0xFF) as u8); }
    }
    Ok(out)
}
