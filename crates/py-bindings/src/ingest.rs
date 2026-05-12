//! `atomr-agents-ingest` bindings — loaders, splitters, embedding
//! cache, and a small builder around `IngestPipeline` that runs loader
//! → splitters → embed → long-store as a single `Callable`.
//!
//! Layout mirrors `embed.rs` / `memory.rs`: dyn handles for `Loader`,
//! `Splitter`, and `KvCache`; concrete factories for the in-repo
//! implementations; and `*_from_factory(key)` helpers that wrap a
//! Python-side guest target registered via `guest.register_*_factory`.
//!
//! Limitations:
//! - `SemanticSplitter` is `AsyncSplitter`-only in the upstream crate
//!   and cannot be added to `IngestPipeline.then(...)` (which takes a
//!   sync `Splitter`). The `semantic_splitter(...)` factory is exposed
//!   for direct use against a single document via `split` (eager,
//!   blocking on the tokio runtime), but it is not chainable inside a
//!   pipeline. Callers that need semantic splitting should run it as a
//!   preprocessing stage themselves.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{CallableHandle, FnCallable};
use atomr_agents_core::{AgentError, CallCtx, Result as AgentResult, Value};
use atomr_agents_embed::Embedder;
use atomr_agents_ingest::{
    ingest as ingest_fn, CachedEmbedder, CodeLang, CodeSplitter, CsvLoader, InMemoryKvCache,
    IngestPipeline, JsonLoader, KvCache, Loader, MarkdownHeaderSplitter, MarkdownLoader,
    RecursiveCharacterSplitter, SemanticSplitter, Splitter, TextLoader, TokenSplitter,
};
use atomr_agents_memory::{LongStore, Namespace};
use atomr_agents_retriever::Document;
use pyo3::prelude::*;

use crate::callable::PyCallable;
use crate::conv::{json_to_py, py_to_json};
use crate::embed::PyEmbedder;
use crate::memory::{PyLongStore, PyNamespace};
use crate::strategy::await_if_coro;

// =============================================================================
// PyDocument — `Document` value type
// =============================================================================

#[pyclass(name = "Document", module = "atomr_agents._native.ingest")]
#[derive(Clone)]
pub struct PyDocument {
    pub(crate) inner: Document,
}

#[pymethods]
impl PyDocument {
    #[new]
    #[pyo3(signature = (id, text, metadata=None, score=0.0))]
    fn new(
        py: Python<'_>,
        id: String,
        text: String,
        metadata: Option<&Bound<'_, PyAny>>,
        score: f32,
    ) -> PyResult<Self> {
        let md = match metadata {
            Some(b) if !b.is_none() => py_to_json(py, b)?,
            _ => Value::Null,
        };
        Ok(Self {
            inner: Document {
                id,
                text,
                metadata: md,
                score,
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    #[getter]
    fn metadata(&self, py: Python<'_>) -> PyResult<PyObject> {
        json_to_py(py, &self.inner.metadata)
    }

    #[getter]
    fn score(&self) -> f32 {
        self.inner.score
    }

    fn __repr__(&self) -> String {
        format!(
            "Document(id={:?}, text_len={}, score={:.3})",
            self.inner.id,
            self.inner.text.len(),
            self.inner.score
        )
    }
}

// =============================================================================
// PyLoader — dyn handle around `Arc<dyn Loader>`
// =============================================================================

#[pyclass(name = "Loader", module = "atomr_agents._native.ingest")]
#[derive(Clone)]
pub struct PyLoader {
    pub(crate) inner: Arc<dyn Loader>,
}

#[pymethods]
impl PyLoader {
    /// `await loader.load()` → `list[Document]`.
    fn load<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let docs = inner.load().await.map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                let out = pyo3::types::PyList::empty_bound(py);
                for d in docs {
                    out.append(Py::new(py, PyDocument { inner: d })?)?;
                }
                Ok(out.unbind().into())
            })
        })
    }

    fn __repr__(&self) -> String {
        "Loader(handle)".into()
    }
}

// ----- Python guest adapter ------------------------------------------------

pub(crate) struct PyLoaderAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl Loader for PyLoaderAdapter {
    async fn load(&self) -> AgentResult<Vec<Document>> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("load")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("load")?.call0()?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py loader: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<Document>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let it = item?;
                let pd: PyDocument = it.extract()?;
                out.push(pd.inner);
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("py loader result: {e}")))
    }
}

