"""Tests for the M8 MCP-bridge stub.

These tests are pure-Python: they don't depend on the PyO3 ``_native``
extension being importable, and they don't drive a real MCP subprocess.
The native path of :py:meth:`McpBridge.to_tool_descriptors` is gated
behind a probe so the test still runs when ``_native`` isn't available.
"""

from __future__ import annotations

import asyncio
import importlib
from pathlib import Path
from typing import Any

import pytest

from atomr_agents.agent_host.errors import AgentHostError, AgentSpecError
from atomr_agents.agent_host.layout import HostPaths
from atomr_agents.agent_host.mcp import (
    MCPServerConfig,
    MCPToolSpec,
    McpBridge,
    load_mcp_servers,
    scaffold_mcp_tool,
)

yaml = pytest.importorskip("yaml")


def _native_available() -> bool:
    try:
        importlib.import_module("atomr_agents._native")
    except ImportError:
        return False
    return True


# ---------- MCPServerConfig.command validation ------------------------------


def test_load_rejects_empty_command(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "fs.yaml").write_text(
        "id: fs\nkind: mcp\ncommand: []\n", encoding="utf-8"
    )
    with pytest.raises(AgentSpecError):
        load_mcp_servers(host)


def test_load_rejects_string_command(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "fs.yaml").write_text(
        'id: fs\nkind: mcp\ncommand: "npx fs-server"\n', encoding="utf-8"
    )
    with pytest.raises(AgentSpecError):
        load_mcp_servers(host)


def test_load_rejects_command_with_non_strings(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "fs.yaml").write_text(
        "id: fs\nkind: mcp\ncommand: [npx, 123]\n", encoding="utf-8"
    )
    with pytest.raises(AgentSpecError):
        load_mcp_servers(host)


def test_load_accepts_valid_command(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "fs.yaml").write_text(
        "id: fs\nkind: mcp\ncommand: [npx, fs-server]\n", encoding="utf-8"
    )
    [cfg] = load_mcp_servers(host)
    assert cfg.command == ["npx", "fs-server"]


# ---------- load_mcp_servers behaviour --------------------------------------


def test_load_returns_one_per_mcp_yaml(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "fs.yaml").write_text(
        "id: fs\nkind: mcp\ncommand: [npx, fs-server]\n",
        encoding="utf-8",
    )
    out = load_mcp_servers(host)
    assert len(out) == 1
    [cfg] = out
    assert isinstance(cfg, MCPServerConfig)
    assert cfg.id == "fs"
    assert cfg.command == ["npx", "fs-server"]
    assert cfg.source_path == host.tools_dir / "fs.yaml"


