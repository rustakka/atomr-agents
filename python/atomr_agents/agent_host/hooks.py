"""M5 — Hooks: thin Python facade over ``atomr_agents._native.host``.

The native crate (``crates/host/src/hooks.rs``) owns the hook substrate; the
PyO3 layer (``crates/py-bindings/src/host.rs``) exposes
:class:`_native.host.HookDefinition`, ``HookResult``, ``HookRegistry``,
``HookDispatcher`` plus the built-in callables ``redact_secrets`` and
``record_to_jsonl``.

This module provides the Python ergonomics on top:

* :class:`HookDefinition` — a thin wrapper aliasing the PyO3-escaped
  ``match_`` attribute back to ``match`` so test code reads naturally.
  The loader returns its own dataclass (see :mod:`.loader`) which already
  exposes ``match``; this class wraps a raw ``_native.host.HookDefinition``
  when callers want to consume the native value directly.
* :class:`HookRegistry` / :class:`HookDispatcher` — Python-side runtime that
  accepts arbitrary ``async`` (or sync, auto-wrapped) callables. The
  native registry is built-in-only and cannot bind arbitrary Python
  closures, so dispatch stays in Python while ``_native.host`` continues
  to supply the parsed definitions and the canonical built-in impls.
* :func:`redact_secrets` / :func:`record_to_jsonl` — Python factories
  returning configurable async hook impls. They are the Python analogs of
  ``_h.redact_secrets`` / ``_h.record_to_jsonl``; the native one-shots are
  re-exported here too for callers that want the raw forms.
"""

from __future__ import annotations

import asyncio
import json
import re
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Awaitable, Callable

from atomr_agents._native import host as _h

from .errors import AgentHostError  # re-exported for callers
from .loader import HookDefinition as _LoaderHookDefinition

__all__ = [
    "DEFAULT_SECRET_PATTERNS",
    "HookDefinition",
    "HookDispatcher",
    "HookImpl",
    "HookRegistry",
    "HookResult",
    "default_hook_resolver",
    "matches",
    "native_record_to_jsonl",
    "native_redact_secrets",
    "record_to_jsonl",
    "redact_secrets",
]


# ---------- native re-exports -----------------------------------------------


#: Direct handle to the native one-shot ``redact_secrets`` (operates on the
#: ``text`` field only, returns the redacted payload). Prefer the Python
#: factory :func:`redact_secrets` when you need a configurable field or
#: async semantics.
native_redact_secrets = _h.redact_secrets

#: Direct handle to the native one-shot ``record_to_jsonl`` (requires
#: ``payload["path"]``). Prefer :func:`record_to_jsonl` when you want a
#: pre-bound target path.
native_record_to_jsonl = _h.record_to_jsonl


# ---------- types ------------------------------------------------------------


HookImpl = Callable[[dict, dict], Awaitable[Any]]
"""An async hook implementation. Receives ``(payload, ctx)`` and returns
anything (often a mutated/cloned payload dict). Sync callables are
auto-wrapped via :func:`asyncio.to_thread` when registered."""


class HookDefinition:
    """Python facade over ``_native.host.HookDefinition``.

    The PyO3 binding exposes the YAML ``match:`` block as ``match_`` to
    sidestep Python's reserved word. This wrapper restores the natural
    spelling via a ``match`` property and forwards every other attribute
    to the wrapped native object.
    """

    __slots__ = ("_native",)

    def __init__(self, native: Any) -> None:
        self._native = native

    @property
    def event(self) -> str:
        return self._native.event

    @property
    def match(self) -> dict[str, Any]:
        return dict(self._native.match_ or {})

    @property
    def match_(self) -> dict[str, Any]:  # noqa: D401 — passthrough
        return dict(self._native.match_ or {})

    @property
    def call(self) -> dict[str, Any]:
        return dict(self._native.call or {})

    @property
    def when(self) -> str:
        return self._native.when

    @property
    def budget(self) -> dict[str, Any]:
        return dict(self._native.budget or {})

    @property
    def source_path(self) -> Path | None:
        sp = getattr(self._native, "source_path", None)
        return Path(sp) if sp else None


@dataclass(frozen=True)
class HookResult:
    """The outcome of a single hook invocation — mirrors
    ``_native.host.HookResult`` field-for-field.

    ``ok=False`` rolls up both timeouts and unexpected exceptions; the
    ``error`` field describes which.
    """

    hook_id: str
    event: str
    when: str
    ok: bool
    output: Any | None
    error: str | None
    duration_ms: float


# ---------- matching --------------------------------------------------------


def matches(hook: Any, payload: dict) -> bool:
    """Return True if the hook's ``match`` mapping is satisfied by ``payload``.

    Accepts either a :class:`HookDefinition` facade (with ``match``) or the
    loader's dataclass form (also exposes ``match``). Every ``(key, value)``
    pair in ``hook.match`` must be present in ``payload`` (equality). An
    empty match dict ALWAYS matches.
    """
    match = getattr(hook, "match", None)
    if not match:
        return True
    for key, value in match.items():
        if key not in payload:
            return False
        if payload[key] != value:
            return False
    return True


