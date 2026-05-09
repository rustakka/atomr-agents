"""Facade over :mod:`atomr_agents._native.agent`.

Re-exports ``AgentSpec``, ``AgentBudgets``, ``TurnResult``.
"""

from ._native import agent as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
