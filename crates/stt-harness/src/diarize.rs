//! Diarization stage.
//!
//! Wraps the configured [`DiarizationPolicy`] and, for the `Layered`
//! policy, owns a rolling PCM accumulator fed by the session task.
//! When an utterance commits, [`DiarizationStage::resolve_segment`]
//! either trusts the backend's speaker tag, clears it, or runs the
//! layered [`Diarizer`] over the retained PCM and stitches the spans
//! onto the segment by maximum overlap.

use atomr_agents_stt_core::{PcmBuffer, Segment, SpeakerTag};
use atomr_agents_stt_diarize_sherpa::DiarizationSpan;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::error::Result;
use crate::events::{SttEventSink, SttHarnessEvent};
use crate::spec::DiarizationPolicy;
use atomr_agents_stt_core::DiarizationSupport;

/// Per-run diarization stage.
pub(crate) struct DiarizationStage {
    policy: DiarizationPolicy,
    backend_support: DiarizationSupport,
    /// PCM chunks the session task forwarded since the last commit.
    /// `None` unless the policy is `Layered`.
    pcm_rx: Option<UnboundedReceiver<PcmBuffer>>,
    /// Rolling f32 mono samples for the current (open) utterance.
    accum: Vec<f32>,
    sample_rate: u32,
}

impl DiarizationStage {
    pub(crate) fn new(policy: DiarizationPolicy, backend_support: DiarizationSupport) -> Self {
        Self {
            policy,
            backend_support,
            pcm_rx: None,
            accum: Vec::new(),
            sample_rate: 16_000,
        }
    }

    /// Attach the PCM channel drained from the session task. Only set
    /// when the policy is `Layered`.
    pub(crate) fn attach_pcm(&mut self, rx: Option<UnboundedReceiver<PcmBuffer>>) {
        self.pcm_rx = rx;
    }

    /// `true` when this stage needs the session task to forward PCM.
    pub(crate) fn wants_pcm(&self) -> bool {
        matches!(self.policy, DiarizationPolicy::Layered(_))
    }

    /// Emit a warning if the policy contradicts the backend's
    /// advertised diarization support. The run still proceeds.
    pub(crate) fn warn_mismatch(&self, sink: &SttEventSink) {
        match (&self.policy, self.backend_support) {
            (
                DiarizationPolicy::Layered(_),
                DiarizationSupport::SpeakerCount | DiarizationSupport::NamedSpeakers,
            ) => sink.emit(SttHarnessEvent::DiarizationWarning {
                detail: "layered diarization requested, but the backend already diarizes".into(),
            }),
            (DiarizationPolicy::Backend, DiarizationSupport::None) => {
                sink.emit(SttHarnessEvent::DiarizationWarning {
                    detail: "backend diarization requested, but the backend reports \
                             DiarizationSupport::None — turns will have no speaker"
                        .into(),
                })
            }
            _ => {}
        }
    }

    /// A short human-readable description of the active policy, for the
    /// `Started` event.
    pub(crate) fn describe(&self) -> &'static str {
        match self.policy {
            DiarizationPolicy::Off => "off",
            DiarizationPolicy::Backend => "backend",
            DiarizationPolicy::Layered(_) => "layered",
        }
    }

    /// Drain whatever PCM the session task has forwarded into the
    /// rolling accumulator.
    fn drain_pcm(&mut self) {
        if let Some(rx) = &mut self.pcm_rx {
            while let Ok(pcm) = rx.try_recv() {
                if pcm.sample_rate > 0 {
                    self.sample_rate = pcm.sample_rate;
                }
                self.accum.extend_from_slice(&pcm.samples);
            }
        }
    }

    /// Resolve the speaker for a freshly-final segment, mutating
    /// `seg.speaker` in place. Resets the layered PCM accumulator so
    /// the next utterance starts clean.
    pub(crate) async fn resolve_segment(&mut self, seg: &mut Segment) -> Result<()> {
        // Clone the diarizer `Arc` (if any) so the borrow of
        // `self.policy` ends before we touch `self` mutably below.
        let diarizer = match &self.policy {
            DiarizationPolicy::Off => {
                seg.speaker = None;
                return Ok(());
            }
            DiarizationPolicy::Backend => {
                // Trust whatever the backend put on the segment.
                return Ok(());
            }
            DiarizationPolicy::Layered(diarizer) => diarizer.clone(),
        };
        self.drain_pcm();
        let samples = std::mem::take(&mut self.accum);
        let buf = PcmBuffer::new(samples, self.sample_rate.max(1), 1);
        let utterance_ms = (buf.duration_secs() * 1000.0) as u32;
        let spans = diarizer.diarize(&buf).await?;
        apply_spans_to_segment(seg, &spans, utterance_ms);
        Ok(())
    }
}

