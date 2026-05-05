use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use parking_lot::RwLock;

use crate::{CacheKey, CachedTurn, LlmCache};

#[derive(Default, Clone)]
pub struct InMemoryLlmCache {
    inner: Arc<RwLock<HashMap<CacheKey, CachedTurn>>>,
}

impl InMemoryLlmCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }
}

#[async_trait]
impl LlmCache for InMemoryLlmCache {
    async fn get(&self, key: &CacheKey) -> Result<Option<CachedTurn>> {
        Ok(self.inner.read().get(key).cloned())
    }
    async fn put(&self, key: CacheKey, value: CachedTurn) -> Result<()> {
        self.inner.write().insert(key, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_infer_core::batch::{ExecuteBatch, Message, MessageContent, Role, SamplingParams};

    fn batch(model: &str, text: &str) -> ExecuteBatch {
        ExecuteBatch {
            request_id: "r".into(),
            model: model.into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text(text.into()),
            }],
            sampling: SamplingParams::default(),
            stream: false,
            estimated_tokens: 1,
        }
    }

    #[tokio::test]
    async fn key_collisions_only_on_identical_payload() {
        let a = CacheKey::from_batch(&batch("m", "hi"));
        let b = CacheKey::from_batch(&batch("m", "hi"));
        let c = CacheKey::from_batch(&batch("m", "different"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn put_get_round_trip() {
        let c = InMemoryLlmCache::new();
        let k = CacheKey::from_batch(&batch("m", "hello"));
        let v = CachedTurn {
            text: "hi back".into(),
            usage: Default::default(),
            finish_reason: None,
        };
        c.put(k.clone(), v.clone()).await.unwrap();
        let got = c.get(&k).await.unwrap().unwrap();
        assert_eq!(got.text, "hi back");
    }
}
