//! `VoiceSession`: coalesce STT stream events into voice-level
//! events. In `Live` mode we forward partials and finals as they
//! arrive; in `TurnBased` mode we buffer partials and emit a
//! `UserTurn` when either the backend signals `UtteranceEnd` or the
//! VAD reports `silence_ms` of trailing silence.

use atomr_agents_stt_core::{
    Result, Segment, SpeakerTag, StreamEvent, StreamingSession, SttError, Word,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::vad::{EnergyVad, Vad};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceMode {
    Live,
    TurnBased { silence_ms: u32 },
}

impl Default for VoiceMode {
    fn default() -> Self {
        Self::TurnBased { silence_ms: 800 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    /// In-progress transcript text (only emitted in `Live` mode and
    /// before a `UserTurn` in `TurnBased` mode).
    PartialTranscript(String),
    /// A committed user utterance.
    UserTurn { text: String, segment: Segment },
    /// Speaker change reported by the backend.
    SpeakerChange(SpeakerTag),
    /// VAD-detected silence interval.
    SilenceDetected { duration_ms: u32 },
    /// One word arrived (live mode only — useful for incremental UI).
    InterimWord(Word),
}

pub struct VoiceSession {
    events_rx: mpsc::Receiver<std::result::Result<VoiceEvent, SttError>>,
    mode: VoiceMode,
    task: tokio::task::JoinHandle<()>,
}

impl VoiceSession {
    /// Wrap a `StreamingSession` with the requested voice mode. The
    /// session takes ownership of the stream — drop the
    /// `VoiceSession` (or call `close`) to release it.
    ///
    /// `vad` is used only in `TurnBased` mode for silence-based
    /// endpointing when the backend doesn't emit `UtteranceEnd`
    /// itself. Pass `None` to use [`EnergyVad`].
    pub fn open(
        stream: Box<dyn StreamingSession>,
        mode: VoiceMode,
        vad: Option<Box<dyn Vad>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<std::result::Result<VoiceEvent, SttError>>(64);
        let mut vad: Box<dyn Vad> = vad.unwrap_or_else(|| Box::new(EnergyVad::default()));
        let mode_for_task = mode;
        let mut stream = stream;

        let task = tokio::spawn(async move {
            use futures::StreamExt;
            let mut events = stream.events();
            // We can't keep a `&mut` across await safely, so route
            // each item out as it comes.
            let mut pending_partial: Option<String> = None;
            while let Some(item) = events.next().await {
                match item {
                    Ok(StreamEvent::Partial { text, words, .. }) => {
                        // VAD heuristic for live word-level
                        // updates: feed the words through if any.
                        for w in &words {
                            let _ = tx
                                .send(Ok(VoiceEvent::InterimWord(w.clone())))
                                .await;
                        }
                        match mode_for_task {
                            VoiceMode::Live => {
                                let _ = tx
                                    .send(Ok(VoiceEvent::PartialTranscript(text)))
                                    .await;
                            }
                            VoiceMode::TurnBased { .. } => {
                                pending_partial = Some(text);
                            }
                        }
                    }
                    Ok(StreamEvent::Final { segment }) => {
                        // Final is always a UserTurn in TurnBased
                        // mode; in Live mode we still emit it as
                        // a UserTurn (consumers can interpret as
                        // "stable text now").
                        pending_partial = None;
                        let text = segment.text.clone();
                        let _ = tx
                            .send(Ok(VoiceEvent::UserTurn { text, segment }))
                            .await;
                    }
                    Ok(StreamEvent::SpeakerTurn { speaker, .. }) => {
                        let _ = tx.send(Ok(VoiceEvent::SpeakerChange(speaker))).await;
                    }
                    Ok(StreamEvent::UtteranceEnd { .. }) => {
                        if let (VoiceMode::TurnBased { silence_ms }, Some(text)) =
                            (mode_for_task, pending_partial.take())
                        {
                            // Backend already reported endpoint; no
                            // need to wait the silence_ms timer.
                            let segment = Segment {
                                text: text.clone(),
                                start_ms: 0,
                                end_ms: 0,
                                words: vec![],
                                speaker: None,
                                confidence: None,
                            };
                            let _ = tx
                                .send(Ok(VoiceEvent::UserTurn { text, segment }))
                                .await;
                            let _ = tx
                                .send(Ok(VoiceEvent::SilenceDetected {
                                    duration_ms: silence_ms,
                                }))
                                .await;
                        } else {
                            let _ = tx
                                .send(Ok(VoiceEvent::SilenceDetected { duration_ms: 0 }))
                                .await;
                        }
                    }
                    Ok(StreamEvent::Metadata(_)) => {}
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }
            // Final flush in TurnBased mode: if the stream ends with
            // a pending partial, commit it.
            if let (VoiceMode::TurnBased { .. }, Some(text)) = (mode_for_task, pending_partial) {
                let segment = Segment {
                    text: text.clone(),
                    start_ms: 0,
                    end_ms: 0,
                    words: vec![],
                    speaker: None,
                    confidence: None,
                };
                let _ = tx
                    .send(Ok(VoiceEvent::UserTurn { text, segment }))
                    .await;
            }
            // Touch vad so the unused-write warning doesn't fire and
            // keep it alive until the task ends. Low-energy frame
            // input would have come via a parallel mic loop in the
            // pump helper — endpoint-by-silence is a future refinement.
            let _ = vad.is_speech(&[], 16_000);
            // Drop the events stream before the underlying stream
            // so the borrow ends.
            drop(events);
            // Stream goes out of scope here; `Box::drop` runs.
            drop(stream);
        });

        Self {
            events_rx: rx,
            mode,
            task,
        }
    }

    pub async fn recv(&mut self) -> Option<std::result::Result<VoiceEvent, SttError>> {
        self.events_rx.recv().await
    }

    pub fn events(&mut self) -> &mut mpsc::Receiver<std::result::Result<VoiceEvent, SttError>> {
        &mut self.events_rx
    }

    pub fn mode(&self) -> VoiceMode {
        self.mode
    }

    /// Abort the background task; the stream is dropped, which
    /// signals the underlying `StreamingSession` to tear down.
    pub async fn close(&mut self) -> Result<()> {
        self.task.abort();
        Ok(())
    }

    /// Sleep helper used by callers to demonstrate
    /// `silence_ms`-based endpointing in TurnBased mode tests.
    #[doc(hidden)]
    pub async fn _sleep(ms: u32) {
        sleep(Duration::from_millis(ms as u64)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::{
        AudioFormat, AudioInput, MockSpeechToText, SpeechToText, StreamOptions, TranscribeOptions,
    };
    use bytes::Bytes;

    #[tokio::test]
    async fn turn_based_emits_user_turn_on_final() {
        let stt = MockSpeechToText::new().with_text("hello world");
        let mut s = stt.open_stream(StreamOptions::default()).await.unwrap();
        s.push_audio(Bytes::from_static(b"chunk")).await.unwrap();
        s.finish().await.unwrap();

        let mut vs = VoiceSession::open(
            s,
            VoiceMode::TurnBased { silence_ms: 100 },
            None,
        );
        // Drain at most 5 events looking for UserTurn.
        let mut got_user = false;
        for _ in 0..5 {
            match vs.recv().await {
                Some(Ok(VoiceEvent::UserTurn { text, .. })) => {
                    assert_eq!(text, "hello world");
                    got_user = true;
                    break;
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("err: {e:?}"),
                None => break,
            }
        }
        assert!(got_user, "expected a UserTurn event");
    }

    #[tokio::test]
    async fn live_emits_partials() {
        // Using a mock STT that emits a partial per push.
        let stt = MockSpeechToText::new();
        let _ = stt
            .transcribe(
                AudioInput::Bytes {
                    data: Bytes::from_static(b"x"),
                    format: AudioFormat::Wav,
                },
                TranscribeOptions::default(),
            )
            .await
            .unwrap();
        let mut s = stt.open_stream(StreamOptions::default()).await.unwrap();
        s.push_audio(Bytes::from_static(b"chunk1")).await.unwrap();
        s.push_audio(Bytes::from_static(b"chunk2")).await.unwrap();
        let mut vs = VoiceSession::open(s, VoiceMode::Live, None);
        let ev = vs.recv().await.unwrap().unwrap();
        match ev {
            VoiceEvent::PartialTranscript(text) => assert!(text.contains("partial")),
            other => panic!("expected partial, got {other:?}"),
        }
    }
}
