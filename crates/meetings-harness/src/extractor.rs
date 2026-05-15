//! Pluggable extraction strategy.
//!
//! A [`MeetingExtractor`] consumes a window of transcript turns plus a
//! [`ToolHandle`] and mutates the in-flight [`crate::MeetingAnalysis`].
//! The harness's loop strategies call into the extractor once per
//! iteration; the extractor is free to invoke tools directly or, in an
//! agent-driven implementation, to dispatch them through an LLM tool-call
//! loop.
//!
//! The shipped default — [`RuleBasedExtractor`] — is deterministic and
//! LLM-free, so the crate's tests and the web UI work end-to-end without
//! a model provider. Callers can drop in their own LLM-driven extractor
//! by implementing the trait; the spec's `model_id` is recorded on the
//! resulting analysis for telemetry.

use std::collections::HashSet;

use async_trait::async_trait;
use atomr_agents_stt_harness::SttConversation;
use regex::Regex;

use crate::analysis::ActionStatus;
use crate::error::Result;
use crate::tools::ToolHandle;

/// A windowed view of source turns to process in one extraction step.
#[derive(Debug, Clone)]
pub struct ExtractionWindow {
    /// Inclusive lower bound. `None` means "from the beginning".
    pub since_turn_index: Option<u64>,
    /// Exclusive upper bound. `None` means "to the end".
    pub until_turn_index: Option<u64>,
}

impl ExtractionWindow {
    /// A window spanning the entire transcript.
    pub fn all() -> Self {
        Self {
            since_turn_index: None,
            until_turn_index: None,
        }
    }

    /// A window starting after a watermark.
    pub fn after(watermark: Option<u64>) -> Self {
        Self {
            since_turn_index: watermark.map(|w| w + 1),
            until_turn_index: None,
        }
    }
}

/// The input to one extraction step.
#[derive(Debug, Clone)]
pub struct ExtractionRequest {
    pub window: ExtractionWindow,
    /// `true` when this is the final step in the run (e.g. live mode
    /// saw `Finished`, or batch is wrapping up). Extractors should
    /// regenerate the TL;DR and call `finalize` on the handle.
    pub finalize: bool,
    /// `true` when the harness is in live mode. Live extractors
    /// segment the running rollup; batch extractors typically don't.
    pub live: bool,
    /// Recommended segment size in turns. The default extractor opens
    /// a new tail segment every `segment_turn_count` turns.
    pub segment_turn_count: u32,
    /// Optional system-prompt override from the harness spec.
    pub system_prompt: Option<String>,
}

/// Pluggable extraction strategy.
#[async_trait]
pub trait MeetingExtractor: Send + Sync + 'static {
    /// Process one window of transcript content. Implementations should
    /// drive the [`ToolHandle`] to add attendees / notes / actions /
    /// summaries; the harness loop persists the result after each call.
    async fn extract(&self, request: &ExtractionRequest, handle: &ToolHandle) -> Result<()>;
}

#[async_trait]
impl MeetingExtractor for Box<dyn MeetingExtractor> {
    async fn extract(&self, request: &ExtractionRequest, handle: &ToolHandle) -> Result<()> {
        (**self).extract(request, handle).await
    }
}

/// A deterministic, LLM-free extractor. Mines attendees from speaker
/// labels, drops a note per turn, regex-detects commitments
/// ("I'll", "I will", "let's", "we should", "we need to", "todo")
/// and assigns the owner to the speaker who said it.
///
/// Used as the default so the rest of the system (tools, harness, web
/// UI, persistence) can be exercised end-to-end without a model
/// provider. Replace with an LLM-driven extractor for production use.
pub struct RuleBasedExtractor {
    commitment_re: Regex,
}

impl Default for RuleBasedExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleBasedExtractor {
    pub fn new() -> Self {
        // Match common commitment markers. Capture group 1 is the
        // remainder of the sentence — used as the action description.
        let commitment_re = Regex::new(
            r"(?i)\b(?:i(?:'ll| will)|let'?s|we should|we'?ll|we need to|i need to|todo:?|action item:?)\b[\s,]*([^.!?\n]{3,200})",
        )
        .expect("static regex");
        Self { commitment_re }
    }

