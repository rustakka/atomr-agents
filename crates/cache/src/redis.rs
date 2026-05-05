//! Redis-backed `LlmCache` stub. Real wiring lives in a deployment
//! patch.

#![cfg(feature = "redis")]

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};

use crate::{CacheKey, CachedTurn, LlmCache};

pub struct RedisLlmCache {
    pub url: String,
}

impl RedisLlmCache {
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        Ok(Self { url: url.into() })
    }
}

fn unsupported<T>() -> Result<T> {
    Err(AgentError::Internal(
        "RedisLlmCache: backend stub. Enable in your deployment patch.".into(),
    ))
}

#[async_trait]
impl LlmCache for RedisLlmCache {
    async fn get(&self, _key: &CacheKey) -> Result<Option<CachedTurn>> {
        unsupported()
    }
    async fn put(&self, _key: CacheKey, _value: CachedTurn) -> Result<()> {
        unsupported()
    }
}
