"""Facade over :mod:`atomr_agents._native.strategy`.

Re-exports `Termination`, `RoutingTarget`, `SkillRef`, `ToolRef`,
`Policy`, `PolicyDecision`, the dyn-handle strategy classes, and the
`*_strategy_from_factory(key)` builders that materialise a strategy
adapter from a Python target registered through
``atomr_agents.guest.strategy(...)``.
"""

from ._native import strategy as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
