//! The structured meeting record this harness accumulates.
//!
//! [`MeetingAnalysis`] is **pure, serializable data** — no live handles.
//! It is the harness's "working memory" while a run is in progress and
//! the value [`crate::MeetingsHarness::run`] ultimately returns.
//!
//! Two invariants the data model commits to:
//!
//! 1. **Append-only ledger.** [`MeetingAnalysis::notes`] and
//!    [`MeetingAnalysis::actions`] grow at the tail. Existing entries
//!    may be patched in place (e.g. status changes on an action) but
//!    are never deleted or reordered. The tool layer enforces this.
//! 2. **Tiered, dynamic summarization.** [`SummaryLevels`] holds three
//!    layers — per-segment summaries, a running rollup, and a final
//!    TL;DR. Only the in-flight tail segment is mutable; earlier
//!    segments are frozen on finalize. Each level can be regenerated
//!    independently, so live updates touch only the affected tier.
//!
//! The `id` field re-uses the source transcript's `conversation_id`, so
//! the analysis and the diarized transcript join naturally in the same
//! persistence backend.

use serde::{Deserialize, Serialize};

/// Lifecycle state of an analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisState {
    /// No extraction has run yet.
    Pending,
    /// Live mode: the run is still consuming new turns.
    Streaming,
    /// `finalize()` has been called; no further updates expected.
    Final,
}

impl Default for AnalysisState {
    fn default() -> Self {
        AnalysisState::Pending
    }
}

/// Status of a single action item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Open,
    Done,
    Cancelled,
}

impl Default for ActionStatus {
    fn default() -> Self {
        ActionStatus::Open
    }
}

/// How a run is driven. Decoupled from the loop strategy so the spec is
/// declarative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunMode {
    /// Read the full transcript once and produce the analysis in a
    /// single bounded run.
    Batch,
    /// Subscribe to the source STT harness's broadcast and continuously
    /// update as new turns commit. `segment_turn_count` is the size at
    /// which the in-flight tail segment finalizes and a new one opens.
    Live {
        /// The size of each per-segment summary window. Once the tail
        /// segment reaches this many turns it is finalized and a new
        /// in-flight segment opens for subsequent turns.
        segment_turn_count: u32,
    },
}

impl Default for RunMode {
    fn default() -> Self {
        RunMode::Batch
    }
}

/// A person who participated in the meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attendee {
    /// Stable id (uuid-shaped) used as a foreign key from
    /// [`Action::owner_attendee_id`].
    pub id: String,
    pub display_name: String,
    /// Optional role descriptor, e.g. "host", "engineer", "PM".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Numeric diarized speaker ids this attendee speaks under.
    #[serde(default)]
    pub speaker_tags: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// A linear, timestamped note in the ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
    /// Indices into [`atomr_agents_stt_harness::SttConversation::turns`]
    /// that originated this note.
    #[serde(default)]
    pub source_turn_indices: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u32>,
}

/// An action item with an optional owner and source quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub description: String,
    /// FK to [`Attendee::id`]. The harness validates references on
    /// insert; once set, an attendee with that id exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_attendee_id: Option<String>,
    /// ISO-8601 date string if extractable, else `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_iso: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supporting_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_turn_index: Option<u64>,
    #[serde(default)]
    pub status: ActionStatus,
}

/// A summary window over a contiguous block of turns. Only the tail
/// (the one with `finalized = false`) is revised; the rest are frozen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentSummary {
    pub id: String,
    pub start_turn_index: u64,
    pub end_turn_index: u64,
    pub text: String,
    #[serde(default)]
    pub finalized: bool,
}

/// The tiered summary stack — each layer can be regenerated
/// independently as the meeting unfolds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryLevels {
    /// Per-segment summaries in chronological order. The last entry
    /// (`finalized == false`) is the in-flight tail; everything earlier
    /// is frozen.
    #[serde(default)]
    pub segments: Vec<SegmentSummary>,
    /// Running rollup of all *finalized* segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
    /// Final TL;DR — populated by `finalize()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tldr: Option<String>,
}

impl SummaryLevels {
    /// Reference the in-flight tail segment, if any.
    pub fn tail(&self) -> Option<&SegmentSummary> {
        self.segments.iter().rev().find(|s| !s.finalized)
    }

    /// Mutable reference to the in-flight tail, if any.
    pub fn tail_mut(&mut self) -> Option<&mut SegmentSummary> {
        self.segments.iter_mut().rev().find(|s| !s.finalized)
    }

    /// Highest turn index already covered by some segment, finalized or
    /// not.
    pub fn highest_covered_turn(&self) -> Option<u64> {
        self.segments.iter().map(|s| s.end_turn_index).max()
    }
}

