"""MCP bridge — thin facade over ``atomr_agents._native.host`` (M8).

The native module ships:

* ``_h.MCPToolSpec(name, description="", schema=None)`` — frozen pyclass.
* ``_h.MCPServerConfig(id, command, env=None, tools=None)`` — frozen pyclass.
* ``_h.McpBridge(config)`` with ``set_mock`` / sync ``call`` / ``tools``.
* ``_h.load_mcp_servers(mcp_dir)`` — deserializes ``*.yaml`` strictly into
  ``MCPServerConfig`` records (no ``kind`` filter, no validation hooks).

The public contract for this Python module diverges from those native
shapes in several places. We therefore keep most of the surface in
Python and document the gaps here:

* :class:`MCPToolSpec` stays a Python frozen dataclass — tests assert
  ``list[MCPToolSpec]`` equality via ``==``, which native PyO3 frozen
  classes do not implement, and the Python field is ``input_schema``
  (not native's ``schema``).
* :class:`MCPServerConfig` stays a Python frozen dataclass — adds
  ``description`` and ``source_path`` fields that the native counterpart
  lacks (tests read both).
* :func:`load_mcp_servers` stays Python — it (a) filters by
  ``kind: mcp`` and ignores other kinds (native deserializes
  unconditionally and would error on YAMLs without a ``command``), (b)
  layers explicit type-shape errors (empty/string/non-string command,
  non-mapping env), and (c) records ``source_path`` per file.
* :class:`McpBridge` stays Python — the test contract is async
  (``await bridge.call(...)``); native ``McpBridge.call`` is sync
  (``tokio_runtime().block_on``). Python also raises
  :class:`AgentHostError` on unknown tool names and a specific
  :class:`AgentSpecError` message ("real MCP subprocess",
  ``mock_handler``) when no handler is installed, which the native
  error message does not preserve verbatim.
* :func:`scaffold_mcp_tool` stays Python — adds ``env``/``description``
  fields, idempotent ``force`` semantics, and pre-write validation that
  mirrors :func:`load_mcp_servers`. The native ``scaffold_mcp_server``
  always overwrites and only persists command.

Where native delegation IS used: :py:meth:`McpBridge.to_tool_descriptors`
constructs ``_native.tool.ToolDescriptor`` instances when the PyO3
extension is importable (fallback returns the raw
:class:`MCPToolSpec` list so tests work without ``_native``).
"""

from __future__ import annotations

import asyncio  # noqa: F401  (kept for future real-transport implementation)
import json  # noqa: F401  (kept for future real-transport implementation)
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable

from .errors import AgentHostError, AgentSpecError
from .layout import HostPaths

__all__ = [
    "MCPHandler",
    "MCPServerConfig",
    "MCPToolSpec",
    "McpBridge",
    "load_mcp_servers",
    "scaffold_mcp_tool",
]


# ---------- pure-data definitions -------------------------------------------


@dataclass(frozen=True)
class MCPToolSpec:
    """A tool exposed by an MCP server.

    Mirrors ``_native.host.MCPToolSpec`` field-for-field (name,
    description, schema). Stays Python-side because the native pyclass
    is frozen without ``__eq__`` and the tests rely on list-equality of
    :class:`MCPToolSpec` values.
    """

    name: str
    description: str
    input_schema: dict = field(default_factory=dict)


@dataclass(frozen=True)
class MCPServerConfig:
    """Loaded from ``<root>/tools/<tool-id>.yaml`` when ``kind`` is ``mcp``.

    Adds ``description`` and ``source_path`` over the native
    ``_native.host.MCPServerConfig`` — both are read by tests and the
    CLI surface and aren't carried by the Rust struct.
    """

    id: str
    command: list[str]
    env: dict[str, str] = field(default_factory=dict)
    description: str | None = None
    source_path: Path | None = None


MCPHandler = Callable[[str, dict], Awaitable[Any]]
"""(tool_name, arguments) → result. Used by the stub bridge for tests."""


# ---------- bridge -----------------------------------------------------------