# ---------- registry --------------------------------------------------------


class HookRegistry:
    """Group hooks by event for dispatch.

    Mirrors the native ``_h.HookRegistry`` surface (``list_ids``,
    ``register``) but holds Python callables — the native registry only
    binds built-ins by name, so arbitrary user-supplied async closures
    live here in Python.

    Registration assigns each hook a stable ``hook_id`` of the form
    ``f"{event}#{index}"`` where ``index`` is the per-event registration
    order. ``when=pre`` and ``when=post`` share the counter so debugging
    output reads like the order things were declared.
    """

    def __init__(self) -> None:
        # event -> list[ (hook_id, HookDefinition, impl) ]
        self._by_event: dict[str, list[tuple[str, Any, HookImpl]]] = {}
        # event -> running counter for next index
        self._counters: dict[str, int] = {}

    def __len__(self) -> int:
        return sum(len(v) for v in self._by_event.values())

    def list_ids(self) -> list[str]:
        """Return all assigned hook_ids, matching the native API name."""
        out: list[str] = []
        for entries in self._by_event.values():
            for hid, _defn, _impl in entries:
                out.append(hid)
        return out

    def register(
        self,
        hook: Any,
        impl: HookImpl | Callable[[dict, dict], Any],
    ) -> str:
        """Register ``impl`` for ``hook``. Returns the assigned ``hook_id``."""
        event = hook.event
        index = self._counters.get(event, 0)
        hook_id = f"{event}#{index}"
        self._counters[event] = index + 1

        async_impl = _ensure_async(impl)
        self._by_event.setdefault(event, []).append((hook_id, hook, async_impl))
        return hook_id

    def register_definitions(
        self,
        hooks: list[Any],
        resolver: Callable[[Any], HookImpl | None],
    ) -> int:
        """Walk ``hooks`` and register each one whose resolver returns non-None.

        Returns the number of hooks that were registered.
        """
        count = 0
        for hook in hooks:
            impl = resolver(hook)
            if impl is None:
                continue
            self.register(hook, impl)
            count += 1
        return count

    def hooks_for(
        self,
        event: str,
        *,
        when: str | None = None,
    ) -> list[tuple[str, Any, HookImpl]]:
        """Return ``(hook_id, definition, impl)`` triples for ``event``.

        If ``when`` is given, only hooks whose ``definition.when`` matches
        (or is ``"both"``) are returned.
        """
        entries = list(self._by_event.get(event, ()))
        if when is None:
            return entries
        return [
            entry for entry in entries
            if entry[1].when == when or entry[1].when == "both"
        ]


# ---------- dispatcher ------------------------------------------------------


class HookDispatcher:
    """Fire registered hooks for an event with per-hook budgets in parallel.

    Mirrors the native ``_h.HookDispatcher`` surface (``dispatch(event,
    payload)``) but supports arbitrary Python callables. Each matching
    hook is wrapped in :func:`asyncio.wait_for` with a timeout derived
    from ``hook.budget.get("ms", default_timeout_ms)``. All hooks for one
    dispatch run concurrently via :func:`asyncio.gather`. Failures
    (timeouts, exceptions) become :class:`HookResult` rows with
    ``ok=False``; they do not propagate out of :py:meth:`dispatch`.
    """

    def __init__(self, registry: HookRegistry) -> None:
        self._registry = registry

    async def dispatch(
        self,
        event: str,
        payload: dict,
        *,
        when: str | None = None,
        ctx: dict | None = None,
        default_timeout_ms: int = 5000,
    ) -> list[HookResult]:
        ctx = ctx if ctx is not None else {}
        entries = self._registry.hooks_for(event, when=when)
        # Filter by match predicate.
        eligible = [
            (hook_id, defn, impl)
            for (hook_id, defn, impl) in entries
            if matches(defn, payload)
        ]
        if not eligible:
            return []

        tasks = [
            _run_one(hook_id, defn, impl, payload, ctx, default_timeout_ms)
            for (hook_id, defn, impl) in eligible
        ]
        return await asyncio.gather(*tasks)


# ---------- internal helpers ------------------------------------------------


def _ensure_async(impl: HookImpl | Callable[[dict, dict], Any]) -> HookImpl:
    """Return an async wrapper around ``impl`` unless it's already a coroutine fn."""
    if asyncio.iscoroutinefunction(impl):
        return impl  # type: ignore[return-value]

    async def _wrapped(payload: dict, ctx: dict) -> Any:
        return await asyncio.to_thread(impl, payload, ctx)

    return _wrapped


