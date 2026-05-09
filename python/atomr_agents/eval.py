"""Facade over :mod:`atomr_agents._native.eval`.

Re-exports ``PairwiseChoice``, ``Verdict``.
"""

from ._native import eval as _sub  # noqa: A004 - shadows builtin intentionally

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
