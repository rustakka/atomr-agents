"""Guest-mode helpers.

Decorators ``@tool``, ``@strategy``, ``@persona`` will register Python
callables as factories that produce ``Box<dyn ToolStrategy>`` /
``Box<dyn PersonaStrategy>`` etc., backed by ``PyActor`` instances on
the ``python-subinterpreter-pool`` dispatcher (see
``../atomr/crates/py-bindings/pycore``). The Rust-side mechanism is in
place (`atomr-agents-strategy` traits + `atomr-agents-tool`); the
factory plumbing is wired alongside the atomr ``_native`` extension and
ships in a follow-up.
"""

from typing import Any, Callable

__all__ = ["tool", "strategy", "persona"]


def tool(toolset: str | None = None) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Marker decorator that records the function as a tool factory.

    The actual cross-FFI wiring is registered when the native module
    starts up; until then, this is a no-op that returns the function
    unchanged so user code can be written ahead of the runtime.
    """

    def _wrap(fn: Callable[..., Any]) -> Callable[..., Any]:
        setattr(fn, "__atomr_agents_tool__", {"toolset": toolset})
        return fn

    return _wrap


def strategy(kind: str) -> Callable[[type], type]:
    """Marker decorator for a strategy class (tool/memory/skill/...)."""

    def _wrap(cls: type) -> type:
        setattr(cls, "__atomr_agents_strategy__", {"kind": kind})
        return cls

    return _wrap


def persona(name: str) -> Callable[[type], type]:
    """Marker decorator for a persona strategy class."""

    def _wrap(cls: type) -> type:
        setattr(cls, "__atomr_agents_persona__", {"name": name})
        return cls

    return _wrap
