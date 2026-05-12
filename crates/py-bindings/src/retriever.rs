//! `Retriever` bindings — `Document`, `Retriever` dyn handle, and
//! per-impl factories wrapping the `atomr-agents-retriever` crate.
//!
//! Python-defined retrievers are registered through
//! `guest.retriever(...)` and materialised via
//! `retriever_from_factory(key)`. The adapter calls back into the
//! Python target's `retrieve(query, ctx)` method, awaiting any
//! returned coroutine. Results may be returned as `Document` instances
//! or as plain dicts with `id`, `text` (or `page_content`), `metadata`,
//! and optional `score`.
//!
//! The `as_callable()` method exposes any retriever as a universal
//! `PyCallable` accepting `{"query": str}` and returning a list of
//! serialized documents — useful for embedding retrievers inside
//! pipelines or harnesses.

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{CallableHandle, FnCallable};
use atomr_agents_core::{AgentError, CallCtx, Result as AgentResult, Value};
use atomr_agents_retriever::{
    Bm25Retriever, ContextualCompressionRetriever, Document, EmbeddingsFilter, EnsembleRetriever,
    MultiQueryRetriever, ParentDocumentRetriever, QueryExpander, Retriever, SelfQueryParser,
    SelfQueryRetriever, TimeWeightedRetriever, VectorRetriever,
};
use pyo3::prelude::*;
use pyo3::types::PyList;

use crate::callable::PyCallable;
use crate::conv::{callctx_from_pydict, json_to_py, py_to_json};
use crate::embed::PyEmbedder;
use crate::memory::{PyLongStore, PyNamespace};
use crate::strategy::await_if_coro;

// ----- PyDocument ----------------------------------------------------------

/// Document — the unit a retriever returns. `page_content` is the
/// textual body, `metadata` is an arbitrary JSON-shaped dict.
#[pyclass(name = "Document", module = "atomr_agents._native.retriever")]
#[derive(Clone)]
pub struct PyDocument {
    pub(crate) inner: Document,
}

#[pymethods]
impl PyDocument {
    #[new]
    #[pyo3(signature = (id, page_content, metadata=None, score=0.0))]
    fn new(
        py: Python<'_>,
        id: String,
        page_content: String,
        metadata: Option<&Bound<'_, PyAny>>,
        score: f32,
    ) -> PyResult<Self> {
        let meta = match metadata {
            Some(m) if !m.is_none() => py_to_json(py, m)?,
            _ => Value::Null,
        };
        Ok(Self {
            inner: Document {
                id,
                text: page_content,
                metadata: meta,
                score,
            },
        })
    }

    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }

    /// LangChain-style alias for `text`.
    #[getter]
    fn page_content(&self) -> &str {
        &self.inner.text
    }

    /// Native field name (atomr's `Document::text`).
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
            "Document(id={:?}, page_content={:?}, score={:.3})",
            self.inner.id, self.inner.text, self.inner.score
        )
    }
}

fn document_to_pyobject(py: Python<'_>, d: &Document) -> PyResult<PyObject> {
    Py::new(py, PyDocument { inner: d.clone() }).map(|p| p.into_py(py))
}

fn documents_to_pylist<'py>(py: Python<'py>, docs: &[Document]) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty_bound(py);
    for d in docs {
        out.append(document_to_pyobject(py, d)?)?;
    }
    Ok(out)
}

fn document_to_json(d: &Document) -> Value {
    serde_json::json!({
        "id": d.id,
        "text": d.text,
        "page_content": d.text,
        "metadata": d.metadata,
        "score": d.score,
    })
}

// ----- PyRetriever dyn handle ---------------------------------------------

/// Retriever — the dyn handle over any concrete or guest-defined
/// `Retriever`. `retrieve(query, ctx=None)` returns an awaitable of
/// `list[Document]`. `as_callable()` exposes the same surface as a
/// `Callable` accepting `{"query": str}`.
#[pyclass(name = "Retriever", module = "atomr_agents._native.retriever")]
#[derive(Clone)]
pub struct PyRetriever {
    pub(crate) inner: Arc<dyn Retriever>,
}

