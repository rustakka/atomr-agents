//! Agent-facing tools for the meetings harness.
//!
//! Each tool implements [`atomr_agents_tool::Tool`] so an LLM-driven
//! agent can call it during a run. Tools share a [`ToolHandle`] that
//! gives them mutable access to the [`MeetingAnalysis`] inside the
//! [`crate::MeetingsHarnessState`], guarded by a `Mutex`.
//!
//! The append-only invariant for `notes` and `actions` is enforced
//! here: there is no `delete_note` or `delete_action` tool. Update
//! tools (e.g. [`UpdateActionTool`]) patch existing rows in place but
//! never reorder.
//!
//! Tools also emit [`crate::MeetingsHarnessEvent`]s through the
//! optional event sink, so the web UI sees incremental progress.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::analysis::{Action, ActionStatus, Attendee, MeetingAnalysis, Note, SegmentSummary};
use crate::error::{MeetingsHarnessError, Result};
use crate::events::MeetingsHarnessEvent;

mod append_action;
mod append_note;
mod finalize;
mod finalize_segment;
mod get_turn;
mod list_turns;
mod regenerate_running;
mod revise_tail_segment;
mod set_title;
mod update_action;
mod upsert_attendee;

pub use append_action::AppendActionTool;
pub use append_note::AppendNoteTool;
pub use finalize::FinalizeTool;
pub use finalize_segment::FinalizeSegmentTool;
pub use get_turn::GetTurnTool;
pub use list_turns::ListTurnsTool;
pub use regenerate_running::RegenerateRunningTool;
pub use revise_tail_segment::ReviseTailSegmentTool;
pub use set_title::SetTitleTool;
pub use update_action::UpdateActionTool;
pub use upsert_attendee::UpsertAttendeeTool;

/// The full bundle of tools available to a meetings agent. Construct
/// one [`ToolHandle`] and pass it to every tool so they share state.
pub struct MeetingsToolSet {
    pub list_turns: ListTurnsTool,
    pub get_turn: GetTurnTool,
    pub upsert_attendee: UpsertAttendeeTool,
    pub append_note: AppendNoteTool,
    pub append_action: AppendActionTool,
    pub update_action: UpdateActionTool,
    pub revise_tail_segment: ReviseTailSegmentTool,
    pub finalize_segment: FinalizeSegmentTool,
    pub regenerate_running: RegenerateRunningTool,
    pub set_title: SetTitleTool,
    pub finalize: FinalizeTool,
}

impl MeetingsToolSet {
    /// Build the full tool bundle around a shared [`ToolHandle`].
    pub fn new(handle: ToolHandle) -> Self {
        Self {
            list_turns: ListTurnsTool::new(handle.clone()),
            get_turn: GetTurnTool::new(handle.clone()),
            upsert_attendee: UpsertAttendeeTool::new(handle.clone()),
            append_note: AppendNoteTool::new(handle.clone()),
            append_action: AppendActionTool::new(handle.clone()),
            update_action: UpdateActionTool::new(handle.clone()),
            revise_tail_segment: ReviseTailSegmentTool::new(handle.clone()),
            finalize_segment: FinalizeSegmentTool::new(handle.clone()),
            regenerate_running: RegenerateRunningTool::new(handle.clone()),
            set_title: SetTitleTool::new(handle.clone()),
            finalize: FinalizeTool::new(handle),
        }
    }
}

/// Shared mutable handle the tools (and a direct-call extractor) use to
/// read and modify the in-flight analysis.
#[derive(Clone)]
pub struct ToolHandle {
    inner: Arc<Mutex<MeetingAnalysis>>,
    transcript: Arc<Mutex<atomr_agents_stt_harness::SttConversation>>,
    events: Option<broadcast::Sender<MeetingsHarnessEvent>>,
}

impl ToolHandle {
    pub fn new(
        analysis: Arc<Mutex<MeetingAnalysis>>,
        transcript: Arc<Mutex<atomr_agents_stt_harness::SttConversation>>,
    ) -> Self {
        Self {
            inner: analysis,
            transcript,
            events: None,
        }
    }

    /// Attach an event sink so tool effects are broadcast to subscribers.
    pub fn with_events(mut self, sink: broadcast::Sender<MeetingsHarnessEvent>) -> Self {
        self.events = Some(sink);
        self
    }

