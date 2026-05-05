//! Annotation queue for human review.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{Result, RunId, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pending,
    Approved,
    Rejected,
    NeedsEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationItem {
    pub id: String,
    pub run_id: RunId,
    pub prompt: String,
    pub output: Value,
    pub verdict: Verdict,
    pub note: Option<String>,
    pub created_at_ms: i64,
}

#[async_trait]
pub trait AnnotationQueue: Send + Sync + 'static {
    async fn enqueue(&self, item: AnnotationItem) -> Result<()>;
    async fn next_pending(&self) -> Result<Option<AnnotationItem>>;
    async fn submit(&self, id: &str, verdict: Verdict, note: Option<String>) -> Result<()>;
    async fn list(&self) -> Result<Vec<AnnotationItem>>;
}

#[derive(Default, Clone)]
pub struct InMemoryAnnotationQueue {
    inner: Arc<RwLock<Vec<AnnotationItem>>>,
}

impl InMemoryAnnotationQueue {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AnnotationQueue for InMemoryAnnotationQueue {
    async fn enqueue(&self, item: AnnotationItem) -> Result<()> {
        self.inner.write().push(item);
        Ok(())
    }

    async fn next_pending(&self) -> Result<Option<AnnotationItem>> {
        Ok(self
            .inner
            .read()
            .iter()
            .find(|i| matches!(i.verdict, Verdict::Pending))
            .cloned())
    }

    async fn submit(&self, id: &str, verdict: Verdict, note: Option<String>) -> Result<()> {
        let mut g = self.inner.write();
        if let Some(item) = g.iter_mut().find(|i| i.id == id) {
            item.verdict = verdict;
            item.note = note;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<AnnotationItem>> {
        Ok(self.inner.read().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> AnnotationItem {
        AnnotationItem {
            id: id.into(),
            run_id: RunId::from(format!("run-{id}")),
            prompt: "hi".into(),
            output: Value::String("answer".into()),
            verdict: Verdict::Pending,
            note: None,
            created_at_ms: 0,
        }
    }

    #[tokio::test]
    async fn enqueue_then_submit_updates_verdict() {
        let q = InMemoryAnnotationQueue::new();
        q.enqueue(item("a")).await.unwrap();
        q.enqueue(item("b")).await.unwrap();
        let next = q.next_pending().await.unwrap().unwrap();
        assert_eq!(next.id, "a");
        q.submit("a", Verdict::Approved, Some("ok".into())).await.unwrap();
        let next = q.next_pending().await.unwrap().unwrap();
        assert_eq!(next.id, "b");
        let all = q.list().await.unwrap();
        assert_eq!(all[0].verdict, Verdict::Approved);
        assert_eq!(all[0].note.as_deref(), Some("ok"));
    }
}
