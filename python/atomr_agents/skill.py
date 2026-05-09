"""Facade over :mod:`atomr_agents._native.skill`.

Re-exports ``Skill``, ``SkillSet``.
"""

from ._native import skill as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
