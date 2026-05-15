//! The conversation record an STT harness accumulates.
//!
//! [`SttConversation`] is **pure, serializable data** — no live
//! handles. It is the harness's "working memory": partials fold into
//! [`SttConversation::open_partial`], finals commit ordered
//! [`SttTurn`]s. It maps cleanly to and from the agentic conversation
//! structures in `atomr_agents_core` ([`Message`] / [`TurnInput`]) via
//! [`SttConversation::to_messages`] and
//! [`SttConversation::to_turn_input`], so an STT interaction can feed an
//! agent turn directly.
//!
//! Speaker labels are editable per conversation: numeric diarizer IDs
//! are stable, but a reviewer can rename "speaker_0" to "Alice" via
//! [`SttConversation::rename_speaker`]; [`SttConversation::effective_label`]
//! resolves the override > the backend's `SpeakerTag` label > the
//! `speaker_{id}` fallback.

use std::collections::HashMap;

use atomr_agents_core::{Message, MessageRole, TurnInput};
use atomr_agents_stt_core::{BackendKind, Segment, SpeakerTag, Word};
use serde::{Deserialize, Serialize};

/// State of a turn: still being revised, or committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    /// In-flight; may still be revised by a later `Final`.
    Partial,
    /// Committed; will not change (except for speaker re-labeling).
    Final,
}

/// Who produced a turn. Bridges STT speaker IDs to agentic roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpeakerRef {
    /// Attributed to a diarizer/backend numeric speaker id.
    Diarized { tag: SpeakerTag },
    /// Mapped to an agentic message role (e.g. an appended assistant
    /// reply, or a caller-assigned role).
    Role { role: MessageRole },
    /// Single-speaker stream, or no diarization was run.
    Unknown,
}

impl SpeakerRef {
    /// The numeric diarizer id, when this turn is diarized.
    pub fn id(&self) -> Option<u8> {
        match self {
            SpeakerRef::Diarized { tag } => Some(tag.id),
            _ => None,
        }
    }
}

/// An in-flight partial transcript, not yet committed to a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialBuf {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
    pub words: Vec<Word>,
}

/// One committed (or in-flight) utterance. Aligns to one agentic
/// [`Message`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttTurn {
    /// Position in the conversation (0-based).
    pub index: u64,
    pub speaker: SpeakerRef,
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
    #[serde(default)]
    pub words: Vec<Word>,
    pub confidence: Option<f32>,
    pub state: TurnState,
}

impl SttTurn {
    /// The numeric diarizer speaker id, when diarized.
    pub fn speaker_id(&self) -> Option<u8> {
        self.speaker.id()
    }
}

/// Caller policy mapping diarized speaker ids to agentic roles. The
/// default treats every speaker as `User` — an STT stream is "what the
/// user(s) said" — which is the right shape for feeding a single agent.
#[derive(Debug, Clone)]
pub struct SpeakerMap {
    pub roles: HashMap<u8, MessageRole>,
    pub default: MessageRole,
}

impl Default for SpeakerMap {
    fn default() -> Self {
        Self {
            roles: HashMap::new(),
            default: MessageRole::User,
        }
    }
}

impl SpeakerMap {
    /// Build a map from explicit `(speaker_id, role)` pairs.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (u8, MessageRole)>) -> Self {
        Self {
            roles: pairs.into_iter().collect(),
            default: MessageRole::User,
        }
    }

    /// Resolve the role for a turn.
    pub fn role_for(&self, turn: &SttTurn) -> MessageRole {
        match &turn.speaker {
            SpeakerRef::Role { role } => *role,
            SpeakerRef::Diarized { tag } => self.roles.get(&tag.id).copied().unwrap_or(self.default),
            SpeakerRef::Unknown => self.default,
        }
    }
}

/// The accumulated record of an STT interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConversation {
    /// Stable id — derived from the harness id + run id.
    pub id: String,
    pub language: Option<String>,
    /// Committed turns, in order.
    pub turns: Vec<SttTurn>,
    /// In-flight partial, not yet a turn. Not serialized.
    #[serde(skip)]
    pub open_partial: Option<PartialBuf>,
    pub backend: Option<BackendKind>,
    pub model_id: Option<String>,
    pub total_audio_secs: f32,
    /// Per-conversation speaker-label overrides keyed by numeric id.
    /// This is the editable surface the review UI writes to.
    #[serde(default)]
    pub speaker_labels: HashMap<u8, String>,
    /// The most recent backend-reported speaker change; partials and
    /// finals without an explicit speaker inherit it. Not serialized.
    #[serde(skip)]
    current_speaker: Option<SpeakerTag>,
}

