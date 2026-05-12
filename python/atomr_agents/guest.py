"""Guest-mode helpers — Python classes implementing Rust traits.

The decorators below register a Python class or callable with the
process-wide guest-factory registry exposed by
:mod:`atomr_agents._native.guest`. Each registration returns a
:class:`~atomr_agents._native.guest.GuestHandle` that pairs the
factory's category and key, and the factory itself is held by the
native registry until ``clear_factories()`` is called.

The Rust adapters that consume these factories — ``PyToolAdapter``,
``PyContextStrategyAdapter``, ``PyPersonaAdapter``, etc. — round-trip
JSON-shaped arguments through ``json.dumps`` / ``json.loads`` so user
code can stay in plain Python without manual ``Bound<PyAny>``
wrangling.

Example::

    from atomr_agents.guest import tool

    @tool(toolset="calc")
    class Calc:
        async def invoke(self, args, ctx):
            return {"sum": args["a"] + args["b"]}

When the host-side Rust agent dispatches the ``"calc"`` toolset, it
finds this class via the registry, instantiates it, and calls
``invoke(args, ctx)`` under the GIL. ``async def`` methods are
awaited; sync ``def`` methods are called directly.
"""

from typing import Any, Callable

try:
    from . import _native as _native_pkg
    _guest = _native_pkg.guest
except (ImportError, AttributeError):  # pragma: no cover - extension not built
    _guest = None


__all__ = [
    "tool",
    "strategy",
    "persona",
    "skill",
    "parser",
    "scorer",
    "memory_store",
    "embedder",
    "callable_",
    "retriever",
    "loader",
    "splitter",
    "kv_cache",
    "long_store",
    "tracer",
    "conversation_agent",
    "diarizer",
    "vad",
    "phonemizer",
    "journal",
    "repair_model",
    "persona_reconciler",
    "inference_client",
    "ann_index",
    "list_factories",
    "clear_factories",
]


def _register(kind: str, key: str, target: Any) -> Any:
    """Register *target* under *kind*/*key*. If the native extension
    isn't built yet, store the marker as an attribute so user code can
    still import; the host loader will replay these markers once
    `_native` is available.
    """
    if _guest is None:
        marker = {"kind": kind, "key": key}
        try:
            setattr(target, f"__atomr_agents_{kind}__", marker)
        except Exception:  # noqa: BLE001 - some objects forbid setattr
            pass
        return target

    handle_fn = {
        "tool": _guest.register_tool_factory,
        "persona": _guest.register_persona_factory,
        "skill": _guest.register_skill_factory,
        "parser": _guest.register_parser_factory,
        "scorer": _guest.register_scorer_factory,
        "memory": _guest.register_memory_factory,
        "embedder": _guest.register_embedder_factory,
        "callable": _guest.register_callable_factory,
        "retriever": _guest.register_retriever_factory,
        "loader": _guest.register_loader_factory,
        "splitter": _guest.register_splitter_factory,
        "kv_cache": _guest.register_kv_cache_factory,
        "long_store": _guest.register_long_store_factory,
        "tracer": _guest.register_tracer_factory,
        "conversation_agent": _guest.register_conversation_agent_factory,
        "diarizer": _guest.register_diarizer_factory,
        "vad": _guest.register_vad_factory,
        "phonemizer": _guest.register_phonemizer_factory,
        "journal": _guest.register_journal_factory,
        "repair_model": _guest.register_repair_model_factory,
        "persona_reconciler": _guest.register_persona_reconciler_factory,
        "inference_client": _guest.register_inference_client_factory,
        "ann_index": _guest.register_ann_index_factory,
    }
    if kind in handle_fn:
        handle = handle_fn[kind](key, target)
    else:
        handle = _guest.register_strategy_factory(kind, key, target)

    try:
        setattr(target, "__atomr_agents_handle__", handle)
    except Exception:  # noqa: BLE001
        pass
    return target


