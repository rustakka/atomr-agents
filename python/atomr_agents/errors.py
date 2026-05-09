"""Facade over :mod:`atomr_agents._native.errors`.

Python exception hierarchy mirroring the Rust ``AgentError`` family.
"""

from ._native import errors as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
