"""atomr-agents: composable agentic framework on top of atomr.

This package re-exports the native module ``atomr_agents._native``
plus user-facing Python helpers. Build the native extension with::

    maturin develop --features python -m crates/py-bindings/Cargo.toml

Host mode currently exposes ``EventBus`` and ``Registry``. Guest-mode
``@tool``/``@strategy``/``@persona`` decorators land on top of the
atomr ``pycore`` subinterpreter-pool dispatcher; see the architecture
doc for the wiring plan.
"""

from importlib import import_module as _import_module

try:
    _native = _import_module("atomr_agents._native")
    Event = _native.Event
    EventBus = _native.EventBus
    Registry = _native.Registry
except ImportError:  # pragma: no cover - native extension not built yet
    _native = None
    Event = None
    EventBus = None
    Registry = None

__all__ = ["Event", "EventBus", "Registry"]
