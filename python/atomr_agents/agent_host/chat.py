"""Local CLI channel + AgentRouter — M2 plumbing.

Thin Python facade over ``atomr_agents._native.host`` for M2.

:class:`AgentRouter` delegates to ``_native.host.AgentRouter`` for
pin/route storage; the wrapper adds mirror dicts so callers (the
``atomr-host gateway show`` CLI in particular) can iterate over the
pin maps, and translates the native ``None`` "no route" return into
:class:`AgentHostError`.

:func:`render_chat_preview` stays Python-side because the Python
:class:`LoadedAgent` is itself a facade dataclass (not the native
``LoadedAgent`` the native preview expects). The output format matches
what ``atomr-host agent show`` surfaces so authors can sanity-check
loaded state by chatting.

Public API
----------

- :func:`build_chat_callable` — wrap a :class:`LoadedAgent` as a
  ``_native.callable.Callable``. Tools/persona/rules are baked into a
  closure; calling the returned callable produces a structured
  response dict.
- :class:`AgentRouter` — facade over ``_h.AgentRouter`` mapping
  ``(channel_id, peer)`` to an agent id. M7 will extend the gateway
  layer; this thin shim stays stable.
- :class:`ChatSession` — boots a :class:`ChannelHarness`, attaches an
  :class:`InMemoryProvider`, opens a thread bound to the agent's
  callable, and persists the JSONL turn log to
  ``agents/<id>/state/threads/<channel>/<thread>.jsonl``.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Mapping

from atomr_agents._native import host as _h

from .errors import AgentHostError
from .loader import LoadedAgent
from .skills import select_skills_for

try:
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover
    _native = None

__all__ = [
    "AgentRouter",
    "ChatSession",
    "build_chat_callable",
    "render_chat_preview",
    "thread_log_path",
]


# ---------- preview responder ------------------------------------------------


def render_chat_preview(loaded: LoadedAgent, user_message: str) -> str:
    """Build the deterministic M2 response string.

    Format::

        [<agent-id>] <identity>
        user: <message>
        rules: N | memory facts: M | skills: K (active: ...)

    Kept Python-side because the Python :class:`LoadedAgent` is a
    facade dataclass — the native ``render_chat_preview`` expects a
    native ``LoadedAgent``. When the native loader path lands, this
    can delegate to ``_h.render_chat_preview`` directly.
    """
    identity = "(no persona)"
    if loaded.persona is not None:
        try:
            identity = str(loaded.persona.identity)
        except AttributeError:
            pass

    selected = select_skills_for(loaded.definition.skills, user_message)
    selected_ids = ",".join(s.id for s in selected) or "(none)"
    summary = (
        f"rules: {len(loaded.rules)} | "
        f"memory facts: {len(loaded.memory_facts)} | "
        f"skills: {len(loaded.definition.skills)} "
        f"(active: {selected_ids})"
    )
    return f"[{loaded.spec.id}] {identity}\nuser: {user_message}\n{summary}"


def build_chat_callable(
    loaded: LoadedAgent,
    *,
    responder: Callable[[LoadedAgent, str], str] | None = None,
    label: str | None = None,
) -> Any:
    """Wrap a :class:`LoadedAgent` as a native ``Callable``.

    ``responder`` defaults to :func:`render_chat_preview`. A real LLM
    integration substitutes its own ``responder`` (M9).

    The returned callable is sync — the channel inbound loop runs on
    tokio worker threads where Python's asyncio loop isn't current
    (per the comment in ``tests/test_channel.py``).
    """
    if _native is None:
        raise AgentHostError(
            "atomr_agents._native is not built — run `maturin develop` "
            "before building chat callables"
        )
    actual_responder = responder or render_chat_preview
    tag = label or f"chat:{loaded.spec.id}"

    def _respond(input_: Mapping[str, Any], _ctx: Any) -> dict[str, Any]:
        user_msg = ""
        if isinstance(input_, dict):
            raw = input_.get("user")
            if isinstance(raw, str):
                user_msg = raw
            elif raw is None:
                content = input_.get("content")
                if isinstance(content, dict) and content.get("kind") == "text":
                    text = content.get("text")
                    if isinstance(text, str):
                        user_msg = text
        text = actual_responder(loaded, user_msg)
        return {"text": text}

    return _native.callable.Callable.from_callable(_respond, tag)


# ---------- agent router (thin facade over _h.AgentRouter) ------------------


class AgentRouter:
    """Map ``(channel_id, peer)`` to an agent id.

    Thin facade over ``_native.host.AgentRouter``. The wrapper:

    - Holds the native router under ``_native``.
    - Mirrors pins in Python-side ``channel_pins`` / ``peer_pins`` dicts
      so callers (CLI ``gateway show``) can iterate the maps. The
      native router exposes only ``pin_channel``/``pin_peer``/``route``.
    - Translates the native ``None`` "no route" return into
      :class:`AgentHostError` so the Python API contract is preserved.

    M7 will replace the routing source with AGENTS.md-driven rules; the
    public surface here stays stable so the swap is a no-op for
    callers.
    """

    def __init__(
        self,
        default_agent: str | None = None,
        channel_pins: dict[str, str] | None = None,
        peer_pins: dict[tuple[str, str], str] | None = None,
    ) -> None:
        self._native = _h.AgentRouter(default_agent=default_agent)
        self.channel_pins: dict[str, str] = {}
        self.peer_pins: dict[tuple[str, str], str] = {}
        if channel_pins:
            for ch, ag in channel_pins.items():
                self.pin_channel(ch, ag)
        if peer_pins:
            for (ch, peer), ag in peer_pins.items():
                self.pin_peer(ch, peer, ag)

    @property
    def default_agent(self) -> str | None:
        return self._native.default_agent

    def pin_channel(self, channel_id: str, agent_id: str) -> None:
        self._native.pin_channel(channel_id, agent_id)
        self.channel_pins[channel_id] = agent_id

    def pin_peer(self, channel_id: str, peer: str, agent_id: str) -> None:
        self._native.pin_peer(channel_id, peer, agent_id)
        self.peer_pins[(channel_id, peer)] = agent_id

    def route(self, channel_id: str, peer: str) -> str:
        result = self._native.route(channel_id, peer)
        if result is None:
            raise AgentHostError(
                f"no route for ({channel_id!r}, {peer!r}) and no default agent set"
            )
        return result


# ---------- thread log -------------------------------------------------------


def thread_log_path(loaded: LoadedAgent, channel_id: str, thread_id: str) -> Path:
    """Compute the JSONL path for a thread under ``state/threads/<channel>/``.

    ``channel_id`` and ``thread_id`` are sanitized for filesystem use:
    forward and back slashes become ``-``, colons become ``__``.
    """
    safe_channel = channel_id.replace("/", "-").replace("\\", "-").replace(":", "__")
    safe_thread = thread_id.replace("/", "-").replace("\\", "-").replace(":", "__")
    return loaded.definition.paths.threads_dir / safe_channel / f"{safe_thread}.jsonl"


def _append_jsonl(path: Path, record: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, ensure_ascii=False) + "\n")


# ---------- chat session -----------------------------------------------------


@dataclass
class ChatSession:
    """One channel + one thread + one agent, ready to serve turns.

    Use :py:meth:`open` to spin everything up, :py:meth:`send` to push
    a user message and read back the response, and :py:meth:`close` to
    drain and tear down. The class is async; the :py:meth:`chat_loop`
    helper wraps an interactive stdio REPL on top of it.
    """

    loaded: LoadedAgent
    channel_id: str = "cli:local"
    peer: str = "user"
    persist: bool = True

    harness: Any | None = field(default=None, init=False)
    provider: Any | None = field(default=None, init=False)
    thread: Any | None = field(default=None, init=False)
    _events: Any | None = field(default=None, init=False)
    _counter: int = field(default=0, init=False)
    _log_path: Path | None = field(default=None, init=False)

    async def open(self) -> None:
        if _native is None:
            raise AgentHostError(
                "atomr_agents._native is not built — run `maturin develop` "
                "before opening a chat session"
            )
        cb = build_chat_callable(self.loaded)
        self.harness = _native.channel.ChannelHarness()
        self.provider = _native.channel.InMemoryProvider(self.channel_id)
        await self.harness.attach_memory(self.provider)
        self.thread = await self.harness.open_thread(self.channel_id, self.peer, cb)
        self._events = self.harness.events()
        if self.persist:
            self._log_path = thread_log_path(self.loaded, self.channel_id, self.thread.id)
            self._log_path.parent.mkdir(parents=True, exist_ok=True)
            _append_jsonl(
                self._log_path,
                {
                    "kind": "thread_opened",
                    "agent_id": self.loaded.spec.id,
                    "channel_id": self.channel_id,
                    "peer": self.peer,
                    "thread_id": self.thread.id,
                },
            )

    async def send(self, text: str, *, max_events: int = 16) -> str:
        """Push ``text`` as an inbound user message and return the agent's reply.

        Raises :class:`AgentHostError` if the harness drops the stream
        without emitting a ``message_sent`` event within ``max_events``.
        """
        if self.provider is None or self.harness is None or self.thread is None:
            raise AgentHostError("ChatSession.open() must be called before send()")
        self._counter += 1
        msg_id = f"cli-{self._counter}"
        if self._log_path:
            _append_jsonl(
                self._log_path,
                {"kind": "user_message", "msg_id": msg_id, "text": text},
            )
        self.provider.push_inbound(self.peer, msg_id, text)
        reply_summary: str | None = None
        for _ in range(max_events):
            ev = await self._events.recv()
            if ev is None:
                break
            kind = ev.get("kind")
            if kind == "turn_completed":
                reply_summary = ev.get("output_summary")
            if kind == "message_sent":
                break
        if reply_summary is None:
            raise AgentHostError("channel harness did not emit a turn_completed event")
        # The provider tail holds the full outbound record; the event
        # carries only a summary, but for the M2 deterministic
        # responder ``output_summary`` IS the full text.
        if self._log_path:
            _append_jsonl(
                self._log_path,
                {"kind": "agent_reply", "msg_id": msg_id, "text": reply_summary},
            )
        return reply_summary

    async def close(self) -> None:
        if self.harness is not None:
            await self.harness.shutdown()
        if self._log_path:
            _append_jsonl(self._log_path, {"kind": "thread_closed"})


# ---------- stdio REPL -------------------------------------------------------


def chat_repl(
    loaded: LoadedAgent,
    *,
    channel_id: str = "cli:local",
    peer: str = "user",
    persist: bool = True,
    in_stream: Any | None = None,
    out_stream: Any | None = None,
) -> None:
    """Run a blocking stdio chat REPL for ``loaded``.

    Each line of input becomes a user turn. ``/quit``, ``/exit`` or EOF
    exits cleanly. Used by ``atomr-host chat <agent>`` and by the M2
    tests with piped stdin.
    """
    import sys as _sys

    inp = in_stream if in_stream is not None else _sys.stdin
    out = out_stream if out_stream is not None else _sys.stdout

    async def _run() -> None:
        session = ChatSession(loaded=loaded, channel_id=channel_id, peer=peer, persist=persist)
        await session.open()
        out.write(
            f"atomr-host chat — agent: {loaded.spec.id} ({loaded.spec.model})\n"
            f"Type your message and press enter. /quit to exit.\n"
        )
        out.flush()
        try:
            while True:
                out.write("> ")
                out.flush()
                line = inp.readline()
                if not line:
                    break
                text = line.rstrip("\n").rstrip("\r")
                if text in {"/quit", "/exit"}:
                    break
                if not text:
                    continue
                reply = await session.send(text)
                out.write(reply + "\n")
                out.flush()
        finally:
            await session.close()

    asyncio.run(_run())
