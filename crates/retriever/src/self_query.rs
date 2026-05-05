//! SelfQuery: NL query → (filter, query). The filter is metadata-
//! based; the rewritten query is what the base retriever sees.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result, Value};

use crate::retriever::{Document, Retriever};

#[derive(Debug, Clone)]
pub struct ParsedSelfQuery {
    pub query: String,
    /// Map of metadata key → required value.
    pub filter: Vec<(String, Value)>,
}

#[async_trait]
pub trait SelfQueryParser: Send + Sync + 'static {
    async fn parse(&self, query: &str) -> Result<ParsedSelfQuery>;
}

/// Trivial parser: looks for `key:value` tokens, extracts them as
/// filter, and returns the remaining text as the query.
pub struct KeyValueParser;

#[async_trait]
impl SelfQueryParser for KeyValueParser {
    async fn parse(&self, query: &str) -> Result<ParsedSelfQuery> {
        let mut filter = Vec::new();
        let mut q_parts = Vec::new();
        for tok in query.split_whitespace() {
            if let Some((k, v)) = tok.split_once(':') {
                filter.push((k.to_string(), Value::String(v.to_string())));
            } else {
                q_parts.push(tok);
            }
        }
        Ok(ParsedSelfQuery { query: q_parts.join(" "), filter })
    }
}

pub struct SelfQueryRetriever {
    pub base: Arc<dyn Retriever>,
    pub parser: Arc<dyn SelfQueryParser>,
}

impl SelfQueryRetriever {
    pub fn new(base: Arc<dyn Retriever>, parser: Arc<dyn SelfQueryParser>) -> Self {
        Self { base, parser }
    }
}

#[async_trait]
impl Retriever for SelfQueryRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let parsed = self.parser.parse(query).await?;
        let hits = self.base.retrieve(&parsed.query, ctx).await?;
        if parsed.filter.is_empty() {
            return Ok(hits);
        }
        Ok(hits
            .into_iter()
            .filter(|d| {
                parsed.filter.iter().all(|(k, v)| {
                    d.metadata.get(k).map(|m| m == v).unwrap_or(false)
                })
            })
            .collect())
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
    async fn self_query_filters_by_metadata() {
        let bm = Bm25Retriever::new(5);
        let mut d1 = Document::new("d1", "rust crate");
        d1.metadata = serde_json::json!({"lang": "rust"});
        let mut d2 = Document::new("d2", "rust crate");
        d2.metadata = serde_json::json!({"lang": "python"});
        bm.add(d1);
        bm.add(d2);
        let r = SelfQueryRetriever::new(Arc::new(bm), Arc::new(KeyValueParser));
        let hits = r.retrieve("crate lang:rust", &ctx()).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "d1");
    }
}
