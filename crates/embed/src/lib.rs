//! EmbeddingActor + pluggable AnnIndex.

mod ann;
mod embedder;
mod tool_strategy;

pub use ann::{AnnId, AnnIndex, InMemoryAnnIndex};
pub use embedder::{Embedder, MockEmbedder};
pub use tool_strategy::EmbeddingToolStrategy;