// ----- Loader factories ----------------------------------------------------

#[pyfunction]
fn text_loader(paths: Vec<String>) -> PyLoader {
    PyLoader {
        inner: Arc::new(TextLoader::new(paths)),
    }
}

#[pyfunction]
fn markdown_loader(paths: Vec<String>) -> PyLoader {
    PyLoader {
        inner: Arc::new(MarkdownLoader(TextLoader::new(paths))),
    }
}

/// CSV loader. `text_field` names the header column whose value becomes
/// each `Document.text`; remaining columns land in `metadata`.
#[pyfunction]
#[pyo3(signature = (paths, text_field="text".to_string()))]
fn csv_loader(paths: Vec<String>, text_field: String) -> PyLoader {
    PyLoader {
        inner: Arc::new(CsvLoader::new(paths, text_field)),
    }
}

/// JSON loader. Each file must contain a top-level array of
/// `{id, text, metadata?}` objects. The optional `pointer` argument is
/// currently unused — the upstream Rust crate only supports root-array
/// JSON shape today. Reserved for forward-compat.
#[pyfunction]
#[pyo3(signature = (paths, pointer=None))]
fn json_loader(paths: Vec<String>, pointer: Option<String>) -> PyLoader {
    let _ = pointer; // TODO: forward when the upstream Loader accepts a pointer.
    PyLoader {
        inner: Arc::new(JsonLoader::new(paths)),
    }
}

#[pyfunction]
fn loader_from_factory(key: String) -> PyResult<PyLoader> {
    let target = crate::guest::must_lookup("loader", &key)?;
    Ok(PyLoader {
        inner: Arc::new(PyLoaderAdapter { target }),
    })
}

// =============================================================================
// PyCodeLang
// =============================================================================

#[pyclass(name = "CodeLang", module = "atomr_agents._native.ingest", eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyCodeLang {
    Rust,
    Python,
    Js,
}

#[pymethods]
impl PyCodeLang {
    fn __repr__(&self) -> String {
        match self {
            PyCodeLang::Rust => "CodeLang.Rust".into(),
            PyCodeLang::Python => "CodeLang.Python".into(),
            PyCodeLang::Js => "CodeLang.Js".into(),
        }
    }
}

impl From<PyCodeLang> for CodeLang {
    fn from(p: PyCodeLang) -> Self {
        match p {
            PyCodeLang::Rust => CodeLang::Rust,
            PyCodeLang::Python => CodeLang::Python,
            PyCodeLang::Js => CodeLang::Js,
        }
    }
}

// =============================================================================
// PySplitter — dyn handle around `Arc<dyn Splitter>`
// =============================================================================

#[pyclass(name = "Splitter", module = "atomr_agents._native.ingest")]
#[derive(Clone)]
pub struct PySplitter {
    pub(crate) inner: Arc<dyn Splitter>,
}

#[pymethods]
impl PySplitter {
    /// Synchronous `split(doc) -> list[Document]`.
    fn split(&self, doc: &PyDocument) -> Vec<PyDocument> {
        self.inner
            .split(&doc.inner)
            .into_iter()
            .map(|d| PyDocument { inner: d })
            .collect()
    }

    /// `split_all(docs) -> list[Document]`.
    fn split_all(&self, docs: Vec<PyDocument>) -> Vec<PyDocument> {
        let raw: Vec<Document> = docs.into_iter().map(|d| d.inner).collect();
        self.inner
            .split_all(&raw)
            .into_iter()
            .map(|d| PyDocument { inner: d })
            .collect()
    }

    fn __repr__(&self) -> String {
        "Splitter(handle)".into()
    }
}

// ----- Python guest adapter ------------------------------------------------
//
// The Rust `Splitter::split` is synchronous; for guest targets we
// must block on a Python call inside a sync function. The adapter
// re-enters the GIL and expects the Python target to return a list
// of `Document` instances synchronously (no coroutines).

pub(crate) struct PySplitterAdapter {
    target: Arc<PyObject>,
}

