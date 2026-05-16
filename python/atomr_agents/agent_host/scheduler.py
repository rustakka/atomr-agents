"""M6 — Scheduler + crons — thin facade over ``_native.host``.

The expression parser, on-disk loader, and scaffolding helpers delegate to
``atomr_agents._native.host`` (aliased as ``_h``); we add small Python-side
guards for inputs the native code doesn't currently validate (non-string
expressions, empty intervals, on-disk dotfile filtering, on-disk
expression validation, scaffold idempotence + ``force``, scaffold-time
expression validation) and convert native results into the dataclasses
the Python API has always exposed.

The :class:`Scheduler` itself stays pure-Python: the native
``_h.Scheduler`` is a much simpler primitive (no injectable clock, no
async impl registration, no per-entry next-fire bookkeeping) and the
Python test surface relies on the richer API documented below:

* :func:`parse_expression` understands the minimal ``every:Ns`` /
  ``every:Nm`` / ``every:Nh`` / ``every:Nd`` grammar. Full crontab strings
  are deliberately out of scope for M6.
* :class:`CronEntry` is the in-memory representation of a single
  ``crons/<id>.yaml`` file.
* :func:`load_crons` walks the host's ``crons/`` directory and parses each
  yaml file into a :class:`CronEntry`.
* :class:`Scheduler` holds an injectable monotonic clock so tests can
  advance time deterministically. :py:meth:`Scheduler.register` seeds a
  per-entry next-fire timestamp using :func:`parse_expression` once, and
  :py:meth:`Scheduler.fire_due` runs every entry that's due in parallel via
  :func:`asyncio.gather`, captures successes and exceptions into
  :class:`CronFireResult`, then reschedules each based on its own
  expression.
* :py:meth:`Scheduler.tick_until` is a small driver useful for tests and
  short-lived runs; it sleeps in small slices and calls
  :py:meth:`Scheduler.fire_due` repeatedly until a deadline.
* :func:`default_cron_resolver` resolves ``call: {kind: builtin, id: ...}``
  to a small registry of built-in factories (only ``noop`` ships in M6).
* :func:`scaffold_cron` writes a starter yaml under ``<root>/crons/``.
"""

from __future__ import annotations

import asyncio
import inspect
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable, Mapping

from atomr_agents._native import host as _h

from .config import HostConfig
from .errors import AgentHostError, AgentSpecError

try:
    import yaml  # type: ignore[import-untyped]

    _yaml_available = True
except ImportError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]
    _yaml_available = False

__all__ = [
    "CronEntry",
    "CronFireResult",
    "CronImpl",
    "DEFAULT_CRON_BUILTINS",
    "Scheduler",
    "default_cron_resolver",
    "load_crons",
    "parse_expression",
    "scaffold_cron",
]


# ---------- types -----------------------------------------------------------


CronImpl = Callable[[dict, dict], Awaitable[Any]]
"""An async cron implementation. Receives ``(input, ctx)`` and returns
anything JSON-serializable (captured into :class:`CronFireResult.output`).
Sync callables are auto-wrapped via :func:`asyncio.to_thread` when
registered."""


@dataclass(frozen=True)
class CronEntry:
    """One cron entry, deserialized from ``<root>/crons/<id>.yaml``."""

    id: str
    """Filename stem; unique per host."""

    expression: str
    """``every:Ns`` / ``every:Nm`` / ``every:Nh`` / ``every:Nd``."""

    call: dict
    """The ``call`` mapping from yaml — typically ``{kind, id, ...}``."""

    input: dict = field(default_factory=dict)
    """JSON payload passed to the impl on each fire."""

    enabled: bool = True
    """Disabled entries are skipped in :py:meth:`Scheduler.fire_due`."""

    source_path: Path | None = None
    """The yaml path the entry was loaded from (None for in-memory entries)."""


@dataclass(frozen=True)
class CronFireResult:
    """The outcome of a single cron invocation.

    ``ok=False`` rolls up any unexpected exception; the ``error`` field is
    the ``repr`` of the underlying exception. Failures NEVER propagate out
    of :py:meth:`Scheduler.fire_due` — they're captured here.
    """

    entry_id: str
    ok: bool
    started_at_ms: float
    duration_ms: float
    output: Any | None
    error: str | None


