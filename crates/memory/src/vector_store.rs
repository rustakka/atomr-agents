//! Backend-agnostic [`VectorStore`] trait + backend-honored
//! [`MetadataFilter`] (FR-10).
//!
//! Motivation (production semantic layer): the retriever zoo needs a
//! concrete, swappable vector backend. The trait lives here (rather than
//! in `atomr-agents-embed`) because `embed` already depends on `memory`,
//! so this is the lowest crate both the embeddings layer and the
//! production backends (pgvector, Redis) can share without a dependency
//! cycle.
//!
//! [`MetadataFilter`] is honored **at query time** by the backend, not
//! post-filtered, so `k` is computed over the admissible set.

use async_trait::async_trait;
use atomr_agents_core::{Result, Value};
use serde::{Deserialize, Serialize};

/// A single vector-store search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub id: String,
    pub score: f32,
    #[serde(default)]
    pub metadata: Value,
}

/// A set of equality / range predicates over metadata keys, honored by
/// the backend at query time. `And` composes sub-predicates
/// conjunctively. [`MetadataFilter::matches`] is provided for in-memory
/// backends and as a documented fallback for backends that cannot push the
/// predicate down to the engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MetadataFilter {
    /// Matches everything (no restriction).
    #[default]
    All,
    /// `metadata[key] == value` (JSON equality).
    Eq(String, Value),
    /// `metadata[key] >= bound` (numeric).
    Gte(String, f64),
    /// `metadata[key] <= bound` (numeric).
    Lte(String, f64),
    /// All sub-predicates must hold.
    And(Vec<MetadataFilter>),
}

impl MetadataFilter {
    /// Conjoin two filters, flattening nested `And`s.
    pub fn and(self, other: MetadataFilter) -> MetadataFilter {
        match (self, other) {
            (MetadataFilter::All, o) => o,
            (s, MetadataFilter::All) => s,
            (MetadataFilter::And(mut a), MetadataFilter::And(b)) => {
                a.extend(b);
                MetadataFilter::And(a)
            }
            (MetadataFilter::And(mut a), o) => {
                a.push(o);
                MetadataFilter::And(a)
            }
            (s, MetadataFilter::And(mut b)) => {
                b.insert(0, s);
                MetadataFilter::And(b)
            }
            (s, o) => MetadataFilter::And(vec![s, o]),
        }
    }

    /// In-memory evaluation against a metadata object.
    pub fn matches(&self, metadata: &Value) -> bool {
        match self {
            MetadataFilter::All => true,
            MetadataFilter::Eq(k, v) => metadata.get(k).map(|m| m == v).unwrap_or(false),
            MetadataFilter::Gte(k, bound) => metadata
                .get(k)
                .and_then(|m| m.as_f64())
                .map(|n| n >= *bound)
                .unwrap_or(false),
            MetadataFilter::Lte(k, bound) => metadata
                .get(k)
                .and_then(|m| m.as_f64())
                .map(|n| n <= *bound)
                .unwrap_or(false),
            MetadataFilter::And(preds) => preds.iter().all(|p| p.matches(metadata)),
        }
    }
}

/// Backend-agnostic vector store: dense upsert, filtered top-k query,
/// and delete. In-memory impl lives in `atomr-agents-embed`; pgvector and
/// Redis backends live in this crate behind feature flags.
#[async_trait]
pub trait VectorStore: Send + Sync + 'static {
    /// Insert or replace `(id, embedding, metadata)` triples.
    async fn upsert(&self, items: Vec<(String, Vec<f32>, Value)>) -> Result<()>;
    /// Return up to `k` hits ranked by cosine similarity to `embedding`,
    /// restricted to rows whose metadata satisfies `filter`.
    async fn query(&self, embedding: Vec<f32>, k: usize, filter: &MetadataFilter) -> Result<Vec<Hit>>;
    /// Remove rows by id (missing ids are ignored).
    async fn delete(&self, ids: Vec<String>) -> Result<()>;
}

/// Cosine similarity (0 for mismatched / zero-norm vectors).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_filter_eq_gte_lte_and() {
        let m = serde_json::json!({"lang": "rust", "ts": 100});
        assert!(MetadataFilter::Eq("lang".into(), serde_json::json!("rust")).matches(&m));
        assert!(!MetadataFilter::Eq("lang".into(), serde_json::json!("python")).matches(&m));
        assert!(MetadataFilter::Gte("ts".into(), 50.0).matches(&m));
        assert!(MetadataFilter::Lte("ts".into(), 100.0).matches(&m));
        assert!(!MetadataFilter::Lte("ts".into(), 99.0).matches(&m));
        let and = MetadataFilter::Eq("lang".into(), serde_json::json!("rust"))
            .and(MetadataFilter::Gte("ts".into(), 50.0));
        assert!(and.matches(&m));
    }
}
