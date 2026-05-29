//! Deterministic, non-LLM retrieval filters: **point-in-time (as-of)**
//! and **compartment entitlement**, enforced uniformly below every
//! retriever.
//!
//! Motivation (lookahead-free + entitlement-enforced retrieval): a thesis
//! built on post-dated evidence yields unrealizable strategies on real
//! capital, and an analyst must see only documents in their compartments.
//! These guarantees cannot be entrusted to an LLM (e.g. SelfQuery's
//! generated predicates); [`FilteredRetriever`] wraps **any** [`Retriever`]
//! and drops inadmissible documents *after* the inner retriever runs, so
//! the guarantee holds for Bm25/Vector/MultiQuery/Ensemble/SelfQuery
//! uniformly. The filter AND-composes with (never OR) any SelfQuery
//! metadata predicate, because it is applied to the inner retriever's
//! already-filtered output.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{CallCtx, Result, Value};
use atomr_agents_security::{ClearanceContext, Compartment};

use crate::retriever::{Document, Retriever};

/// Query-time context carried alongside the query, describing the
/// point-in-time ceiling and the subject's clearance. Convertible into a
/// [`RetrieverFilter`] via [`RetrievalCtx::to_filter`].
#[derive(Debug, Clone, Default)]
pub struct RetrievalCtx {
    /// System-time ceiling (unix ms). Docs newer than this are dropped.
    pub as_of: Option<i64>,
    /// Subject clearance. Docs in compartments the subject lacks are dropped.
    pub clearance: Option<ClearanceContext>,
}

impl RetrievalCtx {
    pub fn as_of(ms: i64) -> Self {
        Self {
            as_of: Some(ms),
            clearance: None,
        }
    }

    pub fn with_clearance(mut self, c: ClearanceContext) -> Self {
        self.clearance = Some(c);
        self
    }

    /// Build the AND-composed [`RetrieverFilter`] this context implies.
    /// Returns `None` when neither an as-of ceiling nor a clearance is set.
    pub fn to_filter(&self) -> Option<RetrieverFilter> {
        let mut parts = Vec::new();
        if let Some(ms) = self.as_of {
            parts.push(RetrieverFilter::AsOf(ms));
        }
        if let Some(c) = &self.clearance {
            parts.push(RetrieverFilter::Entitlement(c.clone()));
        }
        match parts.len() {
            0 => None,
            1 => Some(parts.into_iter().next().unwrap()),
            _ => Some(RetrieverFilter::And(parts)),
        }
    }
}

/// A deterministic admission predicate over a [`Document`]'s metadata.
#[derive(Debug, Clone)]
pub enum RetrieverFilter {
    /// Drop docs whose `system_time` (or `ingested_at`) exceeds the unix-ms
    /// ceiling. Docs with no recorded time are **admitted** (treated as
    /// always-known); ingest should stamp `system_time` for lookahead-free
    /// guarantees on time-sensitive corpora.
    AsOf(i64),
    /// Drop docs whose `compartment` the subject does not hold. Docs with
    /// **no** compartment are public and always admitted.
    Entitlement(ClearanceContext),
    /// All sub-filters must admit (conjunctive; never OR).
    And(Vec<RetrieverFilter>),
}

/// Read a document's recorded system time (unix ms) from metadata,
/// trying `system_time` then `ingested_at`.
fn doc_system_time(meta: &Value) -> Option<i64> {
    meta.get("system_time")
        .or_else(|| meta.get("ingested_at"))
        .and_then(|v| v.as_i64())
}

/// Read a document's compartment from metadata (`compartment` string).
fn doc_compartment(meta: &Value) -> Option<Compartment> {
    meta.get("compartment")
        .and_then(|v| v.as_str())
        .map(Compartment::new)
}

impl RetrieverFilter {
    /// Whether `doc` is admitted by this filter.
    pub fn admits(&self, doc: &Document) -> bool {
        match self {
            RetrieverFilter::AsOf(ceiling) => match doc_system_time(&doc.metadata) {
                // Unknown time is admitted; a recorded time must be <= ceiling.
                Some(t) => t <= *ceiling,
                None => true,
            },
            RetrieverFilter::Entitlement(clearance) => match doc_compartment(&doc.metadata) {
                // No compartment => public => admitted.
                Some(c) => clearance.holds(&c),
                None => true,
            },
            RetrieverFilter::And(parts) => parts.iter().all(|p| p.admits(doc)),
        }
    }
}

/// A [`Retriever`] wrapper that applies a [`RetrieverFilter`] to the inner
/// retriever's results before returning them. Composes with any retriever,
/// so the as-of / entitlement guarantee holds uniformly — even through a
/// SelfQuery retriever, AND-composed with its LLM-generated predicates.
pub struct FilteredRetriever {
    inner: Arc<dyn Retriever>,
    filter: RetrieverFilter,
}

