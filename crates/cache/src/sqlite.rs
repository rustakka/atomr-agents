//! SQLite-backed `LlmCache` stub.

#![cfg(feature = "sqlite")]

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};

use crate::{CacheKey, CachedTurn, LlmCache};

pub struct SqliteLlmCache {
    pub url: String,
}

impl SqliteLlmCache {
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        Ok(Self { url: url.into() })
    }
}

fn unsupported<T>() -> Result<T> {
    Err(AgentError::Internal(
        "SqliteLlmCache: backend stub. Enable in your deployment patch.".into(),
    ))
}

#[async_trait]
impl LlmCache for SqliteLlmCache {
    async fn get(&self, _key: &CacheKey) -> Result<Option<CachedTurn>> {
        unsupported()
    }
    async fn put(&self, _key: CacheKey, _value: CachedTurn) -> Result<()> {
        unsupported()
    }
}
