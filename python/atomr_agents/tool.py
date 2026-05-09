"""Facade over :mod:`atomr_agents._native.tool`.

Re-exports ``ToolDescriptor``, ``ToolSchema``, ``ToolSet``,
``Provider``, ``ParsedToolCall``, ``ToolCallParser``.
"""

from ._native import tool as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
