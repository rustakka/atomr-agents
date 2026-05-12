"""Facade over :mod:`atomr_agents._native.embed`.

Re-exports `Embedder`, `AnnIndex`, and the factory helpers
(`mock_embedder`, `embedder_from_factory`, `in_memory_ann_index`,
`ann_index_from_factory`, `embedding_tool_strategy`).
"""

from ._native import embed as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
