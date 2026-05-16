# MCP bridge (M8)

The host can expose Model Context Protocol (MCP) servers as tools the
agent calls during a turn. M8 ships the **substrate** — a config file
format, a loader, an `McpBridge` that takes a Python-side
`mock_handler` for testing — without yet shelling out to a real MCP
server subprocess. The handler swap to a real `mcp.ClientSession`
lands in a follow-up.

## Tool config on disk

```yaml
# <root>/tools/fs-server.yaml
kind: mcp
command:
  - npx
  - "@modelcontextprotocol/server-fs"
  - .
env:
  MCP_FS_ROOT: /tmp/sandbox
description: Filesystem MCP server scoped to /tmp/sandbox.
```

- `kind` must be `mcp`. Other kinds are ignored by `load_mcp_servers`
  for forward compatibility.
- `command` must be a non-empty **list of strings** — a single string
  is rejected so the launch path stays explicit.
- `env` is a string→string mapping passed to the MCP subprocess when
  the real transport lands.

## CLI

```bash
atomr-host mcp add fs-server \
    --command 'npx @modelcontextprotocol/server-fs .' \
    --description 'fs server scoped to /tmp/sandbox'

atomr-host mcp ls
# fs-server  cmd=npx @modelcontextprotocol/server-fs . — fs server scoped to /tmp/sandbox
```

`--command` is shell-quoted on the way in (`shlex.split`) and stored
as a list so reads round-trip cleanly.

## Programmatic API

```python
from atomr_agents.agent_host import (
    HostConfig, load_mcp_servers, McpBridge, MCPToolSpec,
)

cfg = HostConfig.load_default()
servers = load_mcp_servers(cfg.paths)
config = servers[0]

# M8 ships a stub bridge. Wire your own handler for tests.
async def handler(tool_name, args):
    return {"echo": args}

tools = [MCPToolSpec(name="echo", description="echo args back")]
bridge = McpBridge(config, tools=tools, mock_handler=handler)

result = await bridge.call("echo", {"x": 1})   # → {"echo": {"x": 1}}
descriptors = bridge.to_tool_descriptors()      # native ToolDescriptors when _native is built
```

`to_tool_descriptors()` defers the `_native` import. When the
extension is built it returns real `ToolDescriptor` instances; when
it isn't, it falls back to the raw `MCPToolSpec` list so callers can
still introspect.

## Why a stub?

A production MCP client launches a subprocess and speaks JSON-RPC over
stdio. The shape of that interaction — `(tool_name, arguments) →
result` — is what `McpBridge.call` exposes. By keeping the transport
behind a `mock_handler` parameter, M8 ships the contract and tests
without taking on the `mcp` Python dependency. The follow-up swaps
the no-handler branch from raising to launching a subprocess; the
public API stays unchanged.

## Failure modes

- `MCPServerConfig` from a malformed yaml → `AgentSpecError`.
- `McpBridge.call(unknown_tool, ...)` → `AgentHostError`.
- `McpBridge.call(...)` with `mock_handler=None` (no real transport
  yet) → `AgentSpecError` explicitly noting M8 status.
