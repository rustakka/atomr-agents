//! Dedup / novelty primitive + incremental append-only ingest hook.
//!
//! Motivation (alert-fatigue dedup): opportunity-discovery must not
//! re-surface the same signal. [`NoveltyRetriever`] gives a single, shared
//! similarity definition (one [`VectorStore`] + [`Embeddings`] config) so
//! every desk scores novelty the same way instead of hand-rolling
//! thresholds. [`IncrementalIngest`] writes novel items append-only and
//! reports merges for near-duplicates; re-ingesting an identical item is a
//! no-op ([`IngestOutcome::Merged`]).

use std::sync::Arc;

use atomr_agents_core::{Result, Value};
use atomr_agents_embed::{Embeddings, MetadataFilter, VectorStore};

/// How to resolve a near-duplicate when ingesting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Merge {
    /// Keep the first-seen item; drop the new duplicate.
    #[default]
    KeepFirst,
    /// Overwrite the existing item with the latest content.
    KeepLatest,
    /// Treat near-duplicates as one cluster (report the merge target).
    Cluster,
}

/// Result of a novelty assessment.
#[derive(Debug, Clone, PartialEq)]
pub struct Novelty {
    /// Highest cosine similarity to any existing corpus item (0 if empty).
    pub max_similarity: f64,
    /// Whether the item is novel (`max_similarity < threshold`).
    pub is_novel: bool,
    /// `(id, score)` of the most-similar existing items, highest first.
    pub merge_candidates: Vec<(String, f32)>,
}

/// Policy controlling [`IncrementalIngest::ingest_if_novel`].
#[derive(Debug, Clone, Copy)]
pub struct NoveltyPolicy {
    /// Similarity at/above which an item is considered a duplicate.
    pub threshold: f64,
    /// Merge semantics for duplicates.
    pub merge: Merge,
}

impl Default for NoveltyPolicy {
    fn default() -> Self {
        Self {
            threshold: 0.95,
            merge: Merge::KeepFirst,
        }
    }
}

/// Outcome of an incremental ingest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// The item was novel and was written under this id.
    Ingested { id: String },
    /// The item duplicated an existing item; no new row was written
    /// (or the existing row was refreshed, per [`Merge::KeepLatest`]).
    Merged { into: String },
}

/// Novelty/dedup scorer built on a [`VectorStore`] + [`Embeddings`].
/// Shares its embeddings + similarity definition with [`IncrementalIngest`].
pub struct NoveltyRetriever {
    store: Arc<dyn VectorStore>,
    embeddings: Arc<dyn Embeddings>,
    /// Similarity at/above which an item is a duplicate.
    pub threshold: f64,
    /// How many merge candidates to surface.
    pub top_k: usize,
}

impl NoveltyRetriever {
    pub fn new(store: Arc<dyn VectorStore>, embeddings: Arc<dyn Embeddings>, threshold: f64) -> Self {
        Self {
            store,
            embeddings,
            threshold,
            top_k: 5,
        }
    }

    /// Embed `item` and score it against the corpus.
    pub async fn assess(&self, item: &str) -> Result<Novelty> {
        let emb = self.embed_one(item).await?;
        let hits = self.store.query(emb, self.top_k, &MetadataFilter::All).await?;
        let max_similarity = hits.first().map(|h| h.score as f64).unwrap_or(0.0);
        Ok(Novelty {
            max_similarity,
            is_novel: max_similarity < self.threshold,
            merge_candidates: hits.into_iter().map(|h| (h.id, h.score)).collect(),
        })
    }

    async fn embed_one(&self, item: &str) -> Result<Vec<f32>> {
        let mut v = self.embeddings.embed(vec![item.to_string()]).await?;
        Ok(v.pop().unwrap_or_default())
    }
}

/// Append-only incremental ingest sharing one similarity definition with
/// [`NoveltyRetriever`]. Novel items are upserted into the [`VectorStore`];
/// duplicates are reported as merges (and optionally refreshed).
pub struct IncrementalIngest {
    novelty: NoveltyRetriever,
}

impl IncrementalIngest {
    pub fn new(store: Arc<dyn VectorStore>, embeddings: Arc<dyn Embeddings>, threshold: f64) -> Self {
        Self {
            novelty: NoveltyRetriever::new(store, embeddings, threshold),
        }
    }