#[pymethods]
impl PyRetriever {
    #[pyo3(signature = (query, ctx=None))]
    fn retrieve<'py>(
        &self,
        py: Python<'py>,
        query: String,
        ctx: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let call_ctx = callctx_from_pydict(py, ctx)?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let docs = inner
                .retrieve(&query, &call_ctx)
                .await
                .map_err(crate::errors::map)?;
            Python::with_gil(|py| -> PyResult<PyObject> {
                let list = documents_to_pylist(py, &docs)?;
                Ok(list.unbind().into())
            })
        })
    }

    /// Wrap this retriever as a universal `Callable`. The callable
    /// expects an input shaped like `{"query": str}` (or a bare string)
    /// and returns a JSON-serialised list of documents.
    fn as_callable(&self) -> PyCallable {
        let inner = self.inner.clone();
        let handle: CallableHandle = Arc::new(FnCallable::labeled(
            "retriever",
            move |input: Value, ctx: CallCtx| {
                let inner = inner.clone();
                async move {
                    let query = match &input {
                        Value::String(s) => s.clone(),
                        Value::Object(o) => o
                            .get("query")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        _ => String::new(),
                    };
                    let docs = inner.retrieve(&query, &ctx).await?;
                    let out: Vec<Value> = docs.iter().map(document_to_json).collect();
                    Ok(Value::Array(out))
                }
            },
        ));
        PyCallable::from_handle(handle)
    }

    fn __repr__(&self) -> String {
        "Retriever(handle)".into()
    }
}

// ----- Python guest adapter ------------------------------------------------

pub(crate) struct PyRetrieverAdapter {
    pub(crate) target: Arc<PyObject>,
}

fn extract_document(bound: &Bound<'_, PyAny>) -> PyResult<Document> {
    // Accept either a PyDocument or a plain dict.
    if let Ok(d) = bound.extract::<PyDocument>() {
        return Ok(d.inner);
    }
    let py = bound.py();
    let id: String = bound
        .get_item("id")
        .or_else(|_| bound.getattr("id"))?
        .extract()?;
    let text: String = if let Ok(v) = bound.get_item("text") {
        v.extract()?
    } else if let Ok(v) = bound.get_item("page_content") {
        v.extract()?
    } else if let Ok(v) = bound.getattr("text") {
        v.extract()?
    } else {
        bound.getattr("page_content")?.extract()?
    };
    let metadata: Value = match bound.get_item("metadata") {
        Ok(m) if !m.is_none() => py_to_json(py, &m)?,
        _ => match bound.getattr("metadata") {
            Ok(m) if !m.is_none() => py_to_json(py, &m)?,
            _ => Value::Null,
        },
    };
    let score: f32 = match bound.get_item("score") {
        Ok(s) if !s.is_none() => s.extract().unwrap_or(0.0),
        _ => match bound.getattr("score") {
            Ok(s) if !s.is_none() => s.extract().unwrap_or(0.0),
            _ => 0.0,
        },
    };
    Ok(Document {
        id,
        text,
        metadata,
        score,
    })
}

#[async_trait]
impl Retriever for PyRetrieverAdapter {
    async fn retrieve(&self, query: &str, _ctx: &CallCtx) -> AgentResult<Vec<Document>> {
        let target = self.target.clone();
        let query = query.to_string();
        let coro_or_val = Python::with_gil(|py| -> PyResult<PyObject> {
            let bound = target.bind(py);
            let instance: Bound<'_, PyAny> = if bound.hasattr("retrieve")? {
                bound.clone()
            } else if bound.is_callable() {
                bound.call0()?
            } else {
                bound.clone()
            };
            let r = instance.getattr("retrieve")?.call1((query,))?;
            Ok(r.unbind())
        })
        .map_err(|e| AgentError::Internal(format!("py retriever: {e}")))?;
        let final_val = await_if_coro(coro_or_val).await?;
        Python::with_gil(|py| -> PyResult<Vec<Document>> {
            let bound = final_val.bind(py);
            let mut out = Vec::new();
            for item in bound.iter()? {
                let item = item?;
                out.push(extract_document(&item)?);
            }
            Ok(out)
        })
        .map_err(|e| AgentError::Internal(format!("py retriever result: {e}")))
    }
}

