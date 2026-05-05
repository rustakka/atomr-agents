//! High-level ingestion pipeline.

use std::sync::Arc;

use atomr_agents_core::{Result, Value};
use atomr_agents_embed::Embedder;
use atomr_agents_memory::{LongStore, Namespace};
use atomr_agents_retriever::Document;

use crate::splitters::Splitter;

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
}