# ---------- expression parsing ----------------------------------------------


def parse_expression(expression: str) -> float:
    """Parse ``every:<N><unit>`` into a seconds float.

    Delegates to ``_native.host.parse_expression`` after a Python-side
    guard so non-string input and empty bodies surface as
    :class:`AgentSpecError` rather than panicking the native side.
    """
    if not isinstance(expression, str):
        raise AgentSpecError(
            f"cron expression must be a string, got {type(expression).__name__}"
        )
    raw = expression.strip()
    if raw.startswith("every:") and not raw[len("every:") :].strip():
        raise AgentSpecError(
            f"cron expression {expression!r} has empty interval after 'every:'"
        )
    try:
        seconds = _h.parse_expression(raw)
    except ValueError as exc:
        raise AgentSpecError(str(exc)) from exc
    return float(seconds)


# ---------- loader ----------------------------------------------------------


def load_crons(config: HostConfig) -> list[CronEntry]:
    """Walk ``<root>/crons/*.yaml`` and return one :class:`CronEntry` per file.

    The on-disk yaml schema used by the Python host doesn't carry the
    explicit ``id:`` field the native loader currently requires, so the
    file walk + yaml parse is done in Python; only the expression grammar
    is shared with the native side (via :func:`parse_expression`, which
    delegates to ``_h.parse_expression``).

    The ``id`` of each entry is the filename stem. Files starting with
    ``.`` are skipped. Each entry's ``expression`` is validated up front;
    an invalid expression surfaces as :class:`AgentSpecError` referencing
    the offending file.

    Returns an empty list if the ``crons/`` directory does not exist.
    """
    crons_dir = config.paths.crons_dir
    if not crons_dir.is_dir():
        return []
    if not _yaml_available:
        raise AgentHostError(
            "PyYAML is required to load crons — install atomr-agents[host]"
        )

    entries: list[CronEntry] = []
    paths = sorted(p for p in crons_dir.iterdir() if p.is_file())
    for path in paths:
        if path.name.startswith("."):
            continue
        if path.suffix.lower() not in {".yaml", ".yml"}:
            continue
        try:
            raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as exc:  # type: ignore[union-attr]
            raise AgentSpecError(f"invalid YAML in {path}: {exc}") from exc
        if not isinstance(raw, dict):
            raise AgentSpecError(
                f"{path}: top-level of a cron file must be a YAML mapping"
            )
        expression = raw.get("expression") or raw.get("when")
        if not isinstance(expression, str) or not expression:
            raise AgentSpecError(
                f"{path}: cron file is missing required string field "
                "`expression` (or alias `when`)"
            )
        # Validate immediately so a typo doesn't lurk until first fire.
        try:
            parse_expression(expression)
        except AgentSpecError as exc:
            raise AgentSpecError(f"{path}: {exc}") from exc

        call_raw = raw.get("call") or {}
        if not isinstance(call_raw, dict):
            raise AgentSpecError(f"{path}: `call` must be a mapping")

        input_raw = raw.get("input") or {}
        if not isinstance(input_raw, dict):
            raise AgentSpecError(f"{path}: `input` must be a mapping")

        enabled_raw = raw.get("enabled", True)
        enabled = bool(enabled_raw)

        entries.append(
            CronEntry(
                id=path.stem,
                expression=expression,
                call=dict(call_raw),
                input=dict(input_raw),
                enabled=enabled,
                source_path=path,
            )
        )
    return entries


# ---------- scheduler -------------------------------------------------------


