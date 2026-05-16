"""Multi-channel gateway — M7 facade.

Thin Python facade over ``atomr_agents._native.host`` (aliased ``_h``).
Native primitives this module relies on:

* ``_h.AgentsRoutingRules`` (frozen) — same shape as the Python dataclass
  exposed here. Re-built as a Python dataclass so tests (and authoring
  code) can construct rules directly via kwargs; the native type has
  no ``__init__``.
* ``_h.parse_agents_md`` / ``_h.load_agents_md`` — exist but accept only
  a stricter bullet grammar. We keep the Python parser here because the
  test surface (alt phrasings, malformed-bullet tolerance, ``source_path``
  on missing files) extends what native covers.
* ``_h.AgentRouter`` — already wrapped by :mod:`.chat`. ``build_router``
  uses that wrapper so the CLI's ``.channel_pins`` / ``.peer_pins``
  access keeps working.
* ``_h.Gateway`` — sync Rust gateway that takes a ``HostRuntime``; the
  async :class:`Gateway` below orchestrates :class:`ChatSession` objects
  (M2 plumbing) and is the entry point Python callers want today.

When the corresponding native pieces grow to cover the full surface,
this file's body collapses to a handful of re-exports — the public
names listed in ``__all__`` are the contract.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from pathlib import Path

from atomr_agents._native import host as _h

from .chat import AgentRouter, ChatSession
from .config import HostConfig
from .layout import HostPaths
from .loader import AgentLoader, LoadedAgent

__all__ = [
    "AgentsRoutingRules",
    "Gateway",
    "build_router",
    "load_agents_md",
    "parse_agents_md",
]


# ---------- AGENTS.md routing rules ----------------------------------------


@dataclass(frozen=True)
class AgentsRoutingRules:
    """Routing rules parsed from ``<root>/AGENTS.md``.

    Mirrors :class:`_h.AgentsRoutingRules` but exposes a kwargs
    constructor (the native type is frozen with no ``__init__``).
    """

    default_agent: str | None = None
    channel_pins: dict[str, str] = field(default_factory=dict)
    peer_pins: dict[tuple[str, str], str] = field(default_factory=dict)
    source_path: Path | None = None


# ---------- AGENTS.md parser ----------------------------------------------
#
# The native ``_h.parse_agents_md`` exists but rejects several phrasings
# the test surface accepts (bare agent id, ``=>`` arrow, colon-pin with
# colon-containing channel ids, etc.). The Python parser below is a
# strict superset; once native parity lands the body can be replaced
# with ``return AgentsRoutingRules(**_h.parse_agents_md(text).__dict__)``.


_BULLET_PREFIXES: tuple[str, ...] = ("- ", "* ", "+ ")
_ARROWS: tuple[str, ...] = ("→", "->")
_DEFAULT_BULLET_PREFIXES: tuple[str, ...] = (
    "any unmatched message:",
    "any unmatched:",
    "default:",
    "fallback:",
)
_SECTIONS: dict[str, str] = {
    "## Defaults": "defaults",
    "## Channel pins": "channel_pins",
    "## Peer pins": "peer_pins",
}


def _strip_bullet(line: str) -> str | None:
    stripped = line.strip()
    if not stripped:
        return None
    for prefix in _BULLET_PREFIXES:
        if stripped.startswith(prefix):
            return stripped[len(prefix) :].strip()
    return None


def _split_arrow(text: str) -> tuple[str, str] | None:
    for arrow in _ARROWS:
        idx = text.find(arrow)
        if idx != -1:
            return text[:idx].strip(), text[idx + len(arrow) :].strip()
    return None


def _parse_default_bullet(content: str) -> str | None:
    text = content.strip()
    if not text:
        return None
    lowered = text.lower()
    for marker in _DEFAULT_BULLET_PREFIXES:
        if lowered.startswith(marker):
            text = text[len(marker) :].strip()
            break
    while True:
        if text.startswith("=>"):
            text = text[2:].strip()
            continue
        matched = False
        for arrow in _ARROWS:
            if text.startswith(arrow):
                text = text[len(arrow) :].strip()
                matched = True
                break
        if not matched:
            break
    return text or None


def _parse_channel_bullet(content: str) -> tuple[str, str] | None:
    text = content.strip()
    if not text:
        return None
    arrow_split = _split_arrow(text)
    if arrow_split is not None:
        channel, agent = arrow_split
        return (channel, agent) if channel and agent else None
    # Colon fallback (split on the LAST colon so ``discord:server-1``
    # works as a channel id).
    if ":" in text:
        idx = text.rfind(":")
        channel = text[:idx].strip()
        agent = text[idx + 1 :].strip()
        if channel and agent:
            return channel, agent
    return None


def _parse_peer_bullet(content: str) -> tuple[str, str, str] | None:
    text = content.strip()
    if not text:
        return None
    arrow_split = _split_arrow(text)
    if arrow_split is None:
        return None
    lhs, agent = arrow_split
    if not agent:
        return None
    parts = lhs.rsplit(None, 1)
    if len(parts) != 2:
        return None
    channel, peer = parts[0].strip(), parts[1].strip()
    if not channel or not peer:
        return None
    return channel, peer, agent


def parse_agents_md(text: str, *, source_path: Path | None = None) -> AgentsRoutingRules:
    """Parse the body of an AGENTS.md document.

    Tolerant: missing sections are fine; lines that don't match the
    expected bullet shape are silently ignored.
    """
    default_agent: str | None = None
    channel_pins: dict[str, str] = {}
    peer_pins: dict[tuple[str, str], str] = {}
    current: str | None = None

    for raw_line in text.splitlines():
        stripped = raw_line.rstrip().strip()
        if stripped.startswith("## "):
            current = _SECTIONS.get(stripped)
            continue
        if stripped.startswith("#"):
            current = None
            continue
        if current is None:
            continue
        content = _strip_bullet(raw_line)
        if content is None:
            continue
        if current == "defaults":
            if default_agent is None:
                parsed = _parse_default_bullet(content)
                if parsed is not None:
                    default_agent = parsed
        elif current == "channel_pins":
            parsed_ch = _parse_channel_bullet(content)
            if parsed_ch is not None:
                channel_pins[parsed_ch[0]] = parsed_ch[1]
        elif current == "peer_pins":
            parsed_peer = _parse_peer_bullet(content)
            if parsed_peer is not None:
                channel_id, peer, agent_id = parsed_peer
                peer_pins[(channel_id, peer)] = agent_id

    return AgentsRoutingRules(
        default_agent=default_agent,
        channel_pins=channel_pins,
        peer_pins=peer_pins,
        source_path=source_path,
    )


def load_agents_md(host_paths: HostPaths) -> AgentsRoutingRules:
    """Read ``<root>/AGENTS.md`` and parse it.

    Returns an empty :class:`AgentsRoutingRules` (with ``source_path``
    still set to the looked-up file) when the file is missing.
    """
    path = host_paths.agents_md
    if not path.is_file():
        return AgentsRoutingRules(source_path=path)
    return parse_agents_md(path.read_text(encoding="utf-8"), source_path=path)


# ---------- router construction --------------------------------------------


def build_router(
    config: HostConfig,
    *,
    agents_md: AgentsRoutingRules | None = None,
) -> AgentRouter:
    """Combine ``config.default_agent`` and AGENTS.md rules into a router.

    Precedence: an AGENTS.md ``## Defaults`` value wins over
    ``config.default_agent``. Channel and peer pins flow through.

    Returns the :class:`.chat.AgentRouter` (the M2 Python wrapper) so
    callers retain ``.channel_pins`` / ``.peer_pins`` introspection;
    the native ``_h.AgentRouter`` only exposes ``route`` and
    ``default_agent`` getters.
    """
    rules = agents_md if agents_md is not None else load_agents_md(config.paths)
    default = rules.default_agent or config.default_agent
    router = AgentRouter(default_agent=default)
    for channel_id, agent_id in rules.channel_pins.items():
        router.pin_channel(channel_id, agent_id)
    for (channel_id, peer), agent_id in rules.peer_pins.items():
        router.pin_peer(channel_id, peer, agent_id)
    return router


# ---------- Gateway --------------------------------------------------------


class Gateway:
    """Orchestrate multiple :class:`ChatSession` objects across channels.

    Caches one :class:`LoadedAgent` per agent id (so persona/rules/memory
    are shared) and one :class:`ChatSession` per ``(channel_id, peer)``
    key (so threads stay isolated).

    The native ``_h.Gateway`` is sync and takes a ``HostRuntime`` — it
    serves a different layer (raw Rust callers). This Python class is
    the async-friendly orchestrator the rest of the Python harness
    expects.
    """

    def __init__(
        self,
        config: HostConfig,
        *,
        router: AgentRouter | None = None,
    ) -> None:
        self._config = config
        self._loader = AgentLoader(config)
        self._router = router if router is not None else build_router(config)
        self._agents: dict[str, LoadedAgent] = {}
        self._sessions: dict[tuple[str, str], ChatSession] = {}
        self._lock = asyncio.Lock()

    @property
    def router(self) -> AgentRouter:
        return self._router

    @property
    def config(self) -> HostConfig:
        return self._config

    def open_session_ids(self) -> list[tuple[str, str]]:
        """List currently-open ``(channel_id, peer)`` keys."""
        return list(self._sessions.keys())

    async def session_for(self, channel_id: str, peer: str) -> ChatSession:
        """Get-or-open a :class:`ChatSession` for ``(channel_id, peer)``."""
        key = (channel_id, peer)
        async with self._lock:
            existing = self._sessions.get(key)
            if existing is not None:
                return existing

            agent_id = self._router.route(channel_id, peer)
            loaded = self._agents.get(agent_id)
            if loaded is None:
                loaded = self._loader.load(agent_id)
                self._agents[agent_id] = loaded

            session = ChatSession(
                loaded=loaded,
                channel_id=channel_id,
                peer=peer,
                persist=True,
            )
            await session.open()
            self._sessions[key] = session
            return session

    async def send(self, channel_id: str, peer: str, text: str) -> str:
        """One-shot: get-or-open the session and push ``text``."""
        session = await self.session_for(channel_id, peer)
        return await session.send(text)

    async def close(self) -> None:
        """Close every open session in parallel."""
        async with self._lock:
            sessions = list(self._sessions.values())
            self._sessions.clear()
        if not sessions:
            return
        await asyncio.gather(
            *(s.close() for s in sessions),
            return_exceptions=True,
        )
