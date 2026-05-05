//! Embedding cache.
//!
//! `KvCache` — content-hash → embedding lookup; in-process by default.
//! `CachedEmbedder` wraps any `Embedder` and consults the cache on
//! every `embed`. The key is `(model_id, sha256(text))`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_agents_embed::Embedder;
use parking_lot::RwLock;
use std::collections::HashMap;

#[async_trait]
pub trait KvCache: Send + Sync + 'static {
    async fn get(&self, key: &str) -> Result<Option<Vec<f32>>>;
    async fn put(&self, key: String, value: Vec<f32>) -> Result<()>;
}

#[derive(Default, Clone)]
pub struct InMemoryKvCache {
    inner: Arc<RwLock<HashMap<String, Vec<f32>>>>,
}

impl InMemoryKvCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }
}

#[async_trait]
impl KvCache for InMemoryKvCache {
    async fn get(&self, key: &str) -> Result<Option<Vec<f32>>> {
        Ok(self.inner.read().get(key).cloned())
    }
    async fn put(&self, key: String, value: Vec<f32>) -> Result<()> {
        self.inner.write().insert(key, value);
        Ok(())
    }
}

pub struct CachedEmbedder {
    pub inner: Arc<dyn Embedder>,
    pub cache: Arc<dyn KvCache>,
    pub model_id: String,
}

impl CachedEmbedder {
    pub fn new(inner: Arc<dyn Embedder>, cache: Arc<dyn KvCache>, model_id: impl Into<String>) -> Self {
        Self { inner, cache, model_id: model_id.into() }
    }

    fn key(&self, text: &str) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&(&self.model_id, text), &mut hasher);
        let h = std::hash::Hasher::finish(&hasher);
        format!("{}:{:016x}", self.model_id, h)
    }
}

#[async_trait]
impl Embedder for CachedEmbedder {
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let k = self.key(text);
        if let Some(v) = self.cache.get(&k).await? {
            return Ok(v);
        }
        let v = self.inner.embed(text).await?;
        self.cache.put(k, v.clone()).await?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_embed::MockEmbedder;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEmbedder {
        count: Arc<AtomicUsize>,
        inner: MockEmbedder,
    }
    #[async_trait]
    impl Embedder for CountingEmbedder {
        fn dim(&self) -> usize {
            self.inner.dim()
        }
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            self.inner.embed(text).await
        }
    }

    #[tokio::test]
    async fn cached_embedder_halves_calls_on_repeats() {
        let count = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn Embedder> = Arc::new(CountingEmbedder {
            count: count.clone(),
            inner: MockEmbedder::new(8),
        });
        let cache: Arc<dyn KvCache> = Arc::new(InMemoryKvCache::new());
        let c = CachedEmbedder::new(inner, cache, "mock");
        let _ = c.embed("hello").await.unwrap();
        let _ = c.embed("hello").await.unwrap();
        let _ = c.embed("world").await.unwrap();
        let _ = c.embed("hello").await.unwrap();
        // 4 calls, but only 2 distinct → 2 inner calls.
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
