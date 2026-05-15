//! Persistence for [`MeetingAnalysis`].
//!
//! Mirrors [`atomr_agents_stt_harness::ConversationStore`]: a trait the
//! web layer and any caller use to list, fetch, and mutate analyses;
//! an [`InMemoryMeetingsStore`] default; and (with feature `state`) a
//! [`CheckpointerMeetingsStore`] that routes through `crates/state`'s
//! [`Checkpointer`](atomr_agents_state::Checkpointer), so persistence
//! honours whichever backend the deployment configured.
//!
//! Analyses are keyed by the source transcript's `conversation_id`,
//! identical to how STT conversations are keyed — so the two records
//! join naturally in the same backend.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::analysis::{ActionStatus, AnalysisState, MeetingAnalysis};
use crate::error::{MeetingsHarnessError, Result};

/// Lightweight summary row for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingsSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub attendee_count: usize,
    pub note_count: usize,
    pub action_count: usize,
    pub open_action_count: usize,
    pub state: AnalysisState,
    pub generated_at_ms: i64,
    pub updated_at_ms: i64,
}

impl MeetingsSummary {
    pub fn of(a: &MeetingAnalysis) -> Self {
        Self {
            id: a.id.clone(),
            title: a.title.clone(),
            attendee_count: a.attendees.len(),
            note_count: a.notes.len(),
            action_count: a.actions.len(),
            open_action_count: a
                .actions
                .iter()
                .filter(|act| matches!(act.status, ActionStatus::Open))
                .count(),
            state: a.state,
            generated_at_ms: a.generated_at_ms,
            updated_at_ms: a.updated_at_ms,
        }
    }
}

/// Persistence surface for meeting analyses.
#[async_trait]
pub trait MeetingsStore: Send + Sync + 'static {
    async fn put(&self, analysis: &MeetingAnalysis) -> Result<()>;
    async fn get(&self, id: &str) -> Result<Option<MeetingAnalysis>>;
    async fn list(&self) -> Result<Vec<MeetingsSummary>>;
    async fn delete(&self, id: &str) -> Result<()>;

    /// Patch an attendee in place (rename / role / email). Returns the
    /// updated analysis. Read-modify-write; the edit lands in the
    /// backend the store wraps.
    async fn update_attendee(
        &self,
        id: &str,
        attendee_id: &str,
        display_name: Option<String>,
        role: Option<String>,
        email: Option<String>,
    ) -> Result<Option<MeetingAnalysis>> {
        let Some(mut a) = self.get(id).await? else {
            return Ok(None);
        };
        let Some(att) = a.attendee_mut(attendee_id) else {
            return Err(MeetingsHarnessError::tool(format!(
                "unknown attendee_id `{attendee_id}`"
            )));
        };
        if let Some(name) = display_name {
            att.display_name = name;
        }
        if role.is_some() {
            att.role = role;
        }
        if email.is_some() {
            att.email = email;
        }
        a.touch();
        self.put(&a).await?;
        Ok(Some(a))
    }

    /// Patch an action in place. Returns the updated analysis.
    async fn update_action(
        &self,
        id: &str,
        action_id: &str,
        status: Option<ActionStatus>,
        owner_attendee_id: Option<String>,
        due_iso: Option<String>,
    ) -> Result<Option<MeetingAnalysis>> {
        let Some(mut a) = self.get(id).await? else {
            return Ok(None);
        };
        if let Some(oid) = &owner_attendee_id {
            if a.attendee(oid).is_none() {
                return Err(MeetingsHarnessError::tool(format!(
                    "unknown owner_attendee_id `{oid}`"
                )));
            }
        }
        let Some(action) = a.action_mut(action_id) else {
            return Err(MeetingsHarnessError::tool(format!(
                "unknown action_id `{action_id}`"
            )));
        };
        if let Some(s) = status {
            action.status = s;
        }
        if owner_attendee_id.is_some() {
            action.owner_attendee_id = owner_attendee_id;
        }
        if let Some(due) = due_iso {
            action.due_iso = Some(due);
        }
        a.touch();
        self.put(&a).await?;
        Ok(Some(a))
    }
}

