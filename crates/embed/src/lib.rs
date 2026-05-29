//! EmbeddingActor + pluggable AnnIndex.

mod ann;
mod embedder;
mod tool_strategy;
mod vector_store;

pub use ann::{AnnId, AnnIndex, InMemoryAnnIndex};
pub use embedder::{Embedder, MockEmbedder};
pub use tool_strategy::EmbeddingToolStrategy;
pub use vector_store::{cosine, Embeddings, Hit, InMemoryVectorStore, MetadataFilter, VectorStore};
