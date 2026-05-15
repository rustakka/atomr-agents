//! WebSocket-backed [`StreamingSession`] for Deepgram's live
//! `wss://api.deepgram.com/v1/listen` endpoint.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{
    Capabilities, Result, Segment, SpeakerTag, StreamEvent, StreamingSession, SttError, Word,
};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::caps::CAPS;
use crate::wire::{Alternative, ResultsMessage, WsMessage};

pub(crate) struct DeepgramStreamingSession {
    /// Sender end of the audio bytes channel; the background task
    /// reads from the corresponding receiver and forwards binary
    /// frames over the WS.
    audio_tx: mpsc::Sender<Bytes>,
    /// Receiver of decoded events from the background WS task.
    events_rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<StreamEvent, SttError>>>>>,
    /// Handle to the background task — drop to abort.
    _task: tokio::task::JoinHandle<()>,
    closed: bool,
}

impl DeepgramStreamingSession {
    pub(crate) fn spawn(
        ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Self {
        let (audio_tx, mut audio_rx) = mpsc::channel::<Bytes>(64);
        let (events_tx, events_rx) = mpsc::channel::<std::result::Result<StreamEvent, SttError>>(64);

        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let (mut sink, mut source) = ws.split();

        let task = tokio::spawn(async move {
            // Forwarder: pull audio chunks from caller and push as
            // WS binary frames. When the caller drops `audio_tx`,
            // close the WS.
            let send_task = tokio::spawn(async move {
                while let Some(chunk) = audio_rx.recv().await {
                    if chunk.is_empty() {
                        // Empty chunk = signal "send Close"
                        let _ = sink
                            .send(Message::Text(
                                serde_json::json!({"type":"CloseStream"}).to_string(),
                            ))
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

            // Receiver: parse WS messages, lift to StreamEvent.
            while let Some(msg) = source.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<WsMessage>(&text) {
                            Ok(WsMessage::Results(r)) => {
                                for ev in lift_results(r) {
                                    if events_tx.send(Ok(ev)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(WsMessage::SpeechStarted(_)) => {
                                // Could surface as Metadata; skip for v1 to keep
                                // the event stream concise.
                            }
                            Ok(WsMessage::UtteranceEnd(u)) => {
                                let at_ms = (u.last_word_end.unwrap_or(0.0) * 1000.0) as u32;
                                let _ = events_tx.send(Ok(StreamEvent::UtteranceEnd { at_ms })).await;
                            }
                            Ok(WsMessage::Metadata(m)) => {
                                let v = serde_json::json!({
                                    "request_id": m.request_id,
                                    "model_info": m.model_info,
                                });
                                let _ = events_tx.send(Ok(StreamEvent::Metadata(v))).await;
                            }
                            Ok(WsMessage::Other) => {}
                            Err(e) => {
                                let _ = events_tx
                                    .send(Err(SttError::transport(format!("ws parse: {e}"))))
                                    .await;
                            }
                        }
                    }
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
            // Wait for send task to drain.
            let _ = send_task.await;
        });

        Self {
            audio_tx,
            events_rx: Arc::new(Mutex::new(Some(events_rx))),
            _task: task,
            closed: false,
        }
    }
}

fn lift_results(r: ResultsMessage) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    let is_final = r.is_final.unwrap_or(false);
    let mut prev_speaker: Option<u8> = None;
    if let Some(alt) = r.channel.alternatives.into_iter().next() {
        let words = words_from_alt(&alt);
        // SpeakerTurn events from the first speaker observed in
        // this batch.
        for w in &words {
            if let Some(spk) = w_speaker(w, &alt) {
                if Some(spk) != prev_speaker {
                    out.push(StreamEvent::SpeakerTurn {
                        speaker: SpeakerTag { id: spk, label: None },
                        at_ms: w.start_ms,
                    });
                    prev_speaker = Some(spk);
                }
            }
        }
        if is_final {
            out.push(StreamEvent::Final {
                segment: Segment {
                    text: alt.transcript,
                    start_ms: (r.start * 1000.0) as u32,
                    end_ms: ((r.start + r.duration) * 1000.0) as u32,
                    words,
                    speaker: prev_speaker.map(|id| SpeakerTag { id, label: None }),
                    confidence: alt.confidence,
                },
            });
        } else {
            out.push(StreamEvent::Partial {
                text: alt.transcript,
                start_ms: (r.start * 1000.0) as u32,
                end_ms: ((r.start + r.duration) * 1000.0) as u32,
                words,
            });
        }
    }
    out
}

fn words_from_alt(alt: &Alternative) -> Vec<Word> {
    alt.words
        .iter()
        .map(|w| Word {
            text: w.punctuated_word.clone().unwrap_or_else(|| w.word.clone()),
            start_ms: (w.start * 1000.0) as u32,
            end_ms: (w.end * 1000.0) as u32,
            confidence: w.confidence,
        })
        .collect()
}

/// Locate the speaker for a Word by matching start timestamps.
/// Cheap because words are typically a few dozen per packet.
fn w_speaker(w: &Word, alt: &Alternative) -> Option<u8> {
    alt.words
        .iter()
        .find(|dg| (dg.start * 1000.0) as u32 == w.start_ms)
        .and_then(|dg| dg.speaker)
}

#[async_trait]
impl StreamingSession for DeepgramStreamingSession {
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
            .map_err(|_| SttError::transport("deepgram: stream closed"))
    }

    async fn finish(&mut self) -> Result<()> {
        if !self.closed {
            self.closed = true;
            // Sentinel empty chunk → background task closes the WS.
            let _ = self.audio_tx.send(Bytes::new()).await;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.closed = true;
        // Drop the sender → background tasks unwind.
        let (tx, _) = mpsc::channel::<Bytes>(1);
        self.audio_tx = tx;
        Ok(())
    }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<StreamEvent, SttError>> + Send + '_>> {
        // Take ownership of the receiver lazily so re-entry returns
        // the same stream.
        let mut guard = self.events_rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(rx) => Box::pin(tokio_stream_from_receiver(rx)),
            None => Box::pin(futures::stream::empty()),
        }
    }
}

fn tokio_stream_from_receiver(
    mut rx: mpsc::Receiver<std::result::Result<StreamEvent, SttError>>,
) -> impl Stream<Item = std::result::Result<StreamEvent, SttError>> {
    futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
}