#[async_trait]
impl MeetingsStore for Arc<dyn MeetingsStore> {
    async fn put(&self, analysis: &MeetingAnalysis) -> Result<()> {
        (**self).put(analysis).await
    }
    async fn get(&self, id: &str) -> Result<Option<MeetingAnalysis>> {
        (**self).get(id).await
    }
    async fn list(&self) -> Result<Vec<MeetingsSummary>> {
        (**self).list().await
    }
    async fn delete(&self, id: &str) -> Result<()> {
        (**self).delete(id).await
    }
    async fn update_attendee(
        &self,
        id: &str,
        attendee_id: &str,
        display_name: Option<String>,
        role: Option<String>,
        email: Option<String>,
    ) -> Result<Option<MeetingAnalysis>> {
        (**self)
            .update_attendee(id, attendee_id, display_name, role, email)
            .await
    }
    async fn update_action(
        &self,
        id: &str,
        action_id: &str,
        status: Option<ActionStatus>,
        owner_attendee_id: Option<String>,
        due_iso: Option<String>,
    ) -> Result<Option<MeetingAnalysis>> {
        (**self)
            .update_action(id, action_id, status, owner_attendee_id, due_iso)
            .await
    }
}

/// Process-local, volatile analysis store.
#[derive(Clone, Default)]
pub struct InMemoryMeetingsStore {
    inner: Arc<RwLock<HashMap<String, MeetingAnalysis>>>,
}

impl InMemoryMeetingsStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MeetingsStore for InMemoryMeetingsStore {
    async fn put(&self, analysis: &MeetingAnalysis) -> Result<()> {
        self.inner.write().insert(analysis.id.clone(), analysis.clone());
        Ok(())
    }
    async fn get(&self, id: &str) -> Result<Option<MeetingAnalysis>> {
        Ok(self.inner.read().get(id).cloned())
    }
    async fn list(&self) -> Result<Vec<MeetingsSummary>> {
        let mut rows: Vec<MeetingsSummary> =
            self.inner.read().values().map(MeetingsSummary::of).collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }
    async fn delete(&self, id: &str) -> Result<()> {
        self.inner.write().remove(id);
        Ok(())
    }
}

#[cfg(feature = "state")]
mod checkpointer_store;
#[cfg(feature = "state")]
pub use checkpointer_store::CheckpointerMeetingsStore;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Action, ActionStatus, Attendee};

    fn analysis(id: &str) -> MeetingAnalysis {
        let mut a = MeetingAnalysis::new(id);
        a.attendees.push(Attendee {
            id: "att-1".into(),
            display_name: "Alice".into(),
            role: None,
            speaker_tags: vec![0],
            email: None,
        });
        a.actions.push(Action {
            id: "act-1".into(),
            description: "Ship".into(),
            owner_attendee_id: Some("att-1".into()),
            due_iso: None,
            supporting_quote: None,
            source_turn_index: None,
            status: ActionStatus::Open,
        });
        a
    }

    #[tokio::test]
    async fn in_memory_put_get_list_delete() {
        let store = InMemoryMeetingsStore::new();
        store.put(&analysis("a")).await.unwrap();
        store.put(&analysis("b")).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 2);
        assert!(store.get("a").await.unwrap().is_some());
        store.delete("a").await.unwrap();
        assert!(store.get("a").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn update_action_persists_through_store() {
        let store = InMemoryMeetingsStore::new();
        store.put(&analysis("c1")).await.unwrap();
        let updated = store
            .update_action("c1", "act-1", Some(ActionStatus::Done), None, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.actions[0].status, ActionStatus::Done);
        let reloaded = store.get("c1").await.unwrap().unwrap();
        assert_eq!(reloaded.actions[0].status, ActionStatus::Done);
    }

    #[tokio::test]
    async fn update_action_rejects_unknown_owner() {
        let store = InMemoryMeetingsStore::new();
        store.put(&analysis("c1")).await.unwrap();
        let err = store
            .update_action("c1", "act-1", None, Some("att-missing".into()), None)
            .await
            .unwrap_err();
        match err {
            MeetingsHarnessError::Tool(msg) => assert!(msg.contains("att-missing")),
            other => panic!("expected Tool error, got {other:?}"),
        }
    }
}
