//! High-level ingestion pipeline.

use std::sync::Arc;

use atomr_agents_core::{Result, Value};
use atomr_agents_embed::Embedder;
use atomr_agents_memory::{LongStore, Namespace};
use atomr_agents_retriever::Document;

use crate::splitters::Splitter;

/// Ingest-time metadata stamp for FR-11 retrieval filters.
///
/// Stamps `system_time` (unix ms ceiling used by the as-of filter) and
/// `compartment` (the entitlement wall used by the entitlement filter)
/// onto each [`Document`]'s metadata so deterministic, non-LLM retrieval
/// filters can enforce lookahead-free + entitlement-bounded retrieval.
///
/// Motivation (lookahead-free + entitlement-enforced retrieval): the
/// guarantees in FR-11 are only as good as the metadata the corpus
/// carries; stamping at ingest is where that metadata originates.
#[derive(Debug, Clone, Default)]
pub struct MetadataInjector {
    /// System time (unix ms) to stamp; if `None`, uses current time.
    pub system_time: Option<i64>,
    /// Compartment to stamp (omitted => public document).
    pub compartment: Option<String>,
}

impl MetadataInjector {
    /// Stamp with an explicit system time.
    pub fn at(system_time_ms: i64) -> Self {
        Self {
            system_time: Some(system_time_ms),
            compartment: None,
        }
    }

    /// Set the compartment wall.
    pub fn with_compartment(mut self, c: impl Into<String>) -> Self {
        self.compartment = Some(c.into());
        self
    }

    /// Apply the stamp to a single document, merging into existing
    /// object metadata (a non-object metadata is replaced with an object).
    pub fn apply(&self, doc: &mut Document) {
        let ts = self.system_time.unwrap_or_else(now_ms);
        let map = match &mut doc.metadata {
            Value::Object(m) => m,
            other => {
                *other = Value::Object(serde_json::Map::new());
                match other {
                    Value::Object(m) => m,
                    _ => unreachable!(),
                }
            }
        };
        map.entry("system_time".to_string())
            .or_insert_with(|| Value::from(ts));
        if let Some(c) = &self.compartment {
            map.insert("compartment".to_string(), Value::String(c.clone()));
        }
    }

    /// Apply the stamp to a batch, returning the stamped documents.
    pub fn apply_all(&self, mut docs: Vec<Document>) -> Vec<Document> {
        for d in &mut docs {
            self.apply(d);
        }
        docs
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Push documents through a chain of splitters then write each
/// resulting chunk into a `LongStore` with an embedding under the
/// supplied namespace.
pub async fn ingest(
    store: &dyn LongStore,
    namespace: &Namespace,
    embedder: &dyn Embedder,
    chunks: Vec<Document>,
) -> Result<usize> {
    let mut n = 0;
    for d in chunks {
        let v = embedder.embed(&d.text).await?;
        store
            .put(
                namespace,
                &d.id,
                serde_json::json!({ "text": d.text, "metadata": d.metadata }),
                Some(v),
            )
            .await?;
        n += 1;
    }
    Ok(n)
}

/// Builder that chains splitters and applies them to incoming docs.
pub struct IngestPipeline {
    splitters: Vec<Arc<dyn Splitter>>,
}

impl Default for IngestPipeline {
    fn default() -> Self {
        Self {
            splitters: Vec::new(),
        }
    }
}

impl IngestPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn then(mut self, s: Arc<dyn Splitter>) -> Self {
        self.splitters.push(s);
        self
    }

    pub fn split_all(&self, docs: Vec<Document>) -> Vec<Document> {
        let mut current = docs;
        for s in &self.splitters {
            let mut next = Vec::with_capacity(current.len());
            for d in &current {
                next.extend(s.split(d));
            }
            current = next;
        }
        current
    }
}

#[allow(dead_code)]
fn _value_in_scope(_v: Value) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splitters::{MarkdownHeaderSplitter, RecursiveCharacterSplitter};
    use atomr_agents_embed::MockEmbedder;
    use atomr_agents_memory::InMemoryLongStore;

    #[tokio::test]
    async fn end_to_end_ingest() {
        let pipeline = IngestPipeline::new()
            .then(Arc::new(MarkdownHeaderSplitter::default()))
            .then(Arc::new(RecursiveCharacterSplitter::new(200, 0)));
        let docs = vec![Document::new(
            "kb",
            "# Intro\nrust is a language\n# Cargo\ncargo manages crates\n",
        )];
        let chunks = pipeline.split_all(docs);
        assert!(chunks.len() >= 2);
        let store = InMemoryLongStore::new();
        let embedder = MockEmbedder::new(8);
        let n = ingest(&store, &Namespace::from_parts(["kb"]), &embedder, chunks)
            .await
            .unwrap();
        assert!(n >= 2);
        assert!(store.len() >= 2);
    }

    #[test]
    fn metadata_injector_stamps_system_time_and_compartment() {
        let inj = MetadataInjector::at(1_000).with_compartment("deal:acme");
        let mut d = Document::new("d", "text");
        d.metadata = serde_json::json!({"author": "x"});
        inj.apply(&mut d);
        assert_eq!(d.metadata.get("system_time").and_then(|v| v.as_i64()), Some(1_000));
        assert_eq!(d.metadata.get("compartment").and_then(|v| v.as_str()), Some("deal:acme"));
        // Preserves existing keys.
        assert_eq!(d.metadata.get("author").and_then(|v| v.as_str()), Some("x"));
    }

    #[test]
    fn metadata_injector_handles_null_metadata() {
        let inj = MetadataInjector::at(42);
        let mut d = Document::new("d", "text"); // metadata is Null
        inj.apply(&mut d);
        assert_eq!(d.metadata.get("system_time").and_then(|v| v.as_i64()), Some(42));
        // No compartment => public document (no compartment key).
        assert!(d.metadata.get("compartment").is_none());
    }
}