    /// Ingest `item` under `id` only if it is novel under `policy`.
    /// Re-ingesting an identical item is idempotent (returns `Merged`).
    pub async fn ingest_if_novel(
        &self,
        id: &str,
        item: &str,
        policy: NoveltyPolicy,
    ) -> Result<IngestOutcome> {
        let emb = self.novelty.embed_one(item).await?;
        let hits = self
            .novelty
            .store
            .query(emb.clone(), self.novelty.top_k, &MetadataFilter::All)
            .await?;
        let best = hits.first();
        let is_dup = best.map(|h| h.score as f64 >= policy.threshold).unwrap_or(false);

        if is_dup {
            let into = best.unwrap().id.clone();
            match policy.merge {
                Merge::KeepFirst | Merge::Cluster => Ok(IngestOutcome::Merged { into }),
                Merge::KeepLatest => {
                    // Refresh the existing row's content in place.
                    self.novelty
                        .store
                        .upsert(vec![(into.clone(), emb, item_metadata(id, item))])
                        .await?;
                    Ok(IngestOutcome::Merged { into })
                }
            }
        } else {
            self.novelty
                .store
                .upsert(vec![(id.to_string(), emb, item_metadata(id, item))])
                .await?;
            Ok(IngestOutcome::Ingested { id: id.to_string() })
        }
    }
}

fn item_metadata(id: &str, text: &str) -> Value {
    serde_json::json!({ "id": id, "text": text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_embed::{InMemoryVectorStore, MockEmbedder};

    fn parts() -> (Arc<dyn VectorStore>, Arc<dyn Embeddings>) {
        (
            Arc::new(InMemoryVectorStore::new()),
            Arc::new(MockEmbedder::new(32)),
        )
    }

    #[tokio::test]
    async fn novelty_threshold_distinguishes_new_from_dup() {
        let (store, emb) = parts();
        store
            .upsert(vec![(
                "existing".into(),
                emb.embed(vec!["rust async runtime".into()]).await.unwrap().pop().unwrap(),
                serde_json::json!({"text": "rust async runtime"}),
            )])
            .await
            .unwrap();
        let nov = NoveltyRetriever::new(store, emb, 0.99);
        // Identical text -> high similarity -> not novel.
        let same = nov.assess("rust async runtime").await.unwrap();
        assert!(!same.is_novel);
        assert!(same.max_similarity >= 0.99);
        assert!(!same.merge_candidates.is_empty());
        // Different text -> novel.
        let diff = nov.assess("python pandas dataframe").await.unwrap();
        assert!(diff.is_novel);
    }

    #[tokio::test]
    async fn ingest_writes_novel_and_merges_duplicate() {
        let concrete = InMemoryVectorStore::new();
        let store: Arc<dyn VectorStore> = Arc::new(concrete.clone());
        let emb: Arc<dyn Embeddings> = Arc::new(MockEmbedder::new(32));
        let ing = IncrementalIngest::new(store, emb, 0.99);
        let policy = NoveltyPolicy {
            threshold: 0.99,
            merge: Merge::KeepFirst,
        };
        let r1 = ing.ingest_if_novel("a", "rust async runtime", policy).await.unwrap();
        assert_eq!(r1, IngestOutcome::Ingested { id: "a".into() });

        // A genuinely different item is also ingested.
        let r2 = ing.ingest_if_novel("b", "python pandas dataframe", policy).await.unwrap();
        assert_eq!(r2, IngestOutcome::Ingested { id: "b".into() });

        // Re-ingest an identical item -> idempotent no-op (Merged into "a").
        let r3 = ing.ingest_if_novel("a2", "rust async runtime", policy).await.unwrap();
        assert_eq!(r3, IngestOutcome::Merged { into: "a".into() });
        assert_eq!(concrete.len(), 2, "duplicate must not add a new row");
    }

    #[tokio::test]
    async fn keep_latest_refreshes_in_place() {
        let concrete = InMemoryVectorStore::new();
        let store: Arc<dyn VectorStore> = Arc::new(concrete.clone());
        let emb: Arc<dyn Embeddings> = Arc::new(MockEmbedder::new(32));
        let ing = IncrementalIngest::new(store, emb, 0.99);
        let policy = NoveltyPolicy {
            threshold: 0.99,
            merge: Merge::KeepLatest,
        };
        ing.ingest_if_novel("a", "rust async runtime", policy).await.unwrap();
        let r = ing.ingest_if_novel("a2", "rust async runtime", policy).await.unwrap();
        assert_eq!(r, IngestOutcome::Merged { into: "a".into() });
        assert_eq!(concrete.len(), 1);
    }
}