def _decorator(kind: str):
    """Build a `@kind(name=…)` decorator for the given guest kind."""

    def factory(name: str | None = None):
        def _wrap(target: Any) -> Any:
            key = name or getattr(target, "__name__", "anonymous")
            return _register(kind, key, target)

        return _wrap

    return factory


callable_ = _decorator("callable")
retriever = _decorator("retriever")
loader = _decorator("loader")
splitter = _decorator("splitter")
kv_cache = _decorator("kv_cache")
long_store = _decorator("long_store")
tracer = _decorator("tracer")
conversation_agent = _decorator("conversation_agent")
diarizer = _decorator("diarizer")
vad = _decorator("vad")
phonemizer = _decorator("phonemizer")
journal = _decorator("journal")
repair_model = _decorator("repair_model")
persona_reconciler = _decorator("persona_reconciler")
inference_client = _decorator("inference_client")
ann_index = _decorator("ann_index")


def tool(toolset: str | None = None) -> Callable[[Any], Any]:
    """Decorate a class / function as a tool factory.

    The toolset key defaults to the function or class name. The Rust
    adapter looks the registered target up by toolset key when an
    Agent's ``ToolSet`` resolves to it, instantiates it (if a class)
    and calls ``invoke(args, ctx)``.
    """

    def _wrap(target: Any) -> Any:
        key = toolset or getattr(target, "__name__", "anonymous")
        return _register("tool", key, target)

    return _wrap


def strategy(kind: str) -> Callable[[type], type]:
    """Decorate a class as a strategy factory.

    ``kind`` is one of: ``"tool"``, ``"memory"``, ``"skill"``,
    ``"persona"``, ``"instruction"``, ``"routing"``, ``"loop"``,
    ``"termination"``. Each kind has a corresponding Rust adapter that
    invokes ``resolve()`` / ``select()`` / ``applicable()`` /
    ``retrieve()`` / ``render()`` (whichever the trait requires) on
    the registered class.
    """

    def _wrap(cls: type) -> type:
        key = getattr(cls, "__name__", "anonymous")
        return _register(kind, key, cls)

    return _wrap


def persona(name: str) -> Callable[[type], type]:
    """Decorate a class as a persona-strategy factory."""

    def _wrap(cls: type) -> type:
        return _register("persona", name, cls)

    return _wrap


def skill(name: str) -> Callable[[type], type]:
    """Decorate a class as a skill-strategy factory."""

    def _wrap(cls: type) -> type:
        return _register("skill", name, cls)

    return _wrap


def parser(name: str) -> Callable[[type], type]:
    """Decorate a class as an output-parser factory.

    The class must implement ``parse(raw: str) -> Any`` and may
    optionally implement ``format_instructions() -> str``.
    """

    def _wrap(cls: type) -> type:
        return _register("parser", name, cls)

    return _wrap


def scorer(name: str) -> Callable[[type], type]:
    """Decorate a class as an eval-scorer factory.

    The class must implement ``score(case, output) -> ScorerOutcome``.
    """

    def _wrap(cls: type) -> type:
        return _register("scorer", name, cls)

    return _wrap


def memory_store(name: str) -> Callable[[type], type]:
    """Decorate a class as a memory-store factory."""

    def _wrap(cls: type) -> type:
        return _register("memory", name, cls)

    return _wrap


def embedder(name: str) -> Callable[[type], type]:
    """Decorate a class as an embedder factory.

    The class must implement ``async def embed(text: str) -> list[float]``.
    """

    def _wrap(cls: type) -> type:
        return _register("embedder", name, cls)

    return _wrap


def list_factories(kind: str) -> list[str]:
    """List registered factory keys for a given kind. Returns an
    empty list when the native extension isn't built.
    """
    if _guest is None:
        return []
    return list(_guest.list_factories(kind))


def clear_factories() -> int:
    """Empty the process-wide guest registry. Returns the number of
    entries cleared.
    """
    if _guest is None:
        return 0
    return _guest.clear_factories()
