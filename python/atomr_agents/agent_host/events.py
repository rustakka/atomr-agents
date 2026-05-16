"""Append-only JSONL event log + async tail (M9).

The host writes one JSONL line per ``EventRecord`` to ``<root>/events.jsonl``
so external consumers — ``atomr-host events tail``, the curator, dashboards
— can stream observability data without coupling to any in-process bus.

Design choices:

- One ``open()``/``write()``/``close()`` per append so concurrent writers
  across processes see strictly ordered lines (no buffered interleaving).
- ``ts_ms`` defaults to ``time.time() * 1000`` so records are
  monotonic-ish without forcing callers to pass a clock.
- :meth:`EventLog.tail` is an async generator that yields existing
  records first, then (when ``follow=True``) polls the file for new
  lines. Cancellation is via the standard asyncio task-cancel path.
"""

from __future__ import annotations

import asyncio
import json
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Mapping

from .errors import AgentHostError


__all__ = ["EventRecord", "EventLog"]


def _now_ms() -> float:
    return time.time() * 1000.0


@dataclass(frozen=True)
class EventRecord:
    """One line in ``events.jsonl``.

    The host emits these for tool calls, skill promotions, cron fires,
    gateway routing, and any other observable lifecycle moment.
    """

    kind: str
    """Short tag identifying the event family (e.g. ``"tool_call_ended"``)."""

    ts_ms: float = field(default_factory=_now_ms)
    """Unix epoch in milliseconds. Defaults to ``time.time() * 1000``."""

    agent_id: str | None = None
    """Owning agent's id, when relevant. ``None`` for host-level events."""

    payload: dict = field(default_factory=dict)
    """Event-specific data. Must be JSON-serializable."""

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "ts_ms": self.ts_ms,
            "agent_id": self.agent_id,
            "payload": dict(self.payload),
        }

    @classmethod
    def from_mapping(cls, raw: Mapping[str, Any]) -> EventRecord:
        kind = raw.get("kind")
        if not isinstance(kind, str) or not kind:
            raise AgentHostError("event record is missing required `kind`")
        ts_ms = raw.get("ts_ms")
        if isinstance(ts_ms, (int, float)):
            ts_val = float(ts_ms)
        else:
            ts_val = _now_ms()
        agent_id = raw.get("agent_id")
        if agent_id is not None and not isinstance(agent_id, str):
            agent_id = str(agent_id)
        payload = raw.get("payload") or {}
        if not isinstance(payload, dict):
            payload = {"value": payload}
        return cls(kind=kind, ts_ms=ts_val, agent_id=agent_id, payload=dict(payload))


class EventLog:
    """Append-only JSONL log used as the host-wide observability sink.

    Atomicity guarantee: each :meth:`append` opens the file in append mode,
    writes exactly one ``json.dumps(record) + "\\n"`` line, and closes the
    handle. POSIX guarantees ``O_APPEND`` writes are atomic per call up
    to ``PIPE_BUF`` for pipes; for regular files the kernel still serializes
    each ``write()`` against other appenders, so concurrent writers see
    interleaved lines, not interleaved bytes within a line.
    """

    def __init__(self, path: Path) -> None:
        self._path = Path(path)

    @property
    def path(self) -> Path:
        return self._path

    # ---- writers ---------------------------------------------------------

    def append(self, record: EventRecord) -> None:
        """Atomic single-line append. Creates parent dirs lazily."""
        self._path.parent.mkdir(parents=True, exist_ok=True)
        line = json.dumps(record.to_dict(), separators=(",", ":"), ensure_ascii=False)
        with self._path.open("a", encoding="utf-8") as fh:
            fh.write(line + "\n")

    def emit(
        self,
        kind: str,
        *,
        agent_id: str | None = None,
        ts_ms: float | None = None,
        **payload: Any,
    ) -> EventRecord:
        """Construct + append in one call. Returns the recorded event."""
        record = EventRecord(
            kind=kind,
            ts_ms=_now_ms() if ts_ms is None else float(ts_ms),
            agent_id=agent_id,
            payload=dict(payload),
        )
        self.append(record)
        return record

    # ---- readers ---------------------------------------------------------

    def read_all(self) -> list[EventRecord]:
        """Parse every JSONL line. Empty/missing file → ``[]``."""
        if not self._path.is_file():
            return []
        out: list[EventRecord] = []
        with self._path.open("r", encoding="utf-8") as fh:
            for raw_line in fh:
                line = raw_line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError as exc:
                    raise AgentHostError(
                        f"events.jsonl contains malformed JSON: {exc}"
                    ) from exc
                if not isinstance(obj, dict):
                    raise AgentHostError(
                        "events.jsonl entries must be JSON objects"
                    )
                out.append(EventRecord.from_mapping(obj))
        return out

    async def tail(
        self,
        *,
        follow: bool = True,
        poll_seconds: float = 0.25,
        since_offset: int = 0,
    ):
        """Yield records from ``since_offset`` to EOF, then optionally follow.

        Behaviour:

        - Opens the file (if it exists), seeks to ``since_offset``, and
          yields every complete JSONL record up to EOF.
        - When ``follow=True``, sleeps ``poll_seconds`` and re-reads from
          the last position. Tolerates the file growing or being truncated
          / deleted.
        - When ``follow=False``, returns once EOF is reached on the
          initial scan.
        - Caller cancels by cancelling the awaiting task.

        Pending bytes that don't end in ``\\n`` are buffered until the
        line completes (handles writers mid-append).
        """
        offset = int(since_offset)
        pending = ""
        first_pass_done = False

        while True:
            try:
                with self._path.open("r", encoding="utf-8") as fh:
                    fh.seek(offset)
                    chunk = fh.read()
                    offset = fh.tell()
            except FileNotFoundError:
                if first_pass_done:
                    # File rotated away after we'd seen it once — stop cleanly.
                    return
                if not follow:
                    return
                await asyncio.sleep(poll_seconds)
                continue

            if chunk:
                pending += chunk
                while "\n" in pending:
                    line, pending = pending.split("\n", 1)
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        obj = json.loads(line)
                    except json.JSONDecodeError:
                        # Skip malformed lines in follow-mode rather than
                        # crashing the tail consumer.
                        continue
                    if not isinstance(obj, dict):
                        continue
                    yield EventRecord.from_mapping(obj)

            first_pass_done = True

            if not follow:
                return

            await asyncio.sleep(poll_seconds)