impl Splitter for PySplitterAdapter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let target = self.target.clone();
        let doc = doc.clone();
        Python::with_gil(|py| -> PyResult<Vec<Document>> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("split")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let py_doc = Py::new(py, PyDocument { inner: doc })?;
            let r = instance.getattr("split")?.call1((py_doc,))?;
            let mut out = Vec::new();
            for item in r.iter()? {
                let it = item?;
                let pd: PyDocument = it.extract()?;
                out.push(pd.inner);
            }
            Ok(out)
        })
        .unwrap_or_default()
    }
}

// ----- Splitter factories --------------------------------------------------

#[pyfunction]
fn recursive_character_splitter(chunk_size: usize, overlap: usize) -> PySplitter {
    PySplitter {
        inner: Arc::new(RecursiveCharacterSplitter::new(chunk_size, overlap)),
    }
}

#[pyfunction]
fn token_splitter(chunk_size: usize, overlap: usize) -> PySplitter {
    PySplitter {
        inner: Arc::new(TokenSplitter {
            max_tokens: chunk_size,
            overlap_tokens: overlap,
        }),
    }
}

#[pyfunction]
fn markdown_header_splitter() -> PySplitter {
    PySplitter {
        inner: Arc::new(MarkdownHeaderSplitter::default()),
    }
}

#[pyfunction]
fn code_splitter(lang: PyCodeLang) -> PySplitter {
    PySplitter {
        inner: Arc::new(CodeSplitter {
            lang: lang.into(),
        }),
    }
}

// `SemanticSplitter` doesn't implement the sync `Splitter` trait —
// it's `AsyncSplitter` only. We expose a thin wrapper that runs the
// async split on the shared tokio runtime, so callers can still get
// a `PySplitter` handle. Note: it does NOT chain in `IngestPipeline`
// (which calls `Splitter::split` synchronously per element — and we
// honor that here by blocking).
struct SemanticSplitterSyncAdapter {
    inner: SemanticSplitter,
}

impl Splitter for SemanticSplitterSyncAdapter {
    fn split(&self, doc: &Document) -> Vec<Document> {
        let inner = &self.inner;
        let doc = doc.clone();
        crate::runtime::shared()
            .block_on(async move { inner.split_async(&doc).await })
            .unwrap_or_default()
    }
}

#[pyfunction]
#[pyo3(signature = (embedder, threshold=0.75, max_chunk_chars=2000))]
fn semantic_splitter(embedder: PyEmbedder, threshold: f32, max_chunk_chars: usize) -> PySplitter {
    let s = SemanticSplitter {
        embedder: embedder.inner,
        similarity_threshold: threshold,
        max_chunk_chars,
    };
    PySplitter {
        inner: Arc::new(SemanticSplitterSyncAdapter { inner: s }),
    }
}

#[pyfunction]
fn splitter_from_factory(key: String) -> PyResult<PySplitter> {
    let target = crate::guest::must_lookup("splitter", &key)?;
    Ok(PySplitter {
        inner: Arc::new(PySplitterAdapter { target }),
    })
}

// =============================================================================
// PyKvCache — dyn handle around `Arc<dyn KvCache>`
// =============================================================================

#[pyclass(name = "KvCache", module = "atomr_agents._native.ingest")]
#[derive(Clone)]
pub struct PyKvCache {
    pub(crate) inner: Arc<dyn KvCache>,
}

#[pymethods]
impl PyKvCache {
    fn get<'py>(&self, py: Python<'py>, key: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.get(&key).await.map_err(crate::errors::map)
        })
    }

    fn put<'py>(
        &self,
        py: Python<'py>,
        key: String,
        value: Vec<f32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            inner.put(key, value).await.map_err(crate::errors::map)?;
            Ok(())
        })
    }

    fn __repr__(&self) -> String {
        "KvCache(handle)".into()
    }
}

// ----- Python guest adapter ------------------------------------------------

pub(crate) struct PyKvCacheAdapter {
    target: Arc<PyObject>,
}

