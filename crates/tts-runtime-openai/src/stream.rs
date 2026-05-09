//! Streaming response wrapper. The `/v1/audio/speech` endpoint
//! always uses chunked transfer encoding; we surface those chunks
//! through the `SynthesisStream` trait.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, Result, SttError};
use atomr_agents_tts_core::{AudioChunk, Capabilities, SynthesisStream};
use bytes::Bytes;
use futures::Stream;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::caps::CAPS;

pub(crate) struct OpenAiSynthesisStream {
    rx: Arc<Mutex<Option<mpsc::Receiver<std::result::Result<AudioChunk, SttError>>>>>,
    format: AudioFormat,
}

impl OpenAiSynthesisStream {
    /// Spawn a task that reads the response body's chunked frames
    /// and forwards them as `AudioChunk`s over an mpsc.
    pub(crate) fn spawn(
        body_stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>>
            + Send
            + 'static,
        format: AudioFormat,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<std::result::Result<AudioChunk, SttError>>(64);
        tokio::spawn(async move {
            use futures_util::StreamExt;
            futures::pin_mut!(body_stream);
            let mut seq = 0u64;
            let mut last_chunk: Option<AudioChunk> = None;
            while let Some(res) = body_stream.next().await {
                match res {
                    Ok(bytes) => {
                        if bytes.is_empty() {
                            continue;
                        }
                        if let Some(prev) = last_chunk.take() {
                            let _ = tx.send(Ok(prev)).await;
                        }
                        last_chunk = Some(AudioChunk {
                            bytes,
                            seq,
                            is_final: false,
                            words: Vec::new(),
                        });
                        seq += 1;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(SttError::transport(format!("openai stream: {e}"))))
                            .await;
                        return;
                    }
                }
            }
            // Mark the last buffered chunk as final.
            if let Some(mut last) = last_chunk {
                last.is_final = true;
                let _ = tx.send(Ok(last)).await;
            } else {
                // Empty body — emit one explicit final chunk.
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
impl SynthesisStream for OpenAiSynthesisStream {
    fn capabilities(&self) -> &'static Capabilities { &CAPS }
    fn format(&self) -> &AudioFormat { &self.format }

    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<AudioChunk, SttError>> + Send + '_>>
    {
        let mut guard = self.rx.lock();
        let rx = guard.take();
        drop(guard);
        match rx {
            Some(mut rx) => Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx))),
            None => Box::pin(futures::stream::empty()),
        }
    }

    async fn close(&mut self) -> Result<()> {
        let mut guard = self.rx.lock();
        *guard = None;
        Ok(())
    }
}
