"""Facade over :mod:`atomr_agents._native.registry`.

Re-exports ``Registry``, ``ArtifactKind``, ``ArtifactRecord``,
``EvalSummary``.
"""

from ._native import registry as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
