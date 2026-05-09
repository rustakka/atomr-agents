"""Facade over :mod:`atomr_agents._native.harness`.

Re-exports ``HarnessSpec``, ``IterationCapTermination``.
"""

from ._native import harness as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