// ----- Factory functions ---------------------------------------------------

#[pyfunction]
fn vector_retriever(
    long_store: PyLongStore,
    embedder: PyEmbedder,
    namespace: PyNamespace,
    top_k: usize,
) -> PyRetriever {
    PyRetriever {
        inner: Arc::new(VectorRetriever::new(
            long_store.inner,
            embedder.inner,
            namespace.inner,
            top_k,
        )),
    }
}

/// Build a BM25 retriever from an in-memory corpus. `k1` / `b`
/// arguments are accepted for API parity with LangChain but are not
/// currently routed to the underlying impl (the crate ships a fixed
/// BM25 with `k1=1.5`, `b=0.75`).
#[pyfunction]
#[pyo3(signature = (documents, top_k=5, k1=1.5, b=0.75))]
fn bm25_retriever(
    documents: Vec<PyDocument>,
    top_k: usize,
    k1: f32,
    b: f32,
) -> PyRetriever {
    let _ = (k1, b);
    let r = Bm25Retriever::new(top_k);
    r.add_many(documents.into_iter().map(|d| d.inner));
    PyRetriever { inner: Arc::new(r) }
}

/// Trivial `QueryExpander` that appends caller-provided suffix
/// variants. Used as the default expander for `multi_query_retriever`
/// because the production-grade LLM-driven expander is not yet wired
/// through these bindings.
struct StaticExpander {
    variants: Vec<String>,
}

#[async_trait]
impl QueryExpander for StaticExpander {
    async fn expand(&self, query: &str, _n: usize) -> AgentResult<Vec<String>> {
        Ok(self
            .variants
            .iter()
            .map(|v| format!("{query} {v}"))
            .collect())
    }
}

/// MultiQueryRetriever with a static-suffix expander. For real LLM
/// expansion, register a guest expander once `guest.query_expander(...)`
/// lands.
#[pyfunction]
#[pyo3(signature = (base, n_queries=3, variants=None))]
fn multi_query_retriever(
    base: PyRetriever,
    n_queries: usize,
    variants: Option<Vec<String>>,
) -> PyRetriever {
    let variants = variants.unwrap_or_else(|| {
        (0..n_queries.max(1))
            .map(|i| format!("variant{i}"))
            .collect()
    });
    let expander: Arc<dyn QueryExpander> = Arc::new(StaticExpander { variants });
    PyRetriever {
        inner: Arc::new(MultiQueryRetriever::new(base.inner, expander, n_queries)),
    }
}

/// `ContextualCompressionRetriever` with the default
/// `SentenceFilterCompressor`. LLM-driven extractive compression is
/// not yet wired through these bindings; the regex-based filter
/// shipped by the crate is used unconditionally.
#[pyfunction]
fn contextual_compression_retriever(base: PyRetriever) -> PyRetriever {
    use atomr_agents_retriever::CompressionStep;

    struct SentenceFilter;
    #[async_trait]
    impl CompressionStep for SentenceFilter {
        async fn compress(
            &self,
            query: &str,
            mut doc: Document,
        ) -> AgentResult<Option<Document>> {
            let q_tokens: std::collections::HashSet<String> = query
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            let kept: Vec<&str> = doc
                .text
                .split('.')
                .filter(|s| {
                    let lower = s.to_lowercase();
                    q_tokens.iter().any(|t| lower.contains(t))
                })
                .collect();
            if kept.is_empty() {
                return Ok(None);
            }
            doc.text = kept.join(".").trim().to_string();
            Ok(Some(doc))
        }
    }

    PyRetriever {
        inner: Arc::new(ContextualCompressionRetriever::new(
            base.inner,
            Arc::new(SentenceFilter),
        )),
    }
}