impl SttConversation {
    /// A fresh, empty conversation with the given id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            language: None,
            turns: Vec::new(),
            open_partial: None,
            backend: None,
            model_id: None,
            total_audio_secs: 0.0,
            speaker_labels: HashMap::new(),
            current_speaker: None,
        }
    }

    /// Fold an in-progress partial transcript into the open buffer.
    pub fn apply_partial(&mut self, text: String, start_ms: u32, end_ms: u32, words: Vec<Word>) {
        self.bump_audio(end_ms);
        self.open_partial = Some(PartialBuf {
            text,
            start_ms,
            end_ms,
            words,
        });
    }

    /// Commit a backend `Final` segment as an ordered turn. Clears any
    /// open partial. Returns a clone of the committed turn.
    pub fn commit_segment(&mut self, seg: Segment) -> SttTurn {
        self.bump_audio(seg.end_ms);
        let speaker = match seg.speaker.clone().or_else(|| self.current_speaker.clone()) {
            Some(tag) => SpeakerRef::Diarized { tag },
            None => SpeakerRef::Unknown,
        };
        let turn = SttTurn {
            index: self.turns.len() as u64,
            speaker,
            text: seg.text,
            start_ms: seg.start_ms,
            end_ms: seg.end_ms,
            words: seg.words,
            confidence: seg.confidence,
            state: TurnState::Final,
        };
        self.open_partial = None;
        self.turns.push(turn.clone());
        turn
    }

    /// Record a backend speaker-change marker. Subsequent turns without
    /// an explicit speaker inherit this until the next change.
    pub fn note_speaker_change(&mut self, speaker: SpeakerTag, _at_ms: u32) {
        self.current_speaker = Some(speaker);
    }

    /// On an utterance-end marker, commit any open partial as a final
    /// turn. Returns the committed turn, if there was one.
    pub fn close_open_turn(&mut self, at_ms: u32) -> Option<SttTurn> {
        let partial = self.open_partial.take()?;
        self.bump_audio(at_ms.max(partial.end_ms));
        let speaker = match self.current_speaker.clone() {
            Some(tag) => SpeakerRef::Diarized { tag },
            None => SpeakerRef::Unknown,
        };
        let turn = SttTurn {
            index: self.turns.len() as u64,
            speaker,
            text: partial.text,
            start_ms: partial.start_ms,
            end_ms: partial.end_ms,
            words: partial.words,
            confidence: None,
            state: TurnState::Final,
        };
        self.turns.push(turn.clone());
        Some(turn)
    }

    /// Override the display label for a diarized speaker. The numeric
    /// id is unchanged — every turn by that speaker picks the new label
    /// up via [`Self::effective_label`].
    pub fn rename_speaker(&mut self, speaker_id: u8, label: impl Into<String>) {
        self.speaker_labels.insert(speaker_id, label.into());
    }

    /// Resolve the display label for a speaker id: the per-conversation
    /// override wins, then the `speaker_{id}` fallback.
    pub fn effective_label(&self, speaker_id: u8) -> String {
        self.speaker_labels
            .get(&speaker_id)
            .cloned()
            .unwrap_or_else(|| format!("speaker_{speaker_id}"))
    }

    /// Distinct diarized speaker ids that appear in the conversation.
    pub fn speaker_ids(&self) -> Vec<u8> {
        let mut ids: Vec<u8> = self.turns.iter().filter_map(|t| t.speaker_id()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Map every committed turn to an agentic [`Message`].
    pub fn to_messages(&self, map: &SpeakerMap) -> Vec<Message> {
        self.turns
            .iter()
            .map(|turn| Message {
                role: map.role_for(turn),
                content: turn.text.clone(),
            })
            .collect()
    }

    /// Build a [`TurnInput`] for feeding an agent: the last committed
    /// turn becomes `user`, everything prior becomes `history`. Returns
    /// `None` when there are no committed turns yet.
    pub fn to_turn_input(&self, map: &SpeakerMap) -> Option<TurnInput> {
        let (last, rest) = self.turns.split_last()?;
        Some(TurnInput {
            user: last.text.clone(),
            history: rest
                .iter()
                .map(|turn| Message {
                    role: map.role_for(turn),
                    content: turn.text.clone(),
                })
                .collect(),
        })
    }

    /// Append an agent's reply as an `Assistant`-role turn so the
    /// conversation stays a complete record of the exchange.
    pub fn append_agent_reply(&mut self, text: impl Into<String>) -> SttTurn {
        let turn = SttTurn {
            index: self.turns.len() as u64,
            speaker: SpeakerRef::Role {
                role: MessageRole::Assistant,
            },
            text: text.into(),
            start_ms: 0,
            end_ms: 0,
            words: Vec::new(),
            confidence: None,
            state: TurnState::Final,
        };
        self.turns.push(turn.clone());
        turn
    }

    fn bump_audio(&mut self, end_ms: u32) {
        let secs = end_ms as f32 / 1000.0;
        if secs > self.total_audio_secs {
            self.total_audio_secs = secs;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_stt_core::Segment;

    fn seg(text: &str, speaker: Option<u8>) -> Segment {
        Segment {
            text: text.into(),
            start_ms: 0,
            end_ms: 0,
            words: vec![],
            speaker: speaker.map(|id| SpeakerTag { id, label: None }),
            confidence: Some(1.0),
        }
    }

    #[test]
    fn partials_then_final_commit_an_ordered_turn() {
        let mut c = SttConversation::new("c1");
        c.apply_partial("hel".into(), 0, 100, vec![]);
        assert!(c.open_partial.is_some());
        c.commit_segment(seg("hello", Some(0)));
        assert!(c.open_partial.is_none());
        assert_eq!(c.turns.len(), 1);
        assert_eq!(c.turns[0].index, 0);
        assert_eq!(c.turns[0].text, "hello");
        assert_eq!(c.turns[0].speaker_id(), Some(0));
    }

    #[test]
    fn rename_speaker_updates_effective_label_across_turns() {
        let mut c = SttConversation::new("c1");
        c.commit_segment(seg("hi", Some(0)));
        c.commit_segment(seg("there", Some(0)));
        assert_eq!(c.effective_label(0), "speaker_0");
        c.rename_speaker(0, "Alice");
        // Both turns by speaker 0 now resolve to the new label.
        for turn in &c.turns {
            assert_eq!(c.effective_label(turn.speaker_id().unwrap()), "Alice");
        }
    }

    #[test]
    fn to_turn_input_last_turn_is_user_rest_is_history() {
        let mut c = SttConversation::new("c1");
        c.commit_segment(seg("first", Some(0)));
        c.commit_segment(seg("second", Some(0)));
        c.commit_segment(seg("third", Some(0)));
        let ti = c.to_turn_input(&SpeakerMap::default()).unwrap();
        assert_eq!(ti.user, "third");
        assert_eq!(ti.history.len(), 2);
        assert_eq!(ti.history[0].content, "first");
        assert!(matches!(ti.history[0].role, MessageRole::User));
    }

    #[test]
    fn to_turn_input_is_none_when_empty() {
        let c = SttConversation::new("c1");
        assert!(c.to_turn_input(&SpeakerMap::default()).is_none());
    }

    #[test]
    fn append_agent_reply_round_trips_as_assistant() {
        let mut c = SttConversation::new("c1");
        c.commit_segment(seg("question?", Some(0)));
        c.append_agent_reply("answer.");
        let msgs = c.to_messages(&SpeakerMap::default());
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0].role, MessageRole::User));
        assert!(matches!(msgs[1].role, MessageRole::Assistant));
        assert_eq!(msgs[1].content, "answer.");
    }

    #[test]
    fn serde_round_trip_preserves_turns_and_labels() {
        let mut c = SttConversation::new("c1");
        c.commit_segment(seg("hi", Some(2)));
        c.rename_speaker(2, "Bob");
        let json = serde_json::to_string(&c).unwrap();
        let back: SttConversation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.turns.len(), 1);
        assert_eq!(back.effective_label(2), "Bob");
    }

    #[test]
    fn speaker_map_pairs_assign_roles() {
        let mut c = SttConversation::new("c1");
        c.commit_segment(seg("agent line", Some(1)));
        let map = SpeakerMap::from_pairs([(1u8, MessageRole::Assistant)]);
        let msgs = c.to_messages(&map);
        assert!(matches!(msgs[0].role, MessageRole::Assistant));
    }
}