class Scheduler:
    """Periodic cron driver with an injectable monotonic clock.

    The clock is invoked everywhere internally (``register``, ``fire_due``,
    ``tick_until``, ``schedule_after``) so tests can stub it and advance
    time deterministically without sleeping.

    A separate map holds the per-entry interval (computed once at
    :py:meth:`register` time) so we never re-parse the expression on
    every tick.
    """

    def __init__(self, *, clock: Callable[[], float] | None = None) -> None:
        self._clock: Callable[[], float] = clock if clock is not None else time.monotonic
        # entry_id -> CronEntry
        self._entries: dict[str, CronEntry] = {}
        # entry_id -> async impl
        self._impls: dict[str, CronImpl] = {}
        # entry_id -> seconds interval (cached parse_expression result)
        self._intervals: dict[str, float] = {}
        # entry_id -> next-fire timestamp (clock units)
        self._next_fire: dict[str, float] = {}

    # ----- registration -----

    def register(
        self,
        entry: CronEntry,
        impl: CronImpl | Callable[[dict, dict], Any],
    ) -> None:
        """Register ``impl`` for ``entry`` and seed its next-fire timestamp.

        The interval is computed once via :func:`parse_expression` at
        registration time; subsequent ticks reuse the cached value. The
        first fire is exactly one interval after ``register`` is called
        (i.e. ``clock() + interval``).
        """
        interval = parse_expression(entry.expression)
        self._entries[entry.id] = entry
        self._impls[entry.id] = _ensure_async(impl)
        self._intervals[entry.id] = interval
        self._next_fire[entry.id] = self._clock() + interval

    def schedule_after(self, entry_id: str, delay_s: float) -> None:
        """Set the entry's next-fire to ``clock() + delay_s``.

        Used internally to reschedule after a fire; exposed publicly so
        callers can force an entry to fire sooner (``delay_s=0``) for
        testing or manual triggers.
        """
        if entry_id not in self._entries:
            raise KeyError(f"unknown cron entry {entry_id!r}")
        self._next_fire[entry_id] = self._clock() + float(delay_s)

    def next_fire(self, entry_id: str) -> float | None:
        """Return the next-fire timestamp for ``entry_id``, or None."""
        return self._next_fire.get(entry_id)

    def entries(self) -> list[CronEntry]:
        """Return the list of registered entries in registration order."""
        return list(self._entries.values())

    # ----- firing -----

    async def fire_due(self, *, ctx: dict | None = None) -> list[CronFireResult]:
        """Run every enabled entry whose next-fire ``≤ clock()`` in parallel.

        Each impl is invoked with ``(entry.input, ctx)``. Successes and
        failures are captured into :class:`CronFireResult` rows; failures
        do NOT propagate out of this method. After firing, each entry is
        rescheduled to ``clock() + interval``.
        """
        ctx = ctx if ctx is not None else {}
        now = self._clock()
        due: list[CronEntry] = []
        for entry_id, entry in self._entries.items():
            if not entry.enabled:
                continue
            next_fire = self._next_fire.get(entry_id)
            if next_fire is None:
                continue
            if next_fire <= now:
                due.append(entry)
        if not due:
            return []

        tasks = [
            _run_one(entry, self._impls[entry.id], ctx)
            for entry in due
        ]
        results = await asyncio.gather(*tasks)

        # Reschedule using the (already-cached) per-entry interval. We
        # call clock() again *after* the gather to mirror real-world
        # behavior — a slow impl shouldn't immediately re-fire.
        post = self._clock()
        for entry in due:
            interval = self._intervals[entry.id]
            self._next_fire[entry.id] = post + interval
        return results

    async def tick_until(
        self,
        until_ts: float,
        *,
        ctx: dict | None = None,
        tick_seconds: float = 0.05,
    ) -> list[CronFireResult]:
        """Loop until ``clock() >= until_ts``, firing due entries each tick.

        Sleeps ``tick_seconds`` between ticks (via :func:`asyncio.sleep`).
        Aggregates every :class:`CronFireResult` produced into a single
        list returned at the end. Intended for short test runs and
        single-shot triggers — long-running hosts should drive
        :py:meth:`fire_due` from their own event loop.
        """
        aggregated: list[CronFireResult] = []
        while self._clock() < until_ts:
            results = await self.fire_due(ctx=ctx)
            if results:
                aggregated.extend(results)
            await asyncio.sleep(tick_seconds)
        # One final pass — any entry that just became due during the last
        # sleep should still fire before we return.
        results = await self.fire_due(ctx=ctx)
        if results:
            aggregated.extend(results)
        return aggregated


# ---------- internal helpers ------------------------------------------------