    /// Produce a short segment-summary text from a slice of turns.
    fn summarize_segment(turns: &[(u64, String, String)]) -> String {
        if turns.is_empty() {
            return String::new();
        }
        let speakers: HashSet<&str> = turns.iter().map(|t| t.1.as_str()).collect();
        let mut speaker_list: Vec<&str> = speakers.into_iter().collect();
        speaker_list.sort_unstable();
        let preview = turns
            .iter()
            .take(2)
            .map(|t| format!("{}: {}", t.1, truncate(&t.2, 80)))
            .collect::<Vec<_>>()
            .join(" / ");
        format!(
            "Turns {}-{} ({} speakers: {}). {}",
            turns.first().unwrap().0,
            turns.last().unwrap().0,
            speaker_list.len(),
            speaker_list.join(", "),
            preview
        )
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

#[async_trait]
impl MeetingExtractor for RuleBasedExtractor {
    async fn extract(&self, request: &ExtractionRequest, handle: &ToolHandle) -> Result<()> {
        let conv: SttConversation = handle.transcript_snapshot();
        let since = request.window.since_turn_index.unwrap_or(0);
        let until = request.window.until_turn_index.unwrap_or(u64::MAX);

        // Restrict to turns in window.
        let in_window: Vec<&atomr_agents_stt_harness::SttTurn> = conv
            .turns
            .iter()
            .filter(|t| t.index >= since && t.index < until)
            .collect();

        if in_window.is_empty() && !request.finalize {
            return Ok(());
        }

        // --- Attendees: one per distinct speaker id in window.
        let mut owners_by_speaker: std::collections::HashMap<u8, String> = std::collections::HashMap::new();
        for turn in &in_window {
            let Some(sid) = turn.speaker_id() else { continue };
            let label = conv.effective_label(sid);
            let attendee_id = handle.upsert_attendee(label, None, vec![sid], None);
            owners_by_speaker.insert(sid, attendee_id);
        }
        // Also seed attendees we've already discovered (so older turns
        // can be the owner of a newly-found action that references them).
        for att in &handle.snapshot().attendees {
            for tag in &att.speaker_tags {
                owners_by_speaker.entry(*tag).or_insert_with(|| att.id.clone());
            }
        }

        // --- Notes: one per turn in window with non-trivial text.
        for turn in &in_window {
            let trimmed = turn.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            handle.append_note(
                trimmed.to_string(),
                vec![turn.index],
                Some(turn.start_ms),
                Some(turn.end_ms),
            );
        }

        // --- Actions: commitment markers per turn.
        for turn in &in_window {
            for caps in self.commitment_re.captures_iter(&turn.text) {
                let desc = caps
                    .get(1)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                if desc.is_empty() {
                    continue;
                }
                let owner = turn
                    .speaker_id()
                    .and_then(|sid| owners_by_speaker.get(&sid))
                    .cloned();
                let _ = handle.append_action(desc, owner, None, Some(turn.text.clone()), Some(turn.index))?;
            }
        }

        // --- Tail segment revision (live) or batch-wide segments.
        if request.live {
            // Live: build / revise the in-flight tail segment with
            // exactly the turns from the watermark forward. Once it
            // crosses `segment_turn_count`, finalize and open a new one.
            let snap = handle.snapshot();
            let tail_start = snap
                .summary_levels
                .tail()
                .map(|s| s.start_turn_index)
                .or_else(|| in_window.first().map(|t| t.index));
            let last_seen = in_window.last().map(|t| t.index);
            if let (Some(s), Some(e)) = (tail_start, last_seen) {
                let slice: Vec<(u64, String, String)> = conv
                    .turns
                    .iter()
                    .filter(|t| t.index >= s && t.index <= e)
                    .map(|t| {
                        (
                            t.index,
                            t.speaker_id()
                                .map(|sid| conv.effective_label(sid))
                                .unwrap_or_else(|| "unknown".into()),
                            t.text.clone(),
                        )
                    })
                    .collect();
                let text = Self::summarize_segment(&slice);
                handle.revise_tail_segment(text, s, e)?;
                let span = (e - s) + 1;
                if span >= request.segment_turn_count as u64 {
                    handle.finalize_segment()?;
                    // Recompute the running rollup once a segment freezes.
                    handle.regenerate_running();
                }
            }
            // Advance watermark to the highest processed index.
            if let Some(max_idx) = in_window.iter().map(|t| t.index).max() {
                handle.advance_watermark(max_idx);
            }
        } else if let (Some(first), Some(last)) = (in_window.first(), in_window.last()) {
            // Batch: one segment per `segment_turn_count` chunk.
            let mut cursor = first.index;
            while cursor <= last.index {
                let chunk_end = (cursor + request.segment_turn_count as u64 - 1).min(last.index);
                let slice: Vec<(u64, String, String)> = conv
                    .turns
                    .iter()
                    .filter(|t| t.index >= cursor && t.index <= chunk_end)
                    .map(|t| {
                        (
                            t.index,
                            t.speaker_id()
                                .map(|sid| conv.effective_label(sid))
                                .unwrap_or_else(|| "unknown".into()),
                            t.text.clone(),
                        )
                    })
                    .collect();
                let text = Self::summarize_segment(&slice);
                handle.revise_tail_segment(text, cursor, chunk_end)?;
                handle.finalize_segment()?;
                cursor = chunk_end + 1;
            }
            handle.regenerate_running();
            handle.advance_watermark(last.index);
        }

        if request.finalize {
            // Carry-over: any in-flight tail gets frozen and the rollup
            // refreshed before the TL;DR is composed.
            handle.finalize_segment()?;
            let running = handle.regenerate_running();
            // Title: first 60 chars of the running rollup.
            if handle.snapshot().title.is_none() {
                let title = truncate(running.trim(), 60);
                if !title.is_empty() {
                    handle.set_title(title);
                }
            }
            // Re-open any actions that look done from the transcript
            // alone: keep them Open by default; callers patch state via
            // update_action.
            let _ = ActionStatus::Open;
            let tldr = build_tldr(handle);
            handle.finalize("extractor".into(), Some(tldr));
        }

        Ok(())
    }
}

fn build_tldr(handle: &ToolHandle) -> String {
    let snap = handle.snapshot();
    let att_names: Vec<String> = snap.attendees.iter().map(|a| a.display_name.clone()).collect();
    let open_actions = snap
        .actions
        .iter()
        .filter(|a| matches!(a.status, ActionStatus::Open))
        .count();
    format!(
        "{} attendees ({}). {} notes, {} actions ({} open).",
        snap.attendees.len(),
        att_names.join(", "),
        snap.notes.len(),
        snap.actions.len(),
        open_actions,
    )
}
