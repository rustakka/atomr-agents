//! Persistence for [`ResearchResult`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_deep_research_core::{ResearchResult, ResearchState};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Lightweight summary row for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSummary {
    pub id: String,
    pub query: String,
    pub strategy: String,
    pub state: ResearchState,
    pub citation_count: usize,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

impl ResearchSummary {
    pub fn of(r: &ResearchResult) -> Self {
        Self {
            id: r.id.clone(),
            query: r.query.clone(),
            strategy: r.strategy.clone(),
            state: r.state,
            citation_count: r.citations.len(),
            created_at_ms: r.created_at_ms,
            updated_at_ms: r.updated_at_ms,
            model_id: r.model_id.clone(),
        }
    }
}

#[async_trait]
pub trait ResearchStore: Send + Sync + 'static {
    async fn put(&self, result: &ResearchResult) -> Result<()>;
    async fn get(&self, id: &str) -> Result<Option<ResearchResult>>;
    async fn list(&self) -> Result<Vec<ResearchSummary>>;
    async fn delete(&self, id: &str) -> Result<()>;
}

#[async_trait]
impl ResearchStore for Arc<dyn ResearchStore> {
    async fn put(&self, r: &ResearchResult) -> Result<()> {
        (**self).put(r).await
    }
    async fn get(&self, id: &str) -> Result<Option<ResearchResult>> {
        (**self).get(id).await
    }
    async fn list(&self) -> Result<Vec<ResearchSummary>> {
        (**self).list().await
    }
    async fn delete(&self, id: &str) -> Result<()> {
        (**self).delete(id).await
    }
}

/// Process-local, volatile store.
#[derive(Clone, Default)]
pub struct InMemoryResearchStore {
    inner: Arc<RwLock<HashMap<String, ResearchResult>>>,
}

impl InMemoryResearchStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ResearchStore for InMemoryResearchStore {
    async fn put(&self, r: &ResearchResult) -> Result<()> {
        self.inner.write().insert(r.id.clone(), r.clone());
        Ok(())
    }
    async fn get(&self, id: &str) -> Result<Option<ResearchResult>> {
        Ok(self.inner.read().get(id).cloned())
    }
    async fn list(&self) -> Result<Vec<ResearchSummary>> {
        let mut rows: Vec<ResearchSummary> = self.inner.read().values().map(ResearchSummary::of).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.updated_at_ms));
        Ok(rows)
    }
    async fn delete(&self, id: &str) -> Result<()> {
        self.inner.write().remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::ResearchResult;

    #[tokio::test]
    async fn in_memory_round_trip() {
        let store = InMemoryResearchStore::new();
        let r = ResearchResult::new("q", "s");
        let id = r.id.clone();
        store.put(&r).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert!(store.get(&id).await.unwrap().is_some());
        store.delete(&id).await.unwrap();
        assert!(store.get(&id).await.unwrap().is_none());
    }
}
