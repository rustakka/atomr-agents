//! The session task — sole owner of the live `StreamingSession`.
//!
//! `StreamingSession::push_audio` and `StreamingSession::events` both
//! take `&mut self`, and the event stream borrows the session for its
//! whole lifetime, so a caller cannot push audio while holding the
//! event stream. This task resolves that: it owns both the
//! `StreamingSession` and the [`AudioPump`], pumps audio in, drains
//! events out, and forwards everything over channels the harness loop
//! consumes. The harness loop therefore never touches the session
//! directly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use atomr_agents_stt_core::{PcmBuffer, StreamEvent, StreamingSession, SttError};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::audio_source::AudioPump;

/// After `finish()`, how long the drain phase waits for the next event
/// before treating the backend as fully flushed. Real streaming
/// backends close their transport after a final flush; the in-process
/// mock keeps its internal channel open, so a quiet-period fallback is
/// what actually ends the drain there.
const DRAIN_QUIET: Duration = Duration::from_millis(150);

/// Channels + control handle the harness loop holds onto.
pub(crate) struct SessionHandle {
    /// Forwarded transcript events.
    pub event_rx: UnboundedReceiver<std::result::Result<StreamEvent, SttError>>,
    /// Forwarded per-chunk PCM, only `Some` when layered diarization
    /// asked for it.
    pub pcm_rx: Option<UnboundedReceiver<PcmBuffer>>,
    /// Cooperative stop flag — checked at the top of each pump cycle.
    pub stop: Arc<AtomicBool>,
    /// The spawned task. Awaited during harness teardown.
    pub join: JoinHandle<()>,
}

/// Spawn the session task. `want_pcm` controls whether decoded PCM is
/// forwarded for layered diarization.
pub(crate) fn spawn_session(
    mut session: Box<dyn StreamingSession>,
    mut pump: Box<dyn AudioPump>,
    want_pcm: bool,
) -> SessionHandle {
    let (event_tx, event_rx) = unbounded_channel::<std::result::Result<StreamEvent, SttError>>();
    let (pcm_tx, pcm_rx): (Option<UnboundedSender<PcmBuffer>>, Option<_>) = if want_pcm {
        let (tx, rx) = unbounded_channel::<PcmBuffer>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let stop = Arc::new(AtomicBool::new(false));
    let stop_task = stop.clone();

    let join = tokio::spawn(async move {
        let mut stopped = false;

        // Pump phase: push audio, opportunistically drain ready events.
        loop {
            if stop_task.load(Ordering::Relaxed) {
                stopped = true;
                break;
            }
            let chunk = match pump.next_chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break, // source drained → finish below
                Err(e) => {
                    let _ = event_tx.send(Err(into_stt_error(e)));
                    stopped = true;
                    break;
                }
            };
            if let Err(e) = session.push_audio(chunk.bytes).await {
                let _ = event_tx.send(Err(e));
                stopped = true;
                break;
            }
            if let (Some(tx), Some(pcm)) = (pcm_tx.as_ref(), chunk.pcm) {
                let _ = tx.send(pcm);
            }
            // Drain whatever events are immediately ready. The event
            // stream is acquired and dropped within this block so the
            // session's `&mut self` is free again for the next push.
            {
                let mut events = session.events();
                while let Some(item) = events.next().now_or_never().flatten() {
                    if event_tx.send(item).is_err() {
                        stopped = true;
                        break;
                    }
                }
            }
            if stopped {
                break;
            }
        }

        // Drain phase: no more pushes, so we can hold the event stream.
        // We bound each wait with `DRAIN_QUIET` — a genuinely closed
        // stream yields `None`, while a backend that simply goes quiet
        // (or an in-process mock that never closes its channel) is
        // treated as flushed once the quiet period elapses.
        if !stopped {
            let _ = session.finish().await;
            let mut events = session.events();
            loop {
                match tokio::time::timeout(DRAIN_QUIET, events.next()).await {
                    Ok(Some(item)) => {
                        if event_tx.send(item).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,      // stream genuinely ended
                    Err(_elapsed) => break, // quiet period — assume flushed
                }
            }
        }

        let _ = session.close().await;
    });

    SessionHandle {
        event_rx,
        pcm_rx,
        stop,
        join,
    }
}

/// The pump returns `crate::Result`; map its error into the STT error
/// the event channel carries.
fn into_stt_error(e: crate::error::SttHarnessError) -> SttError {
    match e {
        crate::error::SttHarnessError::Stt(inner) => inner,
        other => SttError::internal(other.to_string()),
    }
}
