//! ParentDocumentRetriever: embed children, return parents.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result};

use crate::retriever::{Document, Retriever};

/// Caller registers (parent_id, child_id) mappings; queries are run
/// against `child_retriever`, and matches are returned as the parent
/// docs.
pub struct ParentDocumentRetriever {
    pub child_retriever: Arc<dyn Retriever>,
    parents: HashMap<String, Document>,
    child_to_parent: HashMap<String, String>,
}

impl ParentDocumentRetriever {
    pub fn new(child_retriever: Arc<dyn Retriever>) -> Self {
        Self { child_retriever, parents: HashMap::new(), child_to_parent: HashMap::new() }
    }

    pub fn add(&mut self, parent: Document, child_ids: Vec<String>) {
        let pid = parent.id.clone();
        self.parents.insert(pid.clone(), parent);
        for c in child_ids {
            self.child_to_parent.insert(c, pid.clone());
        }
    }
}

#[async_trait]
impl Retriever for ParentDocumentRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let child_hits = self.child_retriever.retrieve(query, ctx).await?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for c in child_hits {
            if let Some(pid) = self.child_to_parent.get(&c.id) {
                if seen.insert(pid.clone()) {
                    if let Some(p) = self.parents.get(pid) {
                        let mut p = p.clone();
                        p.score = c.score;
                        out.push(p);
                    }
                }
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
    async fn parent_returned_for_child_match() {
        let bm = Bm25Retriever::new(5);
        bm.add(Document::new("c1", "rust"));
        bm.add(Document::new("c2", "tokio"));
        bm.add(Document::new("c3", "python"));
        let mut p = ParentDocumentRetriever::new(Arc::new(bm));
        p.add(
            Document::new("p1", "Parent doc about Rust runtime"),
            vec!["c1".into(), "c2".into()],
        );
        p.add(
            Document::new("p2", "Parent doc about Python data"),
            vec!["c3".into()],
        );
        let hits = p.retrieve("rust", &ctx()).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "p1");
    }
}
