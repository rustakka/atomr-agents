"""Facade over :mod:`atomr_agents._native.retriever`.

Re-exports `Document`, `Retriever`, and the factory helpers
(`vector_retriever`, `bm25_retriever`, `multi_query_retriever`,
`contextual_compression_retriever`, `parent_document_retriever`,
`ensemble_retriever`, `self_query_retriever`,
`time_weighted_retriever`, `embeddings_filter`,
`retriever_from_factory`).
"""

from ._native import retriever as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
