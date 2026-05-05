//! Document ingestion: loaders, splitters, embedding cache, and a
//! one-call `ingest` helper that wires it all together.

mod cache;
mod ingest;
mod loaders;
mod splitters;

pub use cache::{CachedEmbedder, InMemoryKvCache, KvCache};
pub use ingest::{ingest, IngestPipeline};
pub use loaders::{CsvLoader, JsonLoader, Loader, MarkdownLoader, TextLoader};
pub use splitters::{
    CodeLang, CodeSplitter, MarkdownHeaderSplitter, RecursiveCharacterSplitter, SemanticSplitter, Splitter,
    TokenSplitter,
};
