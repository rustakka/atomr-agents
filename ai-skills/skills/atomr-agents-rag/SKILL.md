---
name: atomr-agents-rag
description: Use when building a retriever pipeline (BM25 / dense / hybrid / contextual-compression / parent-document / self-query / time-weighted), ingesting documents (loaders + splitters), or wiring `LongStore` into an agent. Triggers on `Bm25Retriever::new`, `VectorRetriever::new`, `EnsembleRetriever::with_rrf`, `RecursiveCharacterSplitter`, `LongStore::put`, or porting a LangChain retriever zoo.
---

# RAG in atomr-agents

Three crates cover the RAG half of the framework:

- `agents-memory::LongStore` — namespace-tupled, embedding-indexed,
  cross-thread storage.
- `agents-retriever` — the retriever zoo (BM25 / Vector / MultiQuery
  / ContextualCompression / ParentDocument / Ensemble (RRF) /
  SelfQuery / EmbeddingsFilter / TimeWeighted).
- `agents-ingest` — loaders + splitters + `CachedEmbedder` +
  one-call `ingest()` helper.

## Mental model

- A **`Retriever`** takes a query string, returns ranked
  `Document`s. Every retriever is `Callable`-shaped (sync trait, but
  composes around `Arc::clone`).
- A **`LongStore`** is the persistent backing for dense retrieval —
  but also for `WriteMemoryTool` / `RecallMemoryTool` / cross-thread
  user memory.
- A **`Splitter`** is sync; a **semantic splitter** is async-only
  because it embeds. A loader is async.
- An **`Ensemble`** retriever composes multiple base retrievers via
  Reciprocal Rank Fusion (`k = 60` by default). This is how you do
  hybrid sparse + dense.

## Picking a retriever

| Need | Retriever |
|---|---|
| no embedder; pure lexical | `Bm25Retriever::new(top_k)` |
| dense; one corpus | `VectorRetriever::new(store, embedder, namespace, top_k)` |
| hybrid sparse + dense | `EnsembleRetriever::with_rrf(vec![bm25, dense], top_k)` |
| LLM expands the query | `MultiQueryRetriever::new(base, expander, n_variants)` |
| LLM trims / extracts | `ContextualCompressionRetriever::new(base, step)` |
| embed children, return parents | `ParentDocumentRetriever::new(child_retriever)` |
| metadata filter via `key:value` | `SelfQueryRetriever::new(base, parser)` |
| cosine cutoff | `EmbeddingsFilter::new(base, embedder, threshold)` |
| recency decay | `TimeWeightedRetriever::new(base, decay_rate)` |

Compose freely:

```rust
use std::sync::Arc;
use atomr_agents_retriever::{
    Bm25Retriever, EmbeddingsFilter, EnsembleRetriever, Retriever, TimeWeightedRetriever,
    VectorRetriever,
};

let bm25:  Arc<dyn Retriever> = Arc::new(Bm25Retriever::new(20));
let dense: Arc<dyn Retriever> = Arc::new(VectorRetriever::new(store, embedder.clone(), ns, 20));

let r: Arc<dyn Retriever> = Arc::new(
    TimeWeightedRetriever::new(
        Arc::new(EmbeddingsFilter::new(
            Arc::new(EnsembleRetriever::with_rrf(vec![bm25, dense], 20)),
            embedder,
            0.3,
        )),
        0.05,
    ),
);
```

## LongStore — long-term memory

```rust
use std::sync::Arc;
use atomr_agents_memory::{InMemoryLongStore, LongStore, Namespace};

let store: Arc<dyn LongStore> = Arc::new(InMemoryLongStore::new());

// Put with optional embedding for semantic search.
let ns = Namespace::from_parts(["user", "alice", "facts"]);
store.put(&ns, "city", serde_json::json!("Boston"), Some(embedding)).await?;

// Retrieve by exact key.
let v = store.get(&ns, "city").await?;

// Search semantically (passing None for query_embedding sorts by recency instead).
let hits = store.search(&ns, Some(&query_embedding), 5).await?;

// Namespace prefix search.
let cross_user = store.search(&Namespace::from_parts(["user"]), None, 50).await?;
```

For production, swap to a feature-gated backend:

```toml
atomr-agents-memory = { version = "0.2", features = ["pgvector"] }
```

```rust
use atomr_agents_memory::PgvectorStore;
let store = Arc::new(PgvectorStore::connect("postgres://…").await?);
```

