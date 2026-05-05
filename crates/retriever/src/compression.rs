//! ContextualCompression: a model-driven extractive filter that
//! drops or shortens irrelevant chunks from a base retriever's output.
//!
//! The compression step is plug-in: in production it would call an
//! LLM to extract the relevant span from each doc; v0 ships a regex-
//! based filter that retains only sentences containing query terms.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};

use crate::retriever::{Document, Retriever};

#[async_trait]
pub trait CompressionStep: Send + Sync + 'static {
    async fn compress(&self, query: &str, doc: Document) -> Result<Option<Document>>;
}

/// Default: keep only sentences containing at least one query token,
/// drop docs that have none.
pub struct SentenceFilterCompressor;

#[async_trait]
impl CompressionStep for SentenceFilterCompressor {
    async fn compress(&self, query: &str, mut doc: Document) -> Result<Option<Document>> {
        let q_tokens: std::collections::HashSet<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let kept: Vec<&str> = doc
            .text
            .split('.')
            .filter(|s| {
                let lower = s.to_lowercase();
                q_tokens.iter().any(|t| lower.contains(t))
            })
            .collect();
        if kept.is_empty() {
            return Ok(None);
        }
        doc.text = kept.join(".").trim().to_string();
        Ok(Some(doc))
    }
}

pub struct ContextualCompressionRetriever {
    pub base: Arc<dyn Retriever>,
    pub step: Arc<dyn CompressionStep>,
}

impl ContextualCompressionRetriever {
    pub fn new(base: Arc<dyn Retriever>, step: Arc<dyn CompressionStep>) -> Self {
        Self { base, step }
    }
}

#[async_trait]
impl Retriever for ContextualCompressionRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let docs = self.base.retrieve(query, ctx).await?;
        let mut out = Vec::with_capacity(docs.len());
        for d in docs {
            if let Some(c) = self.step.compress(query, d).await? {
                out.push(c);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bm25::Bm25Retriever;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(5),
            trace: vec![],
        }
    }

    #[tokio::test]
    async fn compression_keeps_only_query_relevant_sentences() {
        let bm = Bm25Retriever::new(5);
        bm.add(Document::new(
            "d1",
            "Rust is fast. The weather is nice today. Cargo manages crates.",
        ));
        let base: Arc<dyn Retriever> = Arc::new(bm);
        let r = ContextualCompressionRetriever::new(base, Arc::new(SentenceFilterCompressor));
        let hits = r.retrieve("cargo crates", &ctx()).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].text.contains("weather"));
        assert!(hits[0].text.contains("Cargo manages"));
    }
}
