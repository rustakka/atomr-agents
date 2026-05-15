//! WebSocket streaming session for AssemblyAI Universal-Streaming.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{Capabilities, Result, Segment, StreamEvent, StreamingSession, SttError, Word};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::caps::CAPS;
use crate::wire::{StreamingMessage, TurnMessage};

pub(crate) struct AssemblyStreamingSession {
    audio_tx: mpsc::Sender<Bytes>,
    events_rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<StreamEvent, SttError>>>>>,
    _task: tokio::task::JoinHandle<()>,
    closed: bool,
}

impl AssemblyStreamingSession {
    pub(crate) fn spawn(
        ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (audio_tx, mut audio_rx) = mpsc::channel::<Bytes>(64);
        let (events_tx, events_rx) = mpsc::channel::<std::result::Result<StreamEvent, SttError>>(64);
        let (mut sink, mut source) = ws.split();

        let task = tokio::spawn(async move {
            let send = tokio::spawn(async move {
                while let Some(chunk) = audio_rx.recv().await {
                    if chunk.is_empty() {
                        let _ = sink
                            .send(Message::Text(serde_json::json!({"type":"Terminate"}).to_string()))
                            .await;
                        let _ = sink.close().await;
                        return;
                    }
                    if sink.send(Message::Binary(chunk.to_vec())).await.is_err() {
                        return;
                    }
                }
                let _ = sink.close().await;
            });

            while let Some(msg) = source.next().await {
                match msg {
                    Ok(Message::Text(text)) => match serde_json::from_str::<StreamingMessage>(&text) {
                        Ok(StreamingMessage::Begin { id, .. }) => {
                            let _ = events_tx
                                .send(Ok(StreamEvent::Metadata(serde_json::json!({
                                    "session_id": id,
                                }))))
                                .await;
                        }
                        Ok(StreamingMessage::Turn(t)) => {
                            for ev in lift_turn(t) {
                                if events_tx.send(Ok(ev)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(StreamingMessage::Termination { .. }) => break,
                        Ok(StreamingMessage::Other) => {}
                        Err(e) => {
                            let _ = events_tx
                                .send(Err(SttError::transport(format!("ws parse: {e}"))))
                                .await;
                        }
                    },
                    Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Frame(_)) => {}
                    Err(e) => {
                        let _ = events_tx
                            .send(Err(SttError::transport(format!("ws recv: {e}"))))
                            .await;
                        break;
                    }
                }
            }
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

fn lift_turn(t: TurnMessage) -> Vec<StreamEvent> {
    let words: Vec<Word> = t
        .words
        .iter()
        .map(|w| Word {
            text: w.text.clone(),
            start_ms: w.start,
            end_ms: w.end,
            confidence: w.confidence,
        })
        .collect();
    let start_ms = words.first().map(|w| w.start_ms).unwrap_or(0);
    let end_ms = words.last().map(|w| w.end_ms).unwrap_or(start_ms);

    if t.end_of_turn {
        let mut out = Vec::with_capacity(2);
        out.push(StreamEvent::Final {
            segment: Segment {
                text: t.transcript,
                start_ms,
                end_ms,
                words,
                speaker: None,
                confidence: t.end_of_turn_confidence,
            },
        });
        out.push(StreamEvent::UtteranceEnd { at_ms: end_ms });
        out
    } else {
        vec![StreamEvent::Partial {
            text: t.transcript,
            start_ms,
            end_ms,
            words,
        }]
    }
}

#[async_trait]
impl StreamingSession for AssemblyStreamingSession {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    async fn push_audio(&mut self, chunk: Bytes) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.audio_tx
            .send(chunk)
            .await
            .map_err(|_| SttError::transport("assemblyai: stream closed"))
    }

    async fn finish(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            let _ = self.audio_tx.send(Bytes::new()).await;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.closed = true;
        let (tx, _) = mpsc::channel::<Bytes>(1);
        self.audio_tx = tx;
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<StreamEvent, SttError>> + Send + '_>> {
        let mut guard = self.events_rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx))),
            None => Box::pin(futures::stream::empty()),
        }
    }
}
