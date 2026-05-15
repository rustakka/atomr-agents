//! Persistence surface for completed (and in-flight) headless runs.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use atomr_agents_coding_cli_core::{CliResult, CliRunId};

use crate::error::HarnessError;

#[async_trait]
pub trait CliRunStore: Send + Sync {
    async fn put(&self, result: &CliResult) -> Result<(), HarnessError>;
    async fn get(&self, id: &CliRunId) -> Result<Option<CliResult>, HarnessError>;
    async fn list(&self, limit: usize) -> Result<Vec<CliResult>, HarnessError>;
}

#[derive(Default, Clone)]
pub struct InMemoryRunStore {
    inner: Arc<DashMap<CliRunId, CliResult>>,
}

impl InMemoryRunStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CliRunStore for InMemoryRunStore {
    async fn put(&self, result: &CliResult) -> Result<(), HarnessError> {
        self.inner.insert(result.run_id.clone(), result.clone());
        Ok(())
    }
    async fn get(&self, id: &CliRunId) -> Result<Option<CliResult>, HarnessError> {
        Ok(self.inner.get(id).map(|r| r.clone()))
    }
    async fn list(&self, limit: usize) -> Result<Vec<CliResult>, HarnessError> {
        let mut all: Vec<CliResult> = self.inner.iter().map(|kv| kv.value().clone()).collect();
        all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        all.truncate(limit);
        Ok(all)
    }
}
