"""Facade over :mod:`atomr_agents._native.callable`.

Re-exports the universal :class:`Callable` handle plus the
:class:`Pipeline` builder and decorator factories (``with_retry``,
``with_timeout``, ``with_fallbacks``, ``with_config``, ``fan_out``,
``branch``, ``lambda_``).
"""

from ._native import callable as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