def _ensure_async(impl: CronImpl | Callable[[dict, dict], Any]) -> CronImpl:
    """Return an async wrapper around ``impl`` unless it's already async."""
    if inspect.iscoroutinefunction(impl):
        return impl  # type: ignore[return-value]

    async def _wrapped(input: dict, ctx: dict) -> Any:
        return await asyncio.to_thread(impl, input, ctx)

    return _wrapped


async def _run_one(
    entry: CronEntry,
    impl: CronImpl,
    ctx: dict,
) -> CronFireResult:
    started_at_ms = time.time() * 1000.0
    start = time.perf_counter()
    try:
        output = await impl(entry.input, ctx)
        duration_ms = (time.perf_counter() - start) * 1000.0
        return CronFireResult(
            entry_id=entry.id,
            ok=True,
            started_at_ms=started_at_ms,
            duration_ms=duration_ms,
            output=output,
            error=None,
        )
    except Exception as exc:  # noqa: BLE001 — we want everything captured
        duration_ms = (time.perf_counter() - start) * 1000.0
        return CronFireResult(
            entry_id=entry.id,
            ok=False,
            started_at_ms=started_at_ms,
            duration_ms=duration_ms,
            output=None,
            error=repr(exc),
        )


# ---------- built-in cron implementations -----------------------------------


def _noop_factory() -> CronImpl:
    """Return an async impl that simply echoes its input.

    Useful as the default ``call`` for scaffolded crons and as a
    placeholder while wiring up a real impl.
    """

    async def _impl(input: dict, ctx: dict) -> dict:
        return dict(input)

    return _impl


DEFAULT_CRON_BUILTINS: dict[str, Callable[..., CronImpl]] = {
    "noop": _noop_factory,
}
"""Registry of built-in cron factories that :func:`default_cron_resolver`
knows how to bind. Each value is a zero-arg factory returning a
:type:`CronImpl`."""


# ---------- default resolver ------------------------------------------------


def default_cron_resolver(
    *,
    builtins: dict[str, Callable[..., CronImpl]] = DEFAULT_CRON_BUILTINS,
) -> Callable[[CronEntry], CronImpl | None]:
    """Return a resolver that binds ``call: {kind: builtin, id: <name>}``.

    The resolver looks up ``call.id`` in ``builtins`` and invokes the
    factory with no arguments. Anything that doesn't match returns
    ``None`` so callers can layer their own resolver(s) on top.
    """

    def _resolve(entry: CronEntry) -> CronImpl | None:
        call = entry.call or {}
        kind = call.get("kind")
        ident = call.get("id")
        if kind != "builtin":
            return None
        if not isinstance(ident, str):
            return None
        factory = builtins.get(ident)
        if factory is None:
            return None
        return factory()

    return _resolve


# ---------- scaffolding -----------------------------------------------------


def scaffold_cron(
    config: HostConfig,
    cron_id: str,
    *,
    when: str = "every:1h",
    call: Mapping[str, Any] | None = None,
    input: Mapping[str, Any] | None = None,
    force: bool = False,
) -> Path:
    """Write ``<root>/crons/<cron_id>.yaml`` with a starter cron entry.

    Idempotent: if the file already exists it's left alone unless
    ``force=True``. The ``when`` value is validated via
    :func:`parse_expression` (which delegates to
    ``_h.parse_expression``) so we never write a file the loader would
    later reject. The yaml shape itself stays Python-owned — the native
    scaffolder writes a different on-disk schema that the Python loader
    can't read back.

    Returns the path to the yaml file (whether or not it was written).
    """
    if not _yaml_available:
        raise AgentHostError(
            "PyYAML is required to scaffold a cron entry — install atomr-agents[host]"
        )
    # Validate eagerly so the on-disk file never contains a bogus expression.
    parse_expression(when)

    crons_dir = config.paths.crons_dir
    crons_dir.mkdir(parents=True, exist_ok=True)
    path = crons_dir / f"{cron_id}.yaml"
    if path.exists() and not force:
        return path

    body: dict[str, Any] = {
        "expression": when,
        "call": dict(call) if call is not None else {"kind": "builtin", "id": "noop"},
        "input": dict(input) if input is not None else {},
        "enabled": True,
    }
    path.write_text(
        yaml.safe_dump(body, sort_keys=False),
        encoding="utf-8",
    )
    return path
