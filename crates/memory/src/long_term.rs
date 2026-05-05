//! Long-term, cross-thread `Store` — LangGraph's `Store` API.
//!
//! Items live in a tuple namespace (e.g. `("user", "alice", "facts")`)
//! and are addressed by a key. Optional embedding-indexed fields make
//! `search` rank by cosine similarity. Backends shipped: in-memory.
//! pgvector / sqlite-vss adapters land in Phase R17.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Namespace(pub Vec<String>);

impl Namespace {
    pub fn from_parts<I: IntoIterator<Item = impl Into<String>>>(parts: I) -> Self {
        Self(parts.into_iter().map(Into::into).collect())
    }

    /// Whether `self` is a (proper or equal) prefix of `other`.
    pub fn is_prefix_of(&self, other: &Namespace) -> bool {
        if self.0.len() > other.0.len() {
            return false;
        }
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a == b)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreItem {
    pub namespace: Namespace,
    pub key: String,
    pub value: Value,
    pub embedding: Option<Vec<f32>>,
    pub score: f32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[async_trait]
pub trait LongStore: Send + Sync + 'static {
    async fn put(
        &self,
        namespace: &Namespace,
        key: &str,
        value: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<()>;
    async fn get(&self, namespace: &Namespace, key: &str) -> Result<Option<StoreItem>>;
    async fn delete(&self, namespace: &Namespace, key: &str) -> Result<()>;
    /// Returns up to `top_k` items ranked by cosine similarity if a
    /// query embedding is provided; otherwise by `updated_at_ms`
    /// descending.
    async fn search(
        &self,
        namespace: &Namespace,
        query_embedding: Option<&[f32]>,
        top_k: usize,
    ) -> Result<Vec<StoreItem>>;
    /// List sub-namespaces under `prefix`.
    async fn list_namespaces(&self, prefix: &Namespace) -> Result<Vec<Namespace>>;
}

#[derive(Default, Clone)]
pub struct InMemoryLongStore {
    inner: Arc<RwLock<Vec<StoreItem>>>,
}

impl InMemoryLongStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl LongStore for InMemoryLongStore {
    async fn put(
        &self,
        namespace: &Namespace,
        key: &str,
        value: Value,
        embedding: Option<Vec<f32>>,
    ) -> Result<()> {
        let now = now_ms();
        let mut g = self.inner.write();
        if let Some(slot) = g
            .iter_mut()
            .find(|i| i.namespace.0 == namespace.0 && i.key == key)
        {
            slot.value = value;
            if embedding.is_some() {
                slot.embedding = embedding;
            }
            slot.updated_at_ms = now;
        } else {
            g.push(StoreItem {
                namespace: namespace.clone(),
                key: key.to_string(),
                value,
                embedding,
                score: 0.0,
                created_at_ms: now,
                updated_at_ms: now,
            });
        }
        Ok(())
    }

    async fn get(&self, namespace: &Namespace, key: &str) -> Result<Option<StoreItem>> {
        Ok(self
            .inner
            .read()
            .iter()
            .find(|i| i.namespace.0 == namespace.0 && i.key == key)
            .cloned())
    }

    async fn delete(&self, namespace: &Namespace, key: &str) -> Result<()> {
        self.inner
            .write()
            .retain(|i| !(i.namespace.0 == namespace.0 && i.key == key));
        Ok(())
    }

    async fn search(
        &self,
        namespace: &Namespace,
        query_embedding: Option<&[f32]>,
        top_k: usize,
    ) -> Result<Vec<StoreItem>> {
        let g = self.inner.read();
        let mut hits: Vec<StoreItem> = g
            .iter()
            .filter(|i| namespace.is_prefix_of(&i.namespace))
            .cloned()
            .collect();
        if let Some(q) = query_embedding {
            for h in &mut hits {
                if let Some(e) = &h.embedding {
                    h.score = cosine(q, e);
                } else {
                    h.score = 0.0;
                }
            }
            hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            hits.sort_by_key(|i| std::cmp::Reverse(i.updated_at_ms));
        }
        hits.truncate(top_k);
        Ok(hits)
    }

    async fn list_namespaces(&self, prefix: &Namespace) -> Result<Vec<Namespace>> {
        let g = self.inner.read();
        let mut out: Vec<Namespace> = Vec::new();
        for i in g.iter() {
            if prefix.is_prefix_of(&i.namespace) && !out.iter().any(|n| n.0 == i.namespace.0) {
                out.push(i.namespace.clone());
            }
        }
        Ok(out)
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

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)]
fn _agent_error_in_scope() -> AgentError {
    AgentError::Memory("placeholder".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_overwrite() {
        let s = InMemoryLongStore::new();
        let ns = Namespace::from_parts(["user", "alice", "facts"]);
        s.put(&ns, "city", serde_json::json!("Boston"), None)
            .await
            .unwrap();
        s.put(&ns, "city", serde_json::json!("NYC"), None).await.unwrap();
        let v = s.get(&ns, "city").await.unwrap().unwrap();
        assert_eq!(v.value, serde_json::json!("NYC"));
        assert_eq!(s.len(), 1);
    }

    #[tokio::test]
    async fn search_ranks_by_cosine_when_embedding() {
        let s = InMemoryLongStore::new();
        let ns = Namespace::from_parts(["user", "alice"]);
        s.put(&ns, "a", serde_json::json!("alpha"), Some(vec![1.0, 0.0]))
            .await
            .unwrap();
        s.put(&ns, "b", serde_json::json!("beta"), Some(vec![0.0, 1.0]))
            .await
            .unwrap();
        let hits = s.search(&ns, Some(&[1.0, 0.0]), 5).await.unwrap();
        assert_eq!(hits[0].key, "a");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn cascade_namespace_search_with_prefix() {
        let s = InMemoryLongStore::new();
        let alice = Namespace::from_parts(["user", "alice", "facts"]);
        let bob = Namespace::from_parts(["user", "bob", "facts"]);
        s.put(&alice, "x", serde_json::json!("a"), None).await.unwrap();
        s.put(&bob, "x", serde_json::json!("b"), None).await.unwrap();
        let prefix = Namespace::from_parts(["user"]);
        let hits = s.search(&prefix, None, 10).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn list_namespaces_with_prefix() {
        let s = InMemoryLongStore::new();
        s.put(
            &Namespace::from_parts(["user", "alice", "facts"]),
            "x",
            serde_json::json!(1),
            None,
        )
        .await
        .unwrap();
        s.put(
            &Namespace::from_parts(["user", "alice", "preferences"]),
            "y",
            serde_json::json!(2),
            None,
        )
        .await
        .unwrap();
        s.put(
            &Namespace::from_parts(["user", "bob", "facts"]),
            "z",
            serde_json::json!(3),
            None,
        )
        .await
        .unwrap();
        let nss = s
            .list_namespaces(&Namespace::from_parts(["user", "alice"]))
            .await
            .unwrap();
        assert_eq!(nss.len(), 2);
    }
}
