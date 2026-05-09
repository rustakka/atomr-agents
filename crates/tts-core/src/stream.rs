//! Streaming-output session: caller awaits a stream of audio chunks
//! emitted as the backend generates them.

use std::pin::Pin;

use async_trait::async_trait;
use atomr_agents_stt_core::{AudioFormat, Result, SttError};
use bytes::Bytes;
use futures::Stream;
use serde::Serialize;

use crate::capabilities::Capabilities;

#[async_trait]
pub trait SynthesisStream: Send {
    fn capabilities(&self) -> &'static Capabilities;
    fn format(&self) -> &AudioFormat;

    /// Stream of audio chunks. Each `Ok(AudioChunk { is_final: true })`
    /// signals the last frame.
    fn events(
        &mut self,
    ) -> Pin<Box<dyn Stream<Item = std::result::Result<AudioChunk, SttError>> + Send + '_>>;

    async fn close(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioChunk {
    /// Encoded bytes in the format advertised by
    /// [`SynthesisStream::format`] (PCM frames or container chunks).
    #[serde(with = "serde_bytes_compat")]
    pub bytes: Bytes,
    /// Monotonic frame / chunk index since session start.
    pub seq: u64,
    /// True for the last chunk of the synthesis. Subsequent
    /// `events()` polls return `None`.
    pub is_final: bool,
    /// Optional aligned word timing for this chunk.
    #[serde(default)]
    pub words: Vec<WordTiming>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WordTiming {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
}

mod serde_bytes_compat {
    use bytes::Bytes;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(b: &Bytes, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_bytes(b)
    }
}