    fn emit(&self, ev: MeetingsHarnessEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ev);
        }
    }

    /// Read a snapshot of the analysis.
    pub fn snapshot(&self) -> MeetingAnalysis {
        self.inner.lock().clone()
    }

    /// Read a snapshot of the source transcript.
    pub fn transcript_snapshot(&self) -> atomr_agents_stt_harness::SttConversation {
        self.transcript.lock().clone()
    }

    /// Replace the transcript snapshot (live mode advances it).
    pub fn set_transcript(&self, conv: atomr_agents_stt_harness::SttConversation) {
        *self.transcript.lock() = conv;
    }

    /// Update a single field via a closure. The closure runs under the
    /// analysis lock; keep it short.
    pub fn with_analysis<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut MeetingAnalysis) -> R,
    {
        let mut g = self.inner.lock();
        let out = f(&mut g);
        g.touch();
        out
    }

    // --- Domain operations the rule-based extractor and the agent
    //     tools share. Each enforces the append-only / id-resolution
    //     invariants and emits the matching event.

    /// Insert or merge an attendee. Matching is by `display_name`
    /// (case-insensitive) OR by any overlap in `speaker_tags`. Returns
    /// the attendee's stable id.
    pub fn upsert_attendee(
        &self,
        display_name: String,
        role: Option<String>,
        speaker_tags: Vec<u8>,
        email: Option<String>,
    ) -> String {
        let (attendee, _changed) = self.with_analysis(|a| {
            let lower = display_name.to_lowercase();
            let existing_idx = a.attendees.iter().position(|x| {
                x.display_name.to_lowercase() == lower
                    || x.speaker_tags.iter().any(|t| speaker_tags.contains(t))
            });
            let attendee = match existing_idx {
                Some(idx) => {
                    let existing = &mut a.attendees[idx];
                    if existing.display_name.to_lowercase() != lower {
                        existing.display_name = display_name.clone();
                    }
                    if let Some(r) = &role {
                        existing.role = Some(r.clone());
                    }
                    if let Some(e) = &email {
                        existing.email = Some(e.clone());
                    }
                    for tag in &speaker_tags {
                        if !existing.speaker_tags.contains(tag) {
                            existing.speaker_tags.push(*tag);
                        }
                    }
                    existing.clone()
                }
                None => {
                    let new = Attendee {
                        id: uuid::Uuid::new_v4().to_string(),
                        display_name,
                        role,
                        speaker_tags,
                        email,
                    };
                    a.attendees.push(new.clone());
                    new
                }
            };
            (attendee, true)
        });
        let id = attendee.id.clone();
        self.emit(MeetingsHarnessEvent::AttendeeUpserted { attendee });
        id
    }

    /// Append a note. Never reorders.
    pub fn append_note(
        &self,
        text: String,
        source_turn_indices: Vec<u64>,
        start_ms: Option<u32>,
        end_ms: Option<u32>,
    ) -> String {
        let note = Note {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            source_turn_indices,
            start_ms,
            end_ms,
        };
        let id = note.id.clone();
        self.with_analysis(|a| a.notes.push(note.clone()));
        self.emit(MeetingsHarnessEvent::NoteAppended { note });
        id
    }

    /// Append an action. The `owner_attendee_id`, if given, must resolve
    /// to an existing attendee.
    pub fn append_action(
        &self,
        description: String,
        owner_attendee_id: Option<String>,
        due_iso: Option<String>,
        supporting_quote: Option<String>,
        source_turn_index: Option<u64>,
    ) -> Result<String> {
        if let Some(oid) = &owner_attendee_id {
            let exists = self.with_analysis(|a| a.attendee(oid).is_some());
            if !exists {
                return Err(MeetingsHarnessError::tool(format!(
                    "unknown owner_attendee_id `{oid}`"
                )));
            }
        }
        let action = Action {
            id: uuid::Uuid::new_v4().to_string(),
            description,
            owner_attendee_id,
            due_iso,
            supporting_quote,
            source_turn_index,
            status: ActionStatus::Open,
        };
        let id = action.id.clone();
        self.with_analysis(|a| a.actions.push(action.clone()));
        self.emit(MeetingsHarnessEvent::ActionAppended { action });
        Ok(id)
    }

    /// Patch an existing action.
    pub fn update_action(
        &self,
        action_id: &str,
        status: Option<ActionStatus>,
        owner_attendee_id: Option<String>,
        due_iso: Option<String>,
        supporting_quote: Option<String>,
    ) -> Result<()> {
        if let Some(oid) = &owner_attendee_id {
            let exists = self.with_analysis(|a| a.attendee(oid).is_some());
            if !exists {
                return Err(MeetingsHarnessError::tool(format!(
                    "unknown owner_attendee_id `{oid}`"
                )));
            }
        }
        let found = self.with_analysis(|a| {
            let Some(action) = a.action_mut(action_id) else {
                return false;
            };
            if let Some(s) = status {
                action.status = s;
            }
            if owner_attendee_id.is_some() {
                action.owner_attendee_id = owner_attendee_id.clone();
            }
            if let Some(due) = &due_iso {
                action.due_iso = Some(due.clone());
            }
            if let Some(q) = &supporting_quote {
                action.supporting_quote = Some(q.clone());
            }
            true
        });
        if !found {
            return Err(MeetingsHarnessError::tool(format!(
                "unknown action_id `{action_id}`"
            )));
        }
        self.emit(MeetingsHarnessEvent::ActionUpdated {
            action_id: action_id.to_string(),
            status,
            owner_attendee_id,
            due_iso,
        });
        Ok(())
    }

    /// Revise (or open) the in-flight tail segment summary.
    pub fn revise_tail_segment(
        &self,
        text: String,
        start_turn_index: u64,
        end_turn_index: u64,
    ) -> Result<SegmentSummary> {
        if end_turn_index < start_turn_index {
            return Err(MeetingsHarnessError::tool(
                "end_turn_index must be >= start_turn_index",
            ));
        }
        let segment = self.with_analysis(|a| {
            if let Some(tail) = a.summary_levels.tail_mut() {
                tail.text = text.clone();
                tail.end_turn_index = end_turn_index.max(tail.end_turn_index);
                tail.start_turn_index = tail.start_turn_index.min(start_turn_index);
                tail.clone()
            } else {
                let seg = SegmentSummary {
                    id: uuid::Uuid::new_v4().to_string(),
                    start_turn_index,
                    end_turn_index,
                    text,
                    finalized: false,
                };
                a.summary_levels.segments.push(seg.clone());
                seg
            }
        });
        self.emit(MeetingsHarnessEvent::SegmentRevised {
            segment: segment.clone(),
        });
        Ok(segment)
    }

    /// Finalize the current tail segment.
    pub fn finalize_segment(&self) -> Result<Option<SegmentSummary>> {
        let segment = self.with_analysis(|a| {
            let Some(tail) = a.summary_levels.tail_mut() else {
                return None;
            };
            tail.finalized = true;
            Some(tail.clone())
        });
        if let Some(seg) = &segment {
            self.emit(MeetingsHarnessEvent::SegmentFinalized { segment: seg.clone() });
        }
        Ok(segment)
    }

    /// Recompute `summary_levels.running` from all *finalized* segments
    /// by concatenating their texts. Higher-level extractors may
    /// replace the text directly via `with_analysis` for a smarter
    /// rollup.
    pub fn regenerate_running(&self) -> String {
        let text = self.with_analysis(|a| {
            let joined = a
                .summary_levels
                .segments
                .iter()
                .filter(|s| s.finalized)
                .map(|s| s.text.clone())
                .collect::<Vec<_>>()
                .join("\n\n");
            a.summary_levels.running = Some(joined.clone());
            joined
        });
        self.emit(MeetingsHarnessEvent::RunningSummaryUpdated { text: text.clone() });
        text
    }

    /// Set the meeting title.
    pub fn set_title(&self, title: String) {
        self.with_analysis(|a| a.title = Some(title.clone()));
        self.emit(MeetingsHarnessEvent::TitleSet { title });
    }

    /// Mark the analysis final. Returns the final ledger sizes.
    pub fn finalize(&self, reason: String, tldr: Option<String>) -> (usize, usize) {
        let (note_count, action_count) = self.with_analysis(|a| {
            a.state = crate::analysis::AnalysisState::Final;
            if let Some(t) = tldr {
                a.summary_levels.tldr = Some(t);
            }
            for s in a.summary_levels.segments.iter_mut() {
                s.finalized = true;
            }
            (a.notes.len(), a.actions.len())
        });
        self.emit(MeetingsHarnessEvent::Finalized {
            reason,
            note_count,
            action_count,
        });
        (note_count, action_count)
    }

    /// Advance the watermark (live mode).
    pub fn advance_watermark(&self, turn_index: u64) {
        self.with_analysis(|a| a.last_processed_turn_index = Some(turn_index));
        self.emit(MeetingsHarnessEvent::WatermarkAdvanced { turn_index });
    }
}