async def _run_one(
    hook_id: str,
    defn: Any,
    impl: HookImpl,
    payload: dict,
    ctx: dict,
    default_timeout_ms: int,
) -> HookResult:
    budget = getattr(defn, "budget", None) or {}
    ms = budget.get("ms", default_timeout_ms) if budget else default_timeout_ms
    try:
        timeout_s = float(ms) / 1000.0
    except (TypeError, ValueError):
        timeout_s = float(default_timeout_ms) / 1000.0

    start = time.perf_counter()
    try:
        output = await asyncio.wait_for(impl(payload, ctx), timeout=timeout_s)
        duration_ms = (time.perf_counter() - start) * 1000.0
        return HookResult(
            hook_id=hook_id,
            event=defn.event,
            when=defn.when,
            ok=True,
            output=output,
            error=None,
            duration_ms=duration_ms,
        )
    except asyncio.TimeoutError:
        duration_ms = (time.perf_counter() - start) * 1000.0
        return HookResult(
            hook_id=hook_id,
            event=defn.event,
            when=defn.when,
            ok=False,
            output=None,
            error=f"timeout after {int(ms)}ms",
            duration_ms=duration_ms,
        )
    except Exception as exc:  # noqa: BLE001 — we want everything captured
        duration_ms = (time.perf_counter() - start) * 1000.0
        return HookResult(
            hook_id=hook_id,
            event=defn.event,
            when=defn.when,
            ok=False,
            output=None,
            error=repr(exc),
            duration_ms=duration_ms,
        )


# ---------- built-in hook implementations -----------------------------------


DEFAULT_SECRET_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"(?i)(?:api[_-]?key|secret|token|password)\s*[:=]\s*[A-Za-z0-9_\-]{6,}"),
    re.compile(r"sk-[A-Za-z0-9]{20,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),  # AWS access keys
)


def redact_secrets(
    text_field: str = "text",
    *,
    patterns: tuple[re.Pattern[str], ...] = DEFAULT_SECRET_PATTERNS,
    replacement: str = "[REDACTED]",
) -> HookImpl:
    """Factory: returns an async hook that scrubs ``payload[text_field]``.

    Configurable analog of ``_native.host.redact_secrets`` (which is
    hard-coded to the ``text`` field with no async semantics). Returns a
    shallow copy of the payload with the named field replaced; if the
    field is missing the payload passes through unchanged. The caller's
    dict is never mutated.
    """

    async def _impl(payload: dict, ctx: dict) -> dict:
        out = dict(payload)
        text = out.get(text_field)
        if not isinstance(text, str):
            return out
        scrubbed = text
        for pattern in patterns:
            scrubbed = pattern.sub(replacement, scrubbed)
        out[text_field] = scrubbed
        return out

    return _impl


def record_to_jsonl(target_path: Path) -> HookImpl:
    """Factory: returns an async hook that appends one JSON line per call.

    Configurable analog of ``_native.host.record_to_jsonl`` (which expects
    ``payload["path"]`` per-call). Each line is
    ``{"event": ..., "payload": ..., "ts_ms": ...}``. Parent directories
    are created on demand. Writes are simple open()+write()+close() — no
    fsync, no rotation; that's for later.
    """
    target_path = Path(target_path)

    async def _impl(payload: dict, ctx: dict) -> dict:
        target_path.parent.mkdir(parents=True, exist_ok=True)
        record = {
            "event": ctx.get("event") or payload.get("event") or "",
            "payload": payload,
            "ts_ms": int(time.time() * 1000),
        }
        line = json.dumps(record, default=str, ensure_ascii=False)
        with target_path.open("a", encoding="utf-8") as fh:
            fh.write(line + "\n")
        return record

    return _impl


# ---------- default resolver ------------------------------------------------


def default_hook_resolver(
    *,
    secrets_field: str = "text",
    jsonl_path: Path | None = None,
) -> Callable[[Any], HookImpl | None]:
    """Return a resolver that binds the two built-in hook impls from YAML.

    Recognized ``call`` shapes (matched on ``id``; ``kind`` may be
    ``builtin`` or ``skill`` to accommodate the fixture YAML)::

        call: {kind: builtin, id: redact_secrets}    -> redact_secrets(...)
        call: {kind: builtin, id: record_to_jsonl}   -> record_to_jsonl(jsonl_path)

    Returns ``None`` for any other call shape so callers can layer their
    own resolver(s) on top.
    """

    def _resolve(hook: Any) -> HookImpl | None:
        call = getattr(hook, "call", None) or {}
        kind = call.get("kind")
        ident = call.get("id")
        if kind not in (None, "builtin", "skill"):
            return None
        if ident == "redact_secrets":
            return redact_secrets(secrets_field)
        if ident == "record_to_jsonl":
            if jsonl_path is None:
                return None
            return record_to_jsonl(jsonl_path)
        return None

    return _resolve