/// The full analysis record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingAnalysis {
    /// Same as `SttConversation::id` — the conversation_id this
    /// analysis is bound to.
    pub id: String,
    /// Optional human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tiered summary stack.
    #[serde(default)]
    pub summary_levels: SummaryLevels,
    /// Attendee roster.
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    /// Linear, append-only notes ledger.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Linear, append-only actions ledger. Existing entries may be
    /// patched in place; never deleted or reordered.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Always equals `id`; kept explicit for clarity and for joins.
    pub source_transcript_id: String,
    /// Watermark used in live mode: the highest turn index already
    /// processed by the extractor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_processed_turn_index: Option<u64>,
    /// Millis-since-epoch when this analysis was first created.
    pub generated_at_ms: i64,
    /// Millis-since-epoch when this analysis was last touched.
    pub updated_at_ms: i64,
    /// Model id used by the extractor (if LLM-driven). Recorded for
    /// telemetry; the rule-based default leaves it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Lifecycle state.
    #[serde(default)]
    pub state: AnalysisState,
}

impl MeetingAnalysis {
    /// Fresh, empty analysis bound to a transcript id.
    pub fn new(conversation_id: impl Into<String>) -> Self {
        let id = conversation_id.into();
        let now = now_ms();
        Self {
            source_transcript_id: id.clone(),
            id,
            title: None,
            summary_levels: SummaryLevels::default(),
            attendees: Vec::new(),
            notes: Vec::new(),
            actions: Vec::new(),
            last_processed_turn_index: None,
            generated_at_ms: now,
            updated_at_ms: now,
            model_id: None,
            state: AnalysisState::Pending,
        }
    }

    /// Look up an attendee by id.
    pub fn attendee(&self, id: &str) -> Option<&Attendee> {
        self.attendees.iter().find(|a| a.id == id)
    }

    /// Look up an attendee by id (mutable).
    pub fn attendee_mut(&mut self, id: &str) -> Option<&mut Attendee> {
        self.attendees.iter_mut().find(|a| a.id == id)
    }

    /// Look up an action by id (mutable).
    pub fn action_mut(&mut self, id: &str) -> Option<&mut Action> {
        self.actions.iter_mut().find(|a| a.id == id)
    }

    /// Find an attendee already linked to a numeric diarized speaker id.
    pub fn attendee_for_speaker(&self, speaker_id: u8) -> Option<&Attendee> {
        self.attendees
            .iter()
            .find(|a| a.speaker_tags.iter().any(|t| *t == speaker_id))
    }

    /// Bump `updated_at_ms` to "now".
    pub fn touch(&mut self) {
        self.updated_at_ms = now_ms();
    }
}

pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_analysis_inherits_transcript_id() {
        let a = MeetingAnalysis::new("call-7");
        assert_eq!(a.id, "call-7");
        assert_eq!(a.source_transcript_id, "call-7");
        assert_eq!(a.state, AnalysisState::Pending);
        assert!(a.notes.is_empty());
        assert!(a.actions.is_empty());
        assert!(a.attendees.is_empty());
    }

    #[test]
    fn serde_round_trip_preserves_all_fields() {
        let mut a = MeetingAnalysis::new("c1");
        a.title = Some("Weekly sync".into());
        a.attendees.push(Attendee {
            id: "att-1".into(),
            display_name: "Alice".into(),
            role: Some("PM".into()),
            speaker_tags: vec![0],
            email: None,
        });
        a.notes.push(Note {
            id: "n-1".into(),
            text: "Discussed Q3 plan".into(),
            source_turn_indices: vec![0, 1, 2],
            start_ms: Some(0),
            end_ms: Some(15_000),
        });
        a.actions.push(Action {
            id: "a-1".into(),
            description: "Ship the proposal".into(),
            owner_attendee_id: Some("att-1".into()),
            due_iso: Some("2026-06-01".into()),
            supporting_quote: Some("I'll send it by next week.".into()),
            source_turn_index: Some(2),
            status: ActionStatus::Open,
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: MeetingAnalysis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title.as_deref(), Some("Weekly sync"));
        assert_eq!(back.attendees.len(), 1);
        assert_eq!(back.notes[0].source_turn_indices, vec![0, 1, 2]);
        assert_eq!(back.actions[0].owner_attendee_id.as_deref(), Some("att-1"));
    }

    #[test]
    fn tail_segment_is_the_unfinalized_one() {
        let mut s = SummaryLevels::default();
        s.segments.push(SegmentSummary {
            id: "s1".into(),
            start_turn_index: 0,
            end_turn_index: 9,
            text: "first".into(),
            finalized: true,
        });
        s.segments.push(SegmentSummary {
            id: "s2".into(),
            start_turn_index: 10,
            end_turn_index: 14,
            text: "growing".into(),
            finalized: false,
        });
        assert_eq!(s.tail().unwrap().id, "s2");
        assert_eq!(s.highest_covered_turn(), Some(14));
    }
}
