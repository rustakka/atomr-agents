//! LLM cache.
//!
//! `LlmCache` is the trait. Stock backends:
//! - `InMemoryLlmCache` (always available)
//! - `SemanticLlmCache<E>` — embeds the prompt; returns a cached
//!   answer if a previous prompt was within `threshold` cosine
//!   distance. Useful for "near-duplicate" cache hits.
//! - `SqliteLlmCache` (feature `sqlite`) and `RedisLlmCache`
//!   (feature `redis`): backend stubs whose real wire-up lives in a
//!   deployment patch.

mod inmem;
mod redis;
mod semantic;
mod sqlite;

pub use inmem::InMemoryLlmCache;
pub use semantic::SemanticLlmCache;

#[cfg(feature = "redis")]
pub use redis::RedisLlmCache;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteLlmCache;

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_infer_core::tokens::{FinishReason, TokenUsage};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub model: String,
    pub messages_hash: u64,
    pub sampling_hash: u64,
}

impl CacheKey {
    pub fn from_batch(batch: &atomr_infer_core::batch::ExecuteBatch) -> Self {
        let mut mh = std::collections::hash_map::DefaultHasher::new();
        for m in &batch.messages {
            (m.role as u8).hash(&mut mh);
            let s = serde_json::to_string(&m.content).unwrap_or_default();
            s.hash(&mut mh);
        }
        let messages_hash = mh.finish();
        let mut sh = std::collections::hash_map::DefaultHasher::new();
        let sampling = serde_json::to_string(&batch.sampling).unwrap_or_default();
        sampling.hash(&mut sh);
        let sampling_hash = sh.finish();
        Self { model: batch.model.clone(), messages_hash, sampling_hash }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTurn {
    pub text: String,
    pub usage: TokenUsage,
    pub finish_reason: Option<FinishReason>,
}

#[async_trait]
pub trait LlmCache: Send + Sync + 'static {
    async fn get(&self, key: &CacheKey) -> Result<Option<CachedTurn>>;
    async fn put(&self, key: CacheKey, value: CachedTurn) -> Result<()>;
}