class McpBridge:
    """Wraps an MCP server connection as a ToolSet for the agent host.

    M8 ships a stub that takes a Python ``mock_handler`` and a pre-built
    list of :class:`MCPToolSpec` instances. A future revision swaps the
    stub for an :class:`mcp.ClientSession` that subprocess-launches the
    server and auto-discovers tools.

    Kept Python-side: the Python contract is async (callers await
    :py:meth:`call`), the unknown-tool error is :class:`AgentHostError`,
    and the "no handler installed" error is :class:`AgentSpecError`
    with a specific message tests assert on. The native ``McpBridge``
    in ``_native.host`` is sync and raises a different error string.
    """

    def __init__(
        self,
        config: MCPServerConfig,
        *,
        tools: list[MCPToolSpec],
        mock_handler: MCPHandler | None = None,
    ) -> None:
        self._config = config
        self._tools = list(tools)
        self._by_name: dict[str, MCPToolSpec] = {t.name: t for t in self._tools}
        self._mock_handler = mock_handler

    @property
    def config(self) -> MCPServerConfig:
        return self._config

    @property
    def tools(self) -> list[MCPToolSpec]:
        return list(self._tools)

    async def call(self, tool_name: str, arguments: dict) -> Any:
        """Invoke a tool by name and return its result.

        Raises :class:`AgentHostError` when ``tool_name`` is not declared
        in :py:attr:`tools`. Raises :class:`AgentSpecError` when no
        ``mock_handler`` was supplied — the real subprocess transport is
        a post-M8 follow-up.
        """
        if tool_name not in self._by_name:
            known = ", ".join(sorted(self._by_name)) or "(none)"
            raise AgentHostError(
                f"MCP bridge {self._config.id!r}: unknown tool {tool_name!r}; "
                f"known tools: {known}"
            )
        if self._mock_handler is None:
            raise AgentSpecError(
                f"MCP bridge {self._config.id!r}: real MCP subprocess transport "
                "not implemented in M8 — pass mock_handler=... to McpBridge "
                "for tests, or wait for the follow-up that integrates "
                "`mcp.ClientSession`."
            )
        return await self._mock_handler(tool_name, arguments)

    def to_tool_descriptors(self) -> list[Any]:
        """Return one entry per tool, as a native ``ToolDescriptor`` when
        the PyO3 extension is importable; otherwise fall back to the raw
        :class:`MCPToolSpec` list so callers can still introspect.

        The JSON schema for arguments is intentionally omitted in M8 —
        the native ``ToolSchema`` constructor takes a dict and we don't
        validate yet.
        """
        try:
            from atomr_agents import _native as _native_pkg  # local import keeps tests light
        except ImportError:
            return list(self._tools)

        descriptors: list[Any] = []
        for spec in self._tools:
            descriptors.append(
                _native_pkg.tool.ToolDescriptor(
                    id=spec.name,
                    name=spec.name,
                    description=spec.description,
                )
            )
        return descriptors


# ---------- yaml IO ----------------------------------------------------------