(The shipped `PgvectorStore` is a stub; wire to your deployment patch.)

## Memory tools (LangMem)

Three built-in `Tool`s let an agent write/recall long-term memory
directly:

```rust
use atomr_agents_memory::{RecallMemoryTool, UpdateMemoryTool, WriteMemoryTool};

let tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(WriteMemoryTool::new(store.clone())),
    Arc::new(UpdateMemoryTool::new(store.clone())),
    Arc::new(RecallMemoryTool::new(store)),
];
```

The model sees `write_memory({namespace, key, value})` as a normal
tool call.

## Ingestion pipeline

```rust
use std::sync::Arc;
use atomr_agents_ingest::{
    CachedEmbedder, IngestPipeline, InMemoryKvCache, KvCache, Loader,
    MarkdownHeaderSplitter, MarkdownLoader, RecursiveCharacterSplitter, TextLoader, ingest,
};

let docs = MarkdownLoader(TextLoader::new(["docs/foo.md", "docs/bar.md"]))
    .load()
    .await?;

let pipeline = IngestPipeline::new()
    .then(Arc::new(MarkdownHeaderSplitter::default()))
    .then(Arc::new(RecursiveCharacterSplitter::new(800, 100)));
let chunks = pipeline.split_all(docs);

let cache: Arc<dyn KvCache> = Arc::new(InMemoryKvCache::new());
let embedder = CachedEmbedder::new(real_embedder, cache, "text-embedding-3-small");

let n = ingest(&store, &Namespace::from_parts(["kb"]), &embedder, chunks).await?;
```

### Splitter cheat sheet

| Splitter | Picks when |
|---|---|
| `RecursiveCharacterSplitter::new(chunk, overlap)` | general-purpose; greedy with separator fallbacks |
| `MarkdownHeaderSplitter` | one chunk per `#` heading section |
| `CodeSplitter { lang: CodeLang::{Rust, Python, Js} }` | top-level fn/class/struct boundaries |
| `TokenSplitter { max_tokens, overlap_tokens }` | budget-aware; whitespace approximation |
| `SemanticSplitter::split_async(...)` | embed sentences, break at low-similarity boundaries; async |

`CachedEmbedder` keys on `(model_id, sha256(text))` — changing the
model invalidates automatically.

## Wiring a retriever as a tool

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

## Lineage queries via `atomr_agents_memory::query`

When you want to replay or audit what a memory-backed agent did
historically (event-sourced journal sitting behind a `LongStore`-backed
strategy), wrap the underlying `atomr_persistence::Journal` in
`SimpleReadJournal` and use the `ReadJournal` surface re-exported from
`atomr_agents_memory::query`:

```rust
use atomr_agents_memory::query::{Offset, ReadJournal, SimpleReadJournal};

let read = SimpleReadJournal::new(my_journal_arc);

// All events tagged "agent:a-1", in offset order.
let envs = read.events_by_tag("agent:a-1", Offset::NoOffset).await?;

// Distinct persistence ids known to this backend.
let ids = read.all_persistence_ids().await?;
```

`SimpleReadJournal` has a default-impl `events_by_tag` that returns
empty for backends without tag indexing — production journals (SQL,
Cassandra) implement it natively. Use this for eval replays, lineage
audits, and "rebuild this run from durable state" flows.

## Canonical references

- [`docs/retrieval-and-ingestion.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/retrieval-and-ingestion.md)
- [`crates/retriever/`](https://github.com/rustakka/atomr-agents/tree/main/crates/retriever) — every retriever impl
- [`crates/ingest/src/splitters.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/ingest/src/splitters.rs)

## Common mistakes

- **Mismatched embedder dim.** `LongStore` stores raw `Vec<f32>`;
  searching with a different-dim query returns 0.0 cosine.
- **Forgetting to embed on `put`.** `LongStore::put(.., None)`
  stores value but won't surface in semantic `search`.
- **`Bm25Retriever` against a too-small corpus.** BM25 needs ≥
  ~100 docs to discriminate. Below that, dense usually wins.
- **`MultiQueryRetriever` with a real LLM expander on every call.**
  N-fold the model spend; cache or use a small expander.
- **`SemanticSplitter` chunks at sentence breaks but returns one
  giant chunk for technical docs without periods.** Pre-split with
  `RecursiveCharacterSplitter` first.
- **`EnsembleRetriever` with `top_k` smaller than its members'
  `top_k`.** RRF needs candidate breadth; let members return more
  than the ensemble caps.
