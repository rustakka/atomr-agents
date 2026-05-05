# Retrieval and ingestion

The two crates `agents-retriever` and `agents-ingest` cover the RAG
half of the framework. Together with `agents-memory::LongStore`,
they replace LangChain's retriever zoo + document loaders + text
splitters in a single composable surface.

## The Retriever trait

```rust
#[async_trait]
pub trait Retriever: Send + Sync + 'static {
    async fn retrieve(&self, query: &str, ctx: &CallCtx) -> Result<Vec<Document>>;
}

pub struct Document {
    pub id: String,
    pub text: String,
    pub metadata: Value,
    pub score: f32,
}
```

Every retriever returns ranked `Document`s. Wrap one as a `Tool`
to expose it to an agent, or chain retrievers (`Ensemble`,
`ContextualCompression`, `EmbeddingsFilter`) by Arc-cloning.

## Stock retrievers

| Retriever | What it does |
|---|---|
| `Bm25Retriever` | pure-Rust BM25 over an in-memory corpus; sparse baseline |
| `VectorRetriever` | dense retrieval over `LongStore::search`; uses any `Embedder` |
| `MultiQueryRetriever` | expand the query into N variants via a `QueryExpander` (LLM in production), union the results, dedupe by id |
| `ContextualCompressionRetriever` | run each result through a `CompressionStep`; ships `SentenceFilterCompressor` (regex-based) — swap for an LLM-extractive compressor in production |
| `ParentDocumentRetriever` | search small children, return their parent docs; dedupe by parent id |
| `EnsembleRetriever::with_rrf(...)` | Reciprocal Rank Fusion across N base retrievers; canonical hybrid (BM25 + dense) at `k=60` |
| `SelfQueryRetriever` | parse `key:value` filter tokens out of a NL query, forward the remainder to the base retriever, post-filter by metadata equality |
| `EmbeddingsFilter` | drop docs below a cosine threshold against the query embedding |
| `TimeWeightedRetriever` | recency-decay scoring on top of any base retriever; reads `ts_ms` from doc metadata |

Each is a `Retriever` itself, so they compose:

```rust
use std::sync::Arc;
use atomr_agents_retriever::*;

let bm25:   Arc<dyn Retriever> = Arc::new(Bm25Retriever::new(20));
let dense:  Arc<dyn Retriever> = Arc::new(VectorRetriever::new(store, embedder.clone(), ns, 20));

// Hybrid search: BM25 + dense fused with RRF, then squeeze through
// an embedding-filter cutoff, then trim to top-5 by recency.
let r: Arc<dyn Retriever> = Arc::new(
    TimeWeightedRetriever::new(
        Arc::new(EmbeddingsFilter::new(
            Arc::new(EnsembleRetriever::with_rrf(vec![bm25, dense], 20)),
            embedder,
            0.3,
        )),
        0.05,  // decay rate
    ),
);
```

## LongStore — long-term memory

`LongStore` is the cross-thread, namespace-tupled, embedding-indexed
storage that backs `VectorRetriever` and the memory tools.

```rust
use atomr_agents_memory::{InMemoryLongStore, LongStore, Namespace};

let store: Arc<dyn LongStore> = Arc::new(InMemoryLongStore::new());
let ns = Namespace::from_parts(["user", "alice", "facts"]);

store.put(&ns, "city", serde_json::json!("Boston"), Some(embedding_vec)).await?;
let hits = store.search(&ns, Some(&query_embedding), 5).await?;
let kids = store.list_namespaces(&Namespace::from_parts(["user"])).await?;
```

Namespace prefix matching means a search at `("user",)` returns
items from `("user", "alice", "facts")` and `("user", "bob",
"facts")` alike — useful for cross-user "all my data" queries.

Backend stubs for production storage live behind feature flags:
`pgvector`, `qdrant`, `chroma`. Each is a `LongStore` impl whose
real wire-up lives in a deployment patch.

## Memory tools (LangMem analogues)