def load_mcp_servers(host_paths: HostPaths) -> list[MCPServerConfig]:
    """Walk ``<root>/tools/*.yaml`` and return one :class:`MCPServerConfig`
    per file with ``kind: mcp``.

    Other ``kind`` values (e.g. ``function``) are ignored — they belong to
    a future tool-yaml spec. Returns an empty list when the tools
    directory is missing.

    Raises :class:`AgentSpecError` on a malformed ``mcp`` entry, including
    a missing or non-list ``command``, or a ``command`` whose entries
    aren't all strings.

    Stays Python-side because the native ``_h.load_mcp_servers`` deserializes
    every YAML into ``MCPServerConfig`` without filtering by ``kind`` and
    without surfacing the type-shape errors the test contract asserts on
    (and without populating ``description``/``source_path``).
    """
    tools_dir = host_paths.tools_dir
    if not tools_dir.is_dir():
        return []

    try:
        import yaml  # type: ignore[import-untyped]
    except ImportError as exc:  # pragma: no cover - host extra installs PyYAML
        raise AgentSpecError(
            "PyYAML is required to load MCP tool YAMLs — install atomr-agents[host]"
        ) from exc

    out: list[MCPServerConfig] = []
    for child in sorted(tools_dir.iterdir()):
        if not child.is_file() or child.suffix not in {".yaml", ".yml"}:
            continue
        try:
            raw = yaml.safe_load(child.read_text(encoding="utf-8")) or {}
        except yaml.YAMLError as exc:
            raise AgentSpecError(f"invalid YAML in {child}: {exc}") from exc
        if not isinstance(raw, dict):
            raise AgentSpecError(f"{child}: top-level must be a YAML mapping")

        kind = raw.get("kind")
        if kind != "mcp":
            # Not ours — let a future tool-yaml spec handle other kinds.
            continue

        command_raw = raw.get("command")
        if not isinstance(command_raw, list):
            raise AgentSpecError(
                f"{child}: `command` must be a list of strings "
                f"(got {type(command_raw).__name__})"
            )
        if not command_raw:
            raise AgentSpecError(f"{child}: `command` must be a non-empty list")
        if not all(isinstance(part, str) for part in command_raw):
            raise AgentSpecError(f"{child}: every entry in `command` must be a string")

        env_raw = raw.get("env") or {}
        if not isinstance(env_raw, dict):
            raise AgentSpecError(f"{child}: `env` must be a mapping when present")
        env: dict[str, str] = {}
        for k, v in env_raw.items():
            if not isinstance(k, str) or not isinstance(v, str):
                raise AgentSpecError(
                    f"{child}: every entry in `env` must map string→string"
                )
            env[k] = v

        description_raw = raw.get("description")
        if description_raw is not None and not isinstance(description_raw, str):
            raise AgentSpecError(f"{child}: `description` must be a string when present")

        tool_id = str(raw.get("id") or child.stem)
        out.append(
            MCPServerConfig(
                id=tool_id,
                command=list(command_raw),
                env=env,
                description=description_raw,
                source_path=child,
            )
        )
    return out


def scaffold_mcp_tool(
    host_paths: HostPaths,
    tool_id: str,
    *,
    command: list[str],
    env: dict[str, str] | None = None,
    description: str | None = None,
    force: bool = False,
) -> Path:
    """Write ``<root>/tools/<tool_id>.yaml`` with ``kind: mcp``.

    Idempotent unless ``force`` is set — an existing file is left alone.
    Validates ``command`` the same way :func:`load_mcp_servers` does so
    the scaffold can't write a YAML it would itself reject on read-back.

    Stays Python-side because the native ``scaffold_mcp_server`` always
    overwrites, doesn't persist ``env``/``description``, and uses a
    flatter YAML layout that omits the ``kind: mcp`` discriminator the
    Python loader keys off.
    """
    if not isinstance(command, list) or not command:
        raise AgentSpecError("scaffold_mcp_tool: `command` must be a non-empty list")
    if not all(isinstance(part, str) for part in command):
        raise AgentSpecError("scaffold_mcp_tool: every entry in `command` must be a string")
    env = dict(env or {})
    for k, v in env.items():
        if not isinstance(k, str) or not isinstance(v, str):
            raise AgentSpecError(
                "scaffold_mcp_tool: every entry in `env` must map string→string"
            )

    try:
        import yaml  # type: ignore[import-untyped]
    except ImportError as exc:  # pragma: no cover - host extra installs PyYAML
        raise AgentSpecError(
            "PyYAML is required to scaffold MCP tool YAMLs — install atomr-agents[host]"
        ) from exc

    host_paths.tools_dir.mkdir(parents=True, exist_ok=True)
    target = host_paths.tools_dir / f"{tool_id}.yaml"
    if target.is_file() and not force:
        return target

    payload: dict[str, Any] = {
        "id": tool_id,
        "kind": "mcp",
        "command": list(command),
    }
    if env:
        payload["env"] = env
    if description is not None:
        payload["description"] = description

    target.write_text(yaml.safe_dump(payload, sort_keys=False), encoding="utf-8")
    return target
