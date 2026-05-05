use async_trait::async_trait;
use atomr_agents_core::Result;

/// Single text → vector. The default implementation in production
/// will wrap an `atomr-infer` `ModelRunner` configured with an
/// embedding model; for unit tests, `MockEmbedder` produces
/// deterministic vectors from a hash.
#[async_trait]
pub trait Embedder: Send + Sync + 'static {
    fn dim(&self) -> usize;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
}

/// Deterministic, dimension-configurable mock embedder. Produces
/// stable vectors so tests are reproducible.
pub struct MockEmbedder {
    dim: usize,
}

impl MockEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    fn hash_to_vec(&self, text: &str) -> Vec<f32> {
        let mut state: u64 = 0xcbf2_9ce4_8422_2325;
        let mut v = vec![0.0f32; self.dim];
        for (i, b) in text.bytes().enumerate() {
            state ^= b as u64;
            state = state.wrapping_mul(0x100_0000_01b3);
            let slot = i % self.dim;
            // Map the rotated state to a [-1, 1] float.
            let f = ((state >> 32) as i32) as f32 / i32::MAX as f32;
            v[slot] += f;
        }
        // Normalize to unit length so cosine similarity behaves.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

#[async_trait]
impl Embedder for MockEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.hash_to_vec(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deterministic() {
        let e = MockEmbedder::new(8);
        let v1 = e.embed("hello").await.unwrap();
        let v2 = e.embed("hello").await.unwrap();
        let v3 = e.embed("world").await.unwrap();
        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
        assert_eq!(v1.len(), 8);
    }
}
