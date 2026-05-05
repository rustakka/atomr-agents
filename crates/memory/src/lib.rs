//! Memory stores and strategies.

mod backends;
mod long_term;
mod memory_tools;
mod recency;
mod store;
mod summarizing;

pub use long_term::{InMemoryLongStore, LongStore, Namespace, StoreItem};
pub use memory_tools::{RecallMemoryTool, UpdateMemoryTool, WriteMemoryTool};
pub use recency::RecencyMemoryStrategy;
pub use store::{InMemoryStore, MemoryStore};
pub use summarizing::SummarizingMemoryStrategy;

pub use atomr_agents_strategy::{ChainedMemoryStrategy, MemoryStrategy};

#[cfg(feature = "chroma")]
pub use backends::chroma::ChromaStore;
#[cfg(feature = "pgvector")]
pub use backends::pgvector::PgvectorStore;
#[cfg(feature = "qdrant")]
pub use backends::qdrant::QdrantStore;