/// Stitch diarization spans onto a single segment by maximum overlap.
///
/// When the backend gave the segment degenerate timing (`end <= start`,
/// common for backends that emit one aggregate `Final`), the segment is
/// treated as spanning the whole retained utterance window so the
/// overlap calculation still works.
fn apply_spans_to_segment(seg: &mut Segment, spans: &[DiarizationSpan], utterance_ms: u32) {
    if spans.is_empty() {
        return;
    }
    let (lo, hi) = if seg.end_ms > seg.start_ms {
        (seg.start_ms, seg.end_ms)
    } else {
        (0, utterance_ms.max(1))
    };
    let mut best: Option<(u8, u32)> = None;
    for span in spans {
        let o_lo = lo.max(span.start_ms);
        let o_hi = hi.min(span.end_ms);
        if o_hi <= o_lo {
            continue;
        }
        let overlap = o_hi - o_lo;
        if best.map(|(_, prev)| overlap > prev).unwrap_or(true) {
            best = Some((span.speaker_id, overlap));
        }
    }
    // No span overlapped the segment window — fall back to the first
    // span's speaker rather than leaving the turn unattributed.
    let id = best.map(|(id, _)| id).unwrap_or(spans[0].speaker_id);
    seg.speaker = Some(SpeakerTag {
        id,
        label: Some(format!("speaker_{id}")),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_diarize_sherpa::MockDiarizer;
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;

    fn seg(start_ms: u32, end_ms: u32) -> Segment {
        Segment {
            text: "x".into(),
            start_ms,
            end_ms,
            words: vec![],
            speaker: Some(SpeakerTag { id: 9, label: None }),
            confidence: None,
        }
    }

    #[tokio::test]
    async fn off_clears_speaker() {
        let mut stage = DiarizationStage::new(DiarizationPolicy::Off, DiarizationSupport::SpeakerCount);
        let mut s = seg(0, 0);
        stage.resolve_segment(&mut s).await.unwrap();
        assert!(s.speaker.is_none());
    }

    #[tokio::test]
    async fn backend_passes_speaker_through() {
        let mut stage = DiarizationStage::new(DiarizationPolicy::Backend, DiarizationSupport::SpeakerCount);
        let mut s = seg(0, 0);
        stage.resolve_segment(&mut s).await.unwrap();
        assert_eq!(s.speaker.unwrap().id, 9);
    }

    #[tokio::test]
    async fn layered_stitches_spans_onto_degenerate_segment() {
        let mut stage = DiarizationStage::new(
            DiarizationPolicy::Layered(Arc::new(MockDiarizer::new(1.0, 2))),
            DiarizationSupport::None,
        );
        let (tx, rx) = unbounded_channel();
        // 2 seconds of 16 kHz mono PCM → MockDiarizer yields 2 spans
        // (speaker 0 then speaker 1).
        tx.send(PcmBuffer::new(vec![0.0; 32_000], 16_000, 1)).unwrap();
        drop(tx);
        stage.attach_pcm(Some(rx));
        let mut s = seg(0, 0); // degenerate timing
        stage.resolve_segment(&mut s).await.unwrap();
        // Speaker 0's span (0-1000ms) wins the max-overlap tie.
        assert_eq!(s.speaker.unwrap().id, 0);
    }
}