#[async_trait]
impl KvCache for PyKvCacheAdapter {
    async fn get(&self, key: &str) -> AgentResult<Option<Vec<f32>>> {
        let target = self.target.clone();
        let key = key.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("get")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("get")?.call1((key,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py kv_cache get: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Option<Vec<f32>>> {
            let bound = final_val.bind(py);
            if bound.is_none() {
                Ok(None)
            } else {
                Ok(Some(bound.extract::<Vec<f32>>()?))
            }
        })
        .map_err(|e| AgentError::Internal(format!("py kv_cache get result: {e}")))
    }

    async fn put(&self, key: String, value: Vec<f32>) -> AgentResult<()> {
        let target = self.target.clone();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("put")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("put")?.call1((key, value))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py kv_cache put: {e}")))?;
        let _ = await_if_coro(coro_or_val).await?;
        Ok(())
    }
}

#[pyfunction]
fn in_memory_kv_cache() -> PyKvCache {
    PyKvCache {
        inner: Arc::new(InMemoryKvCache::new()),
    }
}

#[pyfunction]
fn kv_cache_from_factory(key: String) -> PyResult<PyKvCache> {
    let target = crate::guest::must_lookup("kv_cache", &key)?;
    Ok(PyKvCache {
        inner: Arc::new(PyKvCacheAdapter { target }),
    })
}

// =============================================================================
// cached_embedder — wraps a PyEmbedder + PyKvCache → PyEmbedder
// =============================================================================

#[pyfunction]
#[pyo3(signature = (embedder, kv, model_id="default".to_string()))]
fn cached_embedder(embedder: PyEmbedder, kv: PyKvCache, model_id: String) -> PyEmbedder {
    PyEmbedder {
        inner: Arc::new(CachedEmbedder::new(embedder.inner, kv.inner, model_id)),
    }
}

// =============================================================================
// PyIngestPipeline — builder around the upstream `IngestPipeline`,
// extended with loader / cache / embedder / long-store slots so the
// resulting `.build()` is a single `Callable` that runs end-to-end.
// =============================================================================

#[pyclass(name = "IngestPipeline", module = "atomr_agents._native.ingest")]
pub struct PyIngestPipeline {
    splitters: Vec<Arc<dyn Splitter>>,
    loader: Option<Arc<dyn Loader>>,
    cache: Option<Arc<dyn KvCache>>,
    embedder: Option<Arc<dyn Embedder>>,
    long_store: Option<Arc<dyn LongStore>>,
    namespace: Option<Namespace>,
    model_id: String,
}

#[pymethods]
impl PyIngestPipeline {
    #[new]
    fn new() -> Self {
        Self {
            splitters: Vec::new(),
            loader: None,
            cache: None,
            embedder: None,
            long_store: None,
            namespace: None,
            model_id: "default".into(),
        }
    }