def test_load_ignores_non_mcp_kinds(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()
    (host.tools_dir / "mcp_fs.yaml").write_text(
        "id: fs\nkind: mcp\ncommand: [npx, fs-server]\n",
        encoding="utf-8",
    )
    (host.tools_dir / "fn_echo.yaml").write_text(
        "id: echo\nkind: function\nentrypoint: pkg.module:echo\n",
        encoding="utf-8",
    )
    out = load_mcp_servers(host)
    assert [cfg.id for cfg in out] == ["fs"]


def test_load_missing_tools_dir_returns_empty(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path / "no_root")
    # Deliberately do NOT call host.ensure() — tools_dir won't exist.
    assert load_mcp_servers(host) == []


# ---------- scaffold_mcp_tool -----------------------------------------------


def test_scaffold_writes_and_round_trips(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()

    out_path = scaffold_mcp_tool(
        host,
        "fs",
        command=["npx", "@modelcontextprotocol/server-fs", "."],
        env={"FS_ROOT": "/tmp"},
        description="filesystem MCP server",
    )
    assert out_path == host.tools_dir / "fs.yaml"
    assert out_path.is_file()

    [cfg] = load_mcp_servers(host)
    assert cfg.id == "fs"
    assert cfg.command == ["npx", "@modelcontextprotocol/server-fs", "."]
    assert cfg.env == {"FS_ROOT": "/tmp"}
    assert cfg.description == "filesystem MCP server"


def test_scaffold_is_idempotent_without_force(tmp_path: Path) -> None:
    host = HostPaths(root=tmp_path)
    host.ensure()

    first = scaffold_mcp_tool(host, "fs", command=["npx", "fs-server"])
    first_body = first.read_text(encoding="utf-8")

    # Re-run with a different command — existing file should be untouched.
    again = scaffold_mcp_tool(host, "fs", command=["npx", "different"])
    assert again == first
    assert again.read_text(encoding="utf-8") == first_body

    # force=True overwrites.
    third = scaffold_mcp_tool(host, "fs", command=["npx", "different"], force=True)
    assert third.read_text(encoding="utf-8") != first_body
    [cfg] = load_mcp_servers(host)
    assert cfg.command == ["npx", "different"]


# ---------- McpBridge.call --------------------------------------------------


def _make_bridge(
    *,
    tools: list[MCPToolSpec] | None = None,
    handler: Any = None,
) -> McpBridge:
    cfg = MCPServerConfig(id="fs", command=["npx", "fs-server"])
    if tools is None:
        tools = [
            MCPToolSpec(name="read_file", description="Read a file."),
            MCPToolSpec(name="write_file", description="Write a file."),
        ]
    return McpBridge(cfg, tools=tools, mock_handler=handler)


def test_bridge_call_with_mock_handler() -> None:
    async def handler(name: str, args: dict) -> dict:
        return {"echo": args, "name": name}

    bridge = _make_bridge(handler=handler)
    result = asyncio.run(bridge.call("read_file", {"x": 1}))
    assert result == {"echo": {"x": 1}, "name": "read_file"}


def test_bridge_call_unknown_tool_raises() -> None:
    async def handler(name: str, args: dict) -> dict:  # pragma: no cover - never called
        return args

    bridge = _make_bridge(handler=handler)
    with pytest.raises(AgentHostError):
        asyncio.run(bridge.call("nope", {}))


def test_bridge_call_without_mock_handler_raises_agent_spec_error() -> None:
    bridge = _make_bridge(handler=None)
    with pytest.raises(AgentSpecError) as exc_info:
        asyncio.run(bridge.call("read_file", {}))
    msg = str(exc_info.value)
    assert "real MCP subprocess" in msg
    assert "mock_handler" in msg


# ---------- to_tool_descriptors --------------------------------------------


def test_to_tool_descriptors_returns_one_per_spec() -> None:
    tools = [
        MCPToolSpec(name="read_file", description="Read a file."),
        MCPToolSpec(name="write_file", description="Write a file."),
    ]
    bridge = _make_bridge(tools=tools)
    descriptors = bridge.to_tool_descriptors()
    assert isinstance(descriptors, list)
    assert len(descriptors) == 2


def test_to_tool_descriptors_fallback_when_no_native(monkeypatch: pytest.MonkeyPatch) -> None:
    """When ``_native`` can't be imported, the bridge falls back to the
    raw MCPToolSpec list so callers can still introspect.

    We simulate "_native missing" by removing the attribute from the
    ``atomr_agents`` package and pinning ``sys.modules['atomr_agents._native']``
    to ``None`` — Python's import machinery then raises ImportError on a
    ``from atomr_agents import _native`` inside the function under test.
    """

    import sys
    import atomr_agents

    # Drop the cached attribute (if present) so `from atomr_agents import
    # _native` can't short-circuit through the package namespace.
    if hasattr(atomr_agents, "_native"):
        monkeypatch.delattr(atomr_agents, "_native")
    # Force the import machinery to raise.
    monkeypatch.setitem(sys.modules, "atomr_agents._native", None)

    tools = [MCPToolSpec(name="read_file", description="Read a file.")]
    bridge = _make_bridge(tools=tools)
    out = bridge.to_tool_descriptors()
    assert out == tools  # exact list of MCPToolSpec values


@pytest.mark.skipif(not _native_available(), reason="_native extension not built")
def test_to_tool_descriptors_native_path() -> None:
    tools = [
        MCPToolSpec(name="read_file", description="Read a file."),
        MCPToolSpec(name="write_file", description="Write a file."),
    ]
    bridge = _make_bridge(tools=tools)
    descriptors = bridge.to_tool_descriptors()

    # Native descriptors carry .id and .name fields equal to the spec name.
    from atomr_agents import _native as native_pkg

    assert all(isinstance(d, native_pkg.tool.ToolDescriptor) for d in descriptors)
    assert [d.id for d in descriptors] == ["read_file", "write_file"]
    assert [d.name for d in descriptors] == ["read_file", "write_file"]
