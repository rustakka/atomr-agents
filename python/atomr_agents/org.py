"""Facade over :mod:`atomr_agents._native.org`.

Re-exports the organizational hierarchy builders (``Team``,
``Department``, ``Org``), the routing strategy handle
(``OrgRoutingStrategyHandle``) and its three factories
(``round_robin_router``, ``load_aware_router``,
``capability_match_router``), the ``namespaced_memory`` factory, the
``swarm_loop`` builder, and the ``ActiveAgent`` shared-state class.
"""

from ._native import org as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
