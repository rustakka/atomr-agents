use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{MemoryItem, MemoryNamespace, Result};
use parking_lot::RwLock;

#[async_trait]
pub trait MemoryStore: Send + Sync + 'static {
    async fn put(&self, item: MemoryItem) -> Result<()>;
    async fn list(&self, namespace: &MemoryNamespace, limit: usize) -> Result<Vec<MemoryItem>>;
}

/// Process-local in-memory store. Suitable for tests and the
/// no-cluster developer experience. Production deployments swap in a
/// store backed by `atomr-persistence`.
#[derive(Default, Clone)]
pub struct InMemoryStore {
    inner: Arc<RwLock<Vec<MemoryItem>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl MemoryStore for InMemoryStore {
    async fn put(&self, item: MemoryItem) -> Result<()> {
        self.inner.write().push(item);
        Ok(())
    }

    async fn list(&self, namespace: &MemoryNamespace, limit: usize) -> Result<Vec<MemoryItem>> {
        let g = self.inner.read();
        let mut out: Vec<MemoryItem> = g
            .iter()
            .filter(|i| namespaces_match(&i.namespace, namespace))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        out.truncate(limit);
        Ok(out)
    }
}

fn namespaces_match(a: &MemoryNamespace, b: &MemoryNamespace) -> bool {
    use MemoryNamespace::*;
    match (a, b) {
        (Agent(x), Agent(y)) => x.as_str() == y.as_str(),
        (Team(x), Team(y)) => x.as_str() == y.as_str(),
        (Org(x), Org(y)) => x.as_str() == y.as_str(),
        _ => false,
    }
}
