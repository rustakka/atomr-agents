//! Glue: drain a `SynthesisStream` or `RealtimeSession` into a
//! `SpeakerStream`. Caller owns the futures and decides whether to
//! run them on a dedicated task.

use atomr_agents_stt_core::{Result, SttError};
use atomr_agents_tts_core::{RealtimeEvent, RealtimeSession, SynthesisStream};
use futures::StreamExt;

use crate::speaker::SpeakerStream;

/// Drain every `AudioChunk` from a `SynthesisStream` into the
/// speaker. Returns when the stream ends or `push_pcm_*` fails.
pub async fn pump_synthesis_to_speaker<S: SynthesisStream + ?Sized>(
    stream: &mut S,
    speaker: &SpeakerStream,
) -> Result<()> {
    let mut events = stream.events();
    while let Some(item) = events.next().await {
        let chunk = item?;
        // Backends emit either PCM-S16LE bytes or container bytes.
        // For the streaming case we assume PCM-S16LE (the default
        // for most live TTS backends). Container chunks need to be
        // decoded by the caller before reaching this pump.
        speaker.push_pcm_s16le(&chunk.bytes).await?;
        if chunk.is_final {
            break;
        }
    }
    Ok(())
}

/// Forward only the `AudioOut` events from a `RealtimeSession` to
/// the speaker. Other event kinds (transcripts, VAD signals) are
/// ignored — the caller should drive them via a separate consumer.
pub async fn pump_realtime_audio_to_speaker<R: RealtimeSession + ?Sized>(
    session: &mut R,
    speaker: &SpeakerStream,
) -> Result<()> {
    let mut events = session.events();
    while let Some(item) = events.next().await {
        let ev = item?;
        if let RealtimeEvent::AudioOut { chunk } = ev {
            speaker.push_pcm_s16le(&chunk.bytes).await?;
        }
    }
    Ok::<(), SttError>(())
}
