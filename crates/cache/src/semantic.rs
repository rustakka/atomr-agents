//! Semantic LLM cache.
//!
//! Embeds the concatenated user-message text; on `get`, returns the
//! cached value of the most-similar previous prompt if cosine
//! similarity ≥ `threshold`.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_agents_embed::Embedder;
use parking_lot::RwLock;

use crate::{CacheKey, CachedTurn, LlmCache};

struct Entry {
    embedding: Vec<f32>,
    value: CachedTurn,
    /// Original key so exact-key hits also work.
    key: CacheKey,
    /// The text used to embed (we hash it back for keying when retrieving).
    text: String,
}

pub struct SemanticLlmCache {
    pub embedder: Arc<dyn Embedder>,
    pub threshold: f32,
    inner: Arc<RwLock<Vec<Entry>>>,
}

impl SemanticLlmCache {
    pub fn new(embedder: Arc<dyn Embedder>, threshold: f32) -> Self {
        Self {
            embedder,
            threshold,
            inner: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Variant `get` keyed by the prompt text rather than the
    /// hash-based `CacheKey`. Useful when the caller has the original
    /// prompt available.
    pub async fn get_by_text(&self, text: &str) -> Result<Option<CachedTurn>> {
        let q = self.embedder.embed(text).await?;
        let g = self.inner.read();
        let mut best: Option<(f32, CachedTurn)> = None;
        for e in g.iter() {
            let s = cosine(&q, &e.embedding);
            if s >= self.threshold {
                if best.as_ref().map(|(b, _)| s > *b).unwrap_or(true) {
                    best = Some((s, e.value.clone()));
                }
            }
        }
        Ok(best.map(|(_, v)| v))
    }

    pub async fn put_with_text(
        &self,
        text: impl Into<String>,
        key: CacheKey,
        value: CachedTurn,
    ) -> Result<()> {
        let text = text.into();
        let v = self.embedder.embed(&text).await?;
        self.inner.write().push(Entry {
            embedding: v,
            value,
            key,
            text,
        });
        Ok(())
    }
}

#[async_trait]
impl LlmCache for SemanticLlmCache {
    async fn get(&self, key: &CacheKey) -> Result<Option<CachedTurn>> {
        // Exact-key first.
        if let Some(v) = self
            .inner
            .read()
            .iter()
            .find(|e| &e.key == key)
            .map(|e| e.value.clone())
        {
            return Ok(Some(v));
        }
        // No prompt text available without re-deriving from the key
        // (which is hash-only). Fall back to "miss" — callers that
        // want semantic matching should call `get_by_text` directly.
        Ok(None)
    }
    async fn put(&self, _key: CacheKey, _value: CachedTurn) -> Result<()> {
        // Hashed key alone isn't enough to embed; require `put_with_text`.
        Err(atomr_agents_core::AgentError::Internal(
            "SemanticLlmCache: use put_with_text() so the prompt text can be embedded".into(),
        ))
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[allow(dead_code)]
fn _entry_in_scope(_e: &Entry) {}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_embed::MockEmbedder;
    use atomr_infer_core::tokens::TokenUsage;

    fn turn(text: &str) -> CachedTurn {
        CachedTurn {
            text: text.into(),
            usage: TokenUsage::default(),
            finish_reason: None,
        }
    }

    #[tokio::test]
    async fn hits_on_near_duplicate_prompt() {
        let c = SemanticLlmCache::new(Arc::new(MockEmbedder::new(8)), 0.99);
        let key = CacheKey {
            model: "m".into(),
            messages_hash: 1,
            sampling_hash: 1,
        };
        c.put_with_text("hello", key, turn("hi back")).await.unwrap();
        let v = c.get_by_text("hello").await.unwrap().unwrap();
        assert_eq!(v.text, "hi back");
    }

    #[tokio::test]
    async fn miss_below_threshold() {
        let c = SemanticLlmCache::new(Arc::new(MockEmbedder::new(8)), 0.999);
        let key = CacheKey {
            model: "m".into(),
            messages_hash: 1,
            sampling_hash: 1,
        };
        c.put_with_text("hello", key, turn("hi back")).await.unwrap();
        let v = c.get_by_text("entirely different prompt").await.unwrap();
        assert!(v.is_none());
    }
}
