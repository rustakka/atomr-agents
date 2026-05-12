"""Facade over :mod:`atomr_agents._native.memory`.

Re-exports `MemoryStore`, `LongStore`, `Namespace`, `StoreItem`, and
the factory helpers (`in_memory_store`, `in_memory_long_store`,
`recency_memory_strategy`, `summarizing_memory_strategy`,
`chained_memory_strategy`).
"""

from ._native import memory as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
