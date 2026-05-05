//! EmbeddingsFilter — drop docs whose embedding similarity to the
//! query falls below a threshold.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};
use atomr_agents_embed::Embedder;

use crate::retriever::{Document, Retriever};

pub struct EmbeddingsFilter {
    pub base: Arc<dyn Retriever>,
    pub embedder: Arc<dyn Embedder>,
    pub threshold: f32,
}

impl EmbeddingsFilter {
    pub fn new(base: Arc<dyn Retriever>, embedder: Arc<dyn Embedder>, threshold: f32) -> Self {
        Self {
            base,
            embedder,
            threshold,
        }
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

#[async_trait]
impl Retriever for EmbeddingsFilter {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let q = self.embedder.embed(query).await?;
        let docs = self.base.retrieve(query, ctx).await?;
        let mut out = Vec::with_capacity(docs.len());
        for d in docs {
            let v = self.embedder.embed(&d.text).await?;
            let s = cosine(&q, &v);
            if s >= self.threshold {
                let mut d = d;
                d.score = s;
                out.push(d);
            }
        }
        Ok(out)
    }
}