/// Build a `ParentDocumentRetriever`. `mappings` is a list of
/// `(parent_document, [child_ids])` tuples.
#[pyfunction]
fn parent_document_retriever(
    child_retriever: PyRetriever,
    mappings: Vec<(PyDocument, Vec<String>)>,
) -> PyRetriever {
    let mut r = ParentDocumentRetriever::new(child_retriever.inner);
    for (parent, child_ids) in mappings {
        r.add(parent.inner, child_ids);
    }
    PyRetriever { inner: Arc::new(r) }
}

/// `EnsembleRetriever` using Reciprocal Rank Fusion (k=60). `weights`
/// is accepted for API parity but is not currently routed — RRF in
/// the underlying crate fuses without per-member weighting.
#[pyfunction]
#[pyo3(signature = (members, top_k=10, weights=None))]
fn ensemble_retriever(
    members: Vec<PyRetriever>,
    top_k: usize,
    weights: Option<Vec<f32>>,
) -> PyRetriever {
    let _ = weights;
    let members: Vec<Arc<dyn Retriever>> = members.into_iter().map(|r| r.inner).collect();
    PyRetriever {
        inner: Arc::new(EnsembleRetriever::with_rrf(members, top_k)),
    }
}

/// `SelfQueryRetriever` with the default `KeyValueParser` (extracts
/// `key:value` tokens as a metadata filter). LLM-driven parsing is
/// not yet wired through these bindings.
#[pyfunction]
fn self_query_retriever(base: PyRetriever) -> PyRetriever {
    use atomr_agents_retriever::ParsedSelfQuery;

    struct KeyValue;
    #[async_trait]
    impl SelfQueryParser for KeyValue {
        async fn parse(&self, query: &str) -> AgentResult<ParsedSelfQuery> {
            let mut filter = Vec::new();
            let mut q_parts = Vec::new();
            for tok in query.split_whitespace() {
                if let Some((k, v)) = tok.split_once(':') {
                    filter.push((k.to_string(), Value::String(v.to_string())));
                } else {
                    q_parts.push(tok);
                }
            }
            Ok(ParsedSelfQuery {
                query: q_parts.join(" "),
                filter,
            })
        }
    }

    PyRetriever {
        inner: Arc::new(SelfQueryRetriever::new(base.inner, Arc::new(KeyValue))),
    }
}

#[pyfunction]
#[pyo3(signature = (base, decay_rate=0.01))]
fn time_weighted_retriever(base: PyRetriever, decay_rate: f32) -> PyRetriever {
    PyRetriever {
        inner: Arc::new(TimeWeightedRetriever::new(base.inner, decay_rate)),
    }
}

#[pyfunction]
#[pyo3(signature = (base, embedder, threshold=0.5))]
fn embeddings_filter(base: PyRetriever, embedder: PyEmbedder, threshold: f32) -> PyRetriever {
    PyRetriever {
        inner: Arc::new(EmbeddingsFilter::new(base.inner, embedder.inner, threshold)),
    }
}

#[pyfunction]
fn retriever_from_factory(key: String) -> PyResult<PyRetriever> {
    let target = crate::guest::must_lookup("retriever", &key)?;
    Ok(PyRetriever {
        inner: Arc::new(PyRetrieverAdapter { target }),
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "retriever")?;
    m.add_class::<PyDocument>()?;
    m.add_class::<PyRetriever>()?;
    m.add_function(wrap_pyfunction!(vector_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(bm25_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(multi_query_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(contextual_compression_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(parent_document_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(ensemble_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(self_query_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(time_weighted_retriever, &m)?)?;
    m.add_function(wrap_pyfunction!(embeddings_filter, &m)?)?;
    m.add_function(wrap_pyfunction!(retriever_from_factory, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
