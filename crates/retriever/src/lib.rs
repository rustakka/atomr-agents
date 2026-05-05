//! Retriever zoo.
//!
//! `Retriever` is the unifying trait. Stock impls cover Vector
//! (dense, via `agents-memory::LongStore`), BM25 (sparse, in-process
//! corpus), MultiQuery (LLM query expansion), ContextualCompression
//! (LLM extractive filter), ParentDocument (embed children, return
//! parents), Ensemble (Reciprocal Rank Fusion), SelfQuery (NL →
//! filter+query), EmbeddingsFilter (cosine threshold), and
//! TimeWeighted (recency decay).

mod bm25;
mod compression;
mod ensemble;
mod filter;
mod multi_query;
mod parent_doc;
mod retriever;
mod self_query;
mod time_weighted;
mod vector;

pub use bm25::Bm25Retriever;
pub use compression::{CompressionStep, ContextualCompressionRetriever};
pub use ensemble::EnsembleRetriever;
pub use filter::EmbeddingsFilter;
pub use multi_query::{MultiQueryRetriever, QueryExpander};
pub use parent_doc::ParentDocumentRetriever;
pub use retriever::{Document, Retriever};
pub use self_query::{ParsedSelfQuery, SelfQueryParser, SelfQueryRetriever};
pub use time_weighted::TimeWeightedRetriever;
pub use vector::VectorRetriever;
