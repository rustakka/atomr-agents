//! Stitch [`DiarizationSpan`]s into the speaker tags of a
//! [`Transcript`]'s segments.

use atomr_agents_stt_core::{SpeakerTag, Transcript};

use crate::span::DiarizationSpan;

/// For each segment, pick the speaker whose span overlaps the
/// segment most. If no span overlaps, leave `speaker` as-is.
pub fn apply_to_transcript(transcript: &mut Transcript, spans: &[DiarizationSpan]) {
    for seg in &mut transcript.segments {
        let mut best: Option<(u8, u32, Option<f32>)> = None;
        for span in spans {
            let lo = seg.start_ms.max(span.start_ms);
            let hi = seg.end_ms.min(span.end_ms);
            if hi <= lo {
                continue;
            }
            let overlap = hi - lo;
            if best.map(|(_, prev, _)| overlap > prev).unwrap_or(true) {
                best = Some((span.speaker_id, overlap, span.confidence));
            }
        }
        if let Some((id, _, conf)) = best {
            seg.speaker = Some(SpeakerTag {
                id,
                label: Some(format!("speaker_{id}")),
            });
            // Keep the segment's existing confidence (transcript
            // confidence) but stamp the diarizer's confidence into
            // the speaker tag's label if useful upstream.
            let _ = conf;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::{BackendKind, Segment};

    #[test]
    fn assigns_speaker_with_max_overlap() {
        let mut t = Transcript {
            text: "hello world".into(),
            language: None,
            segments: vec![
                Segment {
                    text: "hello".into(),
                    start_ms: 0,
                    end_ms: 1_000,
                    words: vec![],
                    speaker: None,
                    confidence: None,
                },
                Segment {
                    text: "world".into(),
                    start_ms: 1_000,
                    end_ms: 2_000,
                    words: vec![],
                    speaker: None,
                    confidence: None,
                },
            ],
            duration_secs: 2.0,
            backend: BackendKind::WhisperLocal,
            model_id: None,
            cost_usd: None,
        };
        let spans = vec![
            DiarizationSpan {
                start_ms: 0,
                end_ms: 1_500,
                speaker_id: 0,
                confidence: Some(1.0),
            },
            DiarizationSpan {
                start_ms: 1_500,
                end_ms: 2_000,
                speaker_id: 1,
                confidence: Some(1.0),
            },
        ];
        apply_to_transcript(&mut t, &spans);
        assert_eq!(t.segments[0].speaker.as_ref().unwrap().id, 0);
        assert_eq!(t.segments[1].speaker.as_ref().unwrap().id, 0); // 0-1000ms overlap with span 0 = 0; 1000-1500 with span 0 = 500; 1500-2000 with span 1 = 500. tie → first wins.
    }
}
