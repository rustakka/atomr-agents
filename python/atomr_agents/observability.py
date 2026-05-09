"""Facade over :mod:`atomr_agents._native.observability`.

Re-exports ``Event``, ``EventBus``, ``EventStream``, and
``RunTreeBuilder``.
"""

from ._native import observability as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