    /// Attach a loader. Required before `.build()`.
    fn loader(mut slf: PyRefMut<'_, Self>, loader: PyLoader) -> PyRefMut<'_, Self> {
        slf.loader = Some(loader.inner);
        slf
    }

    /// Chain a splitter stage. Order is preserved.
    fn splitter(mut slf: PyRefMut<'_, Self>, splitter: PySplitter) -> PyRefMut<'_, Self> {
        slf.splitters.push(splitter.inner);
        slf
    }

    /// Attach an embedding cache. Wraps the embedder via
    /// `CachedEmbedder` when both are present.
    fn cache(mut slf: PyRefMut<'_, Self>, kv: PyKvCache) -> PyRefMut<'_, Self> {
        slf.cache = Some(kv.inner);
        slf
    }

    fn embedder(mut slf: PyRefMut<'_, Self>, embedder: PyEmbedder) -> PyRefMut<'_, Self> {
        slf.embedder = Some(embedder.inner);
        slf
    }

    fn long_store(mut slf: PyRefMut<'_, Self>, long_store: PyLongStore) -> PyRefMut<'_, Self> {
        slf.long_store = Some(long_store.inner);
        slf
    }

    /// Target namespace under which embedded chunks are stored.
    fn namespace(mut slf: PyRefMut<'_, Self>, namespace: PyNamespace) -> PyRefMut<'_, Self> {
        slf.namespace = Some(namespace.inner);
        slf
    }

    /// `model_id` is passed through to `CachedEmbedder` when caching.
    fn model_id(mut slf: PyRefMut<'_, Self>, model_id: String) -> PyRefMut<'_, Self> {
        slf.model_id = model_id;
        slf
    }

    /// Freeze the pipeline into a `Callable`. The callable accepts any
    /// input (ignored — the loader is the source of documents) and
    /// returns `{"chunks": <n>}` on completion.
    fn build(&self) -> PyResult<PyCallable> {
        let loader = self.loader.clone().ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(
                "IngestPipeline.build(): loader is required (.loader(...))",
            )
        })?;
        let embedder_raw = self.embedder.clone().ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(
                "IngestPipeline.build(): embedder is required (.embedder(...))",
            )
        })?;
        let store = self.long_store.clone().ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(
                "IngestPipeline.build(): long_store is required (.long_store(...))",
            )
        })?;
        let namespace = self.namespace.clone().unwrap_or_else(|| Namespace(vec![]));
        let splitters = self.splitters.clone();
        let model_id = self.model_id.clone();
        let embedder: Arc<dyn Embedder> = match self.cache.clone() {
            Some(c) => Arc::new(CachedEmbedder::new(embedder_raw, c, model_id)),
            None => embedder_raw,
        };

        let handle: CallableHandle = Arc::new(FnCallable::labeled(
            "ingest_pipeline",
            move |_input: Value, _ctx: CallCtx| {
                let loader = loader.clone();
                let splitters = splitters.clone();
                let embedder = embedder.clone();
                let store = store.clone();
                let namespace = namespace.clone();
                async move {
                    let docs = loader.load().await?;
                    let mut pipeline = IngestPipeline::new();
                    for s in splitters {
                        pipeline = pipeline.then(s);
                    }
                    let chunks = pipeline.split_all(docs);
                    let n = ingest_fn(&*store, &namespace, &*embedder, chunks).await?;
                    Ok(serde_json::json!({"chunks": n}))
                }
            },
        ));
        Ok(PyCallable::from_handle(handle))
    }

    fn __repr__(&self) -> String {
        format!(
            "IngestPipeline(splitters={}, has_loader={}, has_embedder={}, has_store={})",
            self.splitters.len(),
            self.loader.is_some(),
            self.embedder.is_some(),
            self.long_store.is_some(),
        )
    }
}

// =============================================================================
// Free function `ingest(...)`
// =============================================================================

/// One-shot helper mirroring `atomr_agents_ingest::ingest`: push the
/// given `chunks` through `embedder` and write each into `store` under
/// `namespace`. Returns the number of chunks written.
#[pyfunction]
fn ingest<'py>(
    py: Python<'py>,
    store: PyLongStore,
    namespace: PyNamespace,
    embedder: PyEmbedder,
    chunks: Vec<PyDocument>,
) -> PyResult<Bound<'py, PyAny>> {
    let store_inner = store.inner;
    let embedder_inner = embedder.inner;
    let ns = namespace.inner;
    let raw: Vec<Document> = chunks.into_iter().map(|d| d.inner).collect();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let n = ingest_fn(&*store_inner, &ns, &*embedder_inner, raw)
            .await
            .map_err(crate::errors::map)?;
        Ok(n)
    })
}

// =============================================================================
// register
// =============================================================================

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "ingest")?;
    m.add_class::<PyDocument>()?;
    m.add_class::<PyLoader>()?;
    m.add_class::<PySplitter>()?;
    m.add_class::<PyKvCache>()?;
    m.add_class::<PyCodeLang>()?;
    m.add_class::<PyIngestPipeline>()?;
    // Loaders
    m.add_function(wrap_pyfunction!(text_loader, &m)?)?;
    m.add_function(wrap_pyfunction!(markdown_loader, &m)?)?;
    m.add_function(wrap_pyfunction!(csv_loader, &m)?)?;
    m.add_function(wrap_pyfunction!(json_loader, &m)?)?;
    m.add_function(wrap_pyfunction!(loader_from_factory, &m)?)?;
    // Splitters
    m.add_function(wrap_pyfunction!(recursive_character_splitter, &m)?)?;
    m.add_function(wrap_pyfunction!(token_splitter, &m)?)?;
    m.add_function(wrap_pyfunction!(markdown_header_splitter, &m)?)?;
    m.add_function(wrap_pyfunction!(code_splitter, &m)?)?;
    m.add_function(wrap_pyfunction!(semantic_splitter, &m)?)?;
    m.add_function(wrap_pyfunction!(splitter_from_factory, &m)?)?;
    // KvCache + CachedEmbedder
    m.add_function(wrap_pyfunction!(in_memory_kv_cache, &m)?)?;
    m.add_function(wrap_pyfunction!(kv_cache_from_factory, &m)?)?;
    m.add_function(wrap_pyfunction!(cached_embedder, &m)?)?;
    // Free function
    m.add_function(wrap_pyfunction!(ingest, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
