use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use parking_lot::RwLock;

pub type AnnId = u64;

/// Approximate-nearest-neighbor index. v0 ships an in-memory linear
/// scan; a CUDA / external-service implementation slots in behind
/// the same trait without changing call sites.
#[async_trait]
#[allow(clippy::len_without_is_empty)]
pub trait AnnIndex: Send + Sync + 'static {
    async fn upsert(&self, id: AnnId, vec: Vec<f32>) -> Result<()>;
    async fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<(AnnId, f32)>>;
    async fn len(&self) -> Result<usize>;
}

/// Linear-scan, cosine-similarity index. Mirrors the algorithm in
/// `atomr_accel_agents::CpuVectorIndex` but lives in-process to
/// avoid requiring an `ActorSystem` for unit tests.
pub struct InMemoryAnnIndex {
    dim: usize,
    inner: Arc<RwLock<Vec<(AnnId, Vec<f32>)>>>,
}

impl InMemoryAnnIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            inner: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if na == 0.0 || nb == 0.0 {
            0.0
        } else {
            dot / (na * nb)
        }
    }
}

#[async_trait]
impl AnnIndex for InMemoryAnnIndex {
    async fn upsert(&self, id: AnnId, vec: Vec<f32>) -> Result<()> {
        if vec.len() != self.dim {
            return Err(atomr_agents_core::AgentError::Internal(format!(
                "vector dim {} != index dim {}",
                vec.len(),
                self.dim
            )));
        }
        let mut g = self.inner.write();
        if let Some(slot) = g.iter_mut().find(|(i, _)| *i == id) {
            slot.1 = vec;
        } else {
            g.push((id, vec));
        }
        Ok(())
    }

    async fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<(AnnId, f32)>> {
        if query.len() != self.dim {
            return Err(atomr_agents_core::AgentError::Internal(format!(
                "query dim {} != index dim {}",
                query.len(),
                self.dim
            )));
        }
        let g = self.inner.read();
        let mut scored: Vec<(AnnId, f32)> = g.iter().map(|(id, v)| (*id, Self::cosine(v, query))).collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Ok(scored)
    }

    async fn len(&self) -> Result<usize> {
        Ok(self.inner.read().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn topk_works() {
        let idx = InMemoryAnnIndex::new(3);
        idx.upsert(1, vec![1.0, 0.0, 0.0]).await.unwrap();
        idx.upsert(2, vec![0.0, 1.0, 0.0]).await.unwrap();
        idx.upsert(3, vec![0.7, 0.7, 0.0]).await.unwrap();
        let r = idx.search(&[1.0, 0.0, 0.0], 2).await.unwrap();
        assert_eq!(r[0].0, 1);
        assert!((r[0].1 - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn dim_mismatch_errors() {
        let idx = InMemoryAnnIndex::new(3);
        let res = idx.upsert(1, vec![1.0, 0.0]).await;
        assert!(res.is_err());
    }
}
