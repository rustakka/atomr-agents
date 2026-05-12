"""Facade over :mod:`atomr_agents._native.instruction`.

Re-exports `RenderedInstructions`, `ChatPromptTemplate`,
`ChatPromptTemplateBuilder`, `FewShotChatTemplate`, `MessageTemplate`,
`MessagesPlaceholder`, `StringTemplate`, `RenderedMessage`, `Example`,
selectors, and the `*_strategy_from_factory` builders.
"""

from ._native import instruction as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
