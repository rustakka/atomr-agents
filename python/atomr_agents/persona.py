"""Facade over :mod:`atomr_agents._native.persona`.

Re-exports ``RenderedPersona``.
"""

from ._native import persona as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