Three `Tool` implementations let an agent write/update/recall
long-term memory directly via tool calls:

```rust
use atomr_agents_memory::{RecallMemoryTool, UpdateMemoryTool, WriteMemoryTool};

let tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(WriteMemoryTool::new(store.clone())),
    Arc::new(UpdateMemoryTool::new(store.clone())),
    Arc::new(RecallMemoryTool::new(store)),
];
```

The model sees `write_memory({namespace: ["user", "alice"], key:
"prefers_dark_mode", value: true})`, the tool calls `LongStore::put`,
and the next turn's `recall_memory({namespace: ["user", "alice"]})`
returns it. No special handling in the agent — these are normal
tools.

## Document ingestion

The four loaders + five splitters in `agents-ingest` cover the
common loader/splitter zoo:

```rust
use std::sync::Arc;
use atomr_agents_ingest::{
    CachedEmbedder, IngestPipeline, InMemoryKvCache, KvCache,
    Loader, MarkdownLoader, MarkdownHeaderSplitter,
    RecursiveCharacterSplitter, Splitter, ingest,
};
use atomr_agents_memory::Namespace;

// Load
let docs = MarkdownLoader(TextLoader::new(["docs/foo.md", "docs/bar.md"]))
    .load()
    .await?;

// Split (chained)
let pipeline = IngestPipeline::new()
    .then(Arc::new(MarkdownHeaderSplitter::default()))
    .then(Arc::new(RecursiveCharacterSplitter::new(800, 100)));
let chunks = pipeline.split_all(docs);

// Cache embeddings (avoid re-embed on rebuild)
let cache: Arc<dyn KvCache> = Arc::new(InMemoryKvCache::new());
let embedder = CachedEmbedder::new(real_embedder, cache, "text-embedding-3-small");

// Index
let n = ingest(&store, &Namespace::from_parts(["kb"]), &embedder, chunks).await?;
println!("ingested {n} chunks");
```

### Splitter cheat sheet

| Splitter | Use case |
|---|---|
| `RecursiveCharacterSplitter::new(chunk_size, overlap)` | general-purpose; greedy merge with paragraph/sentence/word fallbacks; supports `overlap` |
| `MarkdownHeaderSplitter` | one chunk per heading section; respects nesting up to `max_level` |
| `CodeSplitter { lang }` | break at top-level `fn` / `class` / `def` boundaries; v0 supports Rust / Python / JS |
| `TokenSplitter { max_tokens, overlap_tokens }` | budget-aware (whitespace-token approximation) |
| `SemanticSplitter::split_async(...)` | embed sentences, break at low-similarity boundaries; async-only |

### CachedEmbedder

`CachedEmbedder` is content-hash → vector lookup; halves redundant
embed calls on a re-ingest. The cache key is `(model_id,
sha256(text))` so changing the model invalidates automatically.

## Wiring a retriever into an agent tool

```rust
use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result, ToolId, Value};
use atomr_agents_retriever::Retriever;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};

struct SearchTool {
    inner: Arc<dyn Retriever>,
    descriptor: ToolDescriptor,
}

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> &ToolDescriptor { &self.descriptor }
    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value> {
        let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let docs = self.inner.retrieve(q, &ctx.call).await?;
        Ok(serde_json::json!({
            "hits": docs.iter().map(|d| serde_json::json!({
                "id": d.id, "text": d.text, "score": d.score,
            })).collect::<Vec<_>>()
        }))
    }
}
```

Now any retriever — single, filtered, ensembled, time-weighted — is
addressable from the agent's tool layer.

## Where to go from here

- [Agent pipeline](agent-pipeline.md) — wiring retriever-backed tools
  into the per-turn loop.
- [State and checkpointing](state-and-checkpointing.md) — durable
  storage of conversation history alongside `LongStore`.
- [Eval](eval.md) — eval suites for RAG: judge / pairwise scorers
  over retrieval+answer pairs.