impl FilteredRetriever {
    pub fn new(inner: Arc<dyn Retriever>, filter: RetrieverFilter) -> Self {
        Self { inner, filter }
    }
}

#[async_trait]
impl Retriever for FilteredRetriever {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>> {
        let docs = self.inner.retrieve(query, ctx).await?;
        Ok(docs.into_iter().filter(|d| self.filter.admits(d)).collect())
    }
}

/// Ergonomic `retriever.with_filter(..)` constructor for any retriever.
pub trait WithFilter: Retriever + Sized {
    /// Wrap `self` in a [`FilteredRetriever`] applying `filter`.
    fn with_filter(self, filter: RetrieverFilter) -> FilteredRetriever {
        FilteredRetriever::new(Arc::new(self), filter)
    }
}

impl<R: Retriever + Sized> WithFilter for R {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bm25::Bm25Retriever;
    use crate::self_query::{KeyValueParser, SelfQueryRetriever};
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use atomr_agents_security::ClearanceLevel;
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(5),
            trace: vec![],
            extensions: Default::default(),
        }
    }

    fn doc(id: &str, text: &str, meta: Value) -> Document {
        let mut d = Document::new(id, text);
        d.metadata = meta;
        d
    }

    #[tokio::test]
    async fn as_of_drops_future_docs_keeps_past_and_undated() {
        let bm = Bm25Retriever::new(10);
        bm.add(doc("past", "rust news", serde_json::json!({"system_time": 100})));
        bm.add(doc("future", "rust news", serde_json::json!({"system_time": 300})));
        bm.add(doc("undated", "rust news", serde_json::json!({"author": "x"})));
        let r = bm.with_filter(RetrieverFilter::AsOf(200));
        let hits = r.retrieve("rust news", &ctx()).await.unwrap();
        let ids: std::collections::HashSet<_> = hits.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains("past"));
        assert!(ids.contains("undated"));
        assert!(!ids.contains("future"), "post-as_of doc must never be returned");
    }

    #[tokio::test]
    async fn entitlement_drops_out_of_compartment_keeps_public() {
        let bm = Bm25Retriever::new(10);
        bm.add(doc("held", "deal memo", serde_json::json!({"compartment": "deal:acme"})));
        bm.add(doc("other", "deal memo", serde_json::json!({"compartment": "deal:secret"})));
        bm.add(doc("public", "deal memo", serde_json::json!({"author": "x"})));
        let clearance =
            ClearanceContext::new("alice", ClearanceLevel::Confidential).with_compartment("deal:acme");
        let r = bm.with_filter(RetrieverFilter::Entitlement(clearance));
        let hits = r.retrieve("deal memo", &ctx()).await.unwrap();
        let ids: std::collections::HashSet<_> = hits.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains("held"));
        assert!(ids.contains("public"), "public (no-compartment) docs pass");
        assert!(!ids.contains("other"), "out-of-compartment doc must never be returned");
    }

    #[tokio::test]
    async fn enforced_below_self_query_and_composed() {
        // SelfQuery filters by metadata lang:rust (LLM-style predicate). The
        // as_of + entitlement filter wraps it and AND-composes.
        let bm = Bm25Retriever::new(10);
        bm.add(doc(
            "ok",
            "crate lang",
            serde_json::json!({"lang": "rust", "system_time": 100, "compartment": "deal:acme"}),
        ));
        bm.add(doc(
            "future",
            "crate lang",
            serde_json::json!({"lang": "rust", "system_time": 999, "compartment": "deal:acme"}),
        ));
        bm.add(doc(
            "wrongcompartment",
            "crate lang",
            serde_json::json!({"lang": "rust", "system_time": 100, "compartment": "deal:x"}),
        ));
        bm.add(doc(
            "wronglang",
            "crate lang",
            serde_json::json!({"lang": "python", "system_time": 100, "compartment": "deal:acme"}),
        ));
        let sq = SelfQueryRetriever::new(Arc::new(bm), Arc::new(KeyValueParser));
        let clearance =
            ClearanceContext::new("alice", ClearanceLevel::Confidential).with_compartment("deal:acme");
        let rctx = RetrievalCtx::as_of(200).with_clearance(clearance);
        let r = FilteredRetriever::new(Arc::new(sq), rctx.to_filter().unwrap());
        let hits = r.retrieve("crate lang lang:rust", &ctx()).await.unwrap();
        let ids: std::collections::HashSet<_> = hits.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("ok"));
        assert!(!ids.contains("future"));
        assert!(!ids.contains("wrongcompartment"));
        assert!(!ids.contains("wronglang"));
    }
}
