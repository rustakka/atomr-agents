"""Claude Agent SDK harness — Python wrapper + facade.

Wraps Anthropic's programmable Claude Code agent
(`claude-agent-sdk <https://pypi.org/project/claude-agent-sdk/>`_) so it runs
as an atomr-agents harness. The Rust harness
(:mod:`atomr_agents._native.agent_sdk`) drives this wrapper over the PyO3
bridge; the wrapper builds ``ClaudeAgentOptions`` and normalizes SDK message
objects into plain dicts the Rust side can carry.

Quick start::

    import atomr_agents.agent_sdk as asdk

    # In-process atomr tool exposed to the agent as `mcp__atomr__add`.
    async def add(args):
        return {"content": [{"type": "text", "text": str(args["a"] + args["b"])}]}

    h = asdk.harness(tools=[add], spec={"default_permission_mode": "bypassPermissions"})
    result = await h.run({"prompt": "Use the add tool on 2 and 3", "cwd": "/repo",
                          "allowed_tools": ["mcp__atomr__add"]})
    print(result["result"], result["cost_usd"])

Interactive (the actor surface)::

    sess = await h.session({"cwd": "/repo"})
    await sess.query("/review the auth module")
    async for ev in sess.events():
        print(ev["kind"])
    await sess.close()

**Billing:** every run shells out to the bundled ``claude`` CLI and bills your
Anthropic API credits (``ANTHROPIC_API_KEY``, or ``CLAUDE_CODE_USE_BEDROCK`` /
``CLAUDE_CODE_USE_VERTEX``). claude.ai OAuth/subscription tokens are *not*
accepted by the Agent SDK.
"""

from __future__ import annotations

import inspect
import json
import uuid
from typing import Any, Callable, Iterable, Optional

# ----- native bridge --------------------------------------------------------

try:
    from . import _native as _native_pkg

    _guest = _native_pkg.guest
    _agent_sdk_native = _native_pkg.agent_sdk
except (ImportError, AttributeError):  # pragma: no cover - extension not built
    _native_pkg = None
    _guest = None
    _agent_sdk_native = None

# Facade re-exports of the native pyclasses.
AgentSdkHarness = getattr(_agent_sdk_native, "AgentSdkHarness", None)
AgentSdkSession = getattr(_agent_sdk_native, "AgentSdkSession", None)
AgentSdkEventStream = getattr(_agent_sdk_native, "AgentSdkEventStream", None)

# ----- optional claude-agent-sdk import -------------------------------------

try:
    import claude_agent_sdk as _sdk
    from claude_agent_sdk import (  # noqa: F401
        ClaudeAgentOptions,
        ClaudeSDKClient,
        create_sdk_mcp_server,
    )
    from claude_agent_sdk import tool as _sdk_tool

    _SDK_AVAILABLE = True
    _SDK_IMPORT_ERR: Optional[Exception] = None
except Exception as _e:  # noqa: BLE001 - any import-time failure is "not available"
    _SDK_AVAILABLE = False
    _SDK_IMPORT_ERR = _e


def _require_sdk() -> None:
    if not _SDK_AVAILABLE:
        raise RuntimeError(
            "claude-agent-sdk is not installed. Install with "
            "`pip install atomr-agents[agent-sdk]` (Python 3.10+) and set "
            f"ANTHROPIC_API_KEY. Original import error: {_SDK_IMPORT_ERR!r}"
        )


__all__ = [
    "AgentSdkHarness",
    "AgentSdkSession",
    "AgentSdkEventStream",
    "ClaudeAgentSDKBackend",
    "agent_sdk_backend",
    "harness",
    "tools_to_sdk_server",
    "sdk_available",
]


def sdk_available() -> bool:
    """Whether `claude-agent-sdk` is importable in this environment."""
    return _SDK_AVAILABLE


# ----- message normalization ------------------------------------------------
#
# Dispatch on the class *name* (not isinstance) so we don't couple to the SDK's
# exact import symbols, which shift across versions. The output dict shape must
# stay in lock step with the Rust `AgentSdkMessage` / `ContentBlock` serde.


def _jsonsafe(v: Any) -> Any:
    try:
        json.dumps(v)
        return v
    except (TypeError, ValueError):
        return repr(v)


def _usage(u: Any) -> dict:
    if u is None:
        u = {}
    get = (lambda k: u.get(k, 0)) if isinstance(u, dict) else (lambda k: getattr(u, k, 0))
    return {
        "input_tokens": int(get("input_tokens") or 0),
        "output_tokens": int(get("output_tokens") or 0),
        "cache_creation_input_tokens": int(get("cache_creation_input_tokens") or 0),
        "cache_read_input_tokens": int(get("cache_read_input_tokens") or 0),
    }


def _block(b: Any) -> dict:
    name = type(b).__name__
    if name == "TextBlock":
        return {"kind": "text", "text": getattr(b, "text", "")}
    if name == "ThinkingBlock":
        return {"kind": "thinking", "text": getattr(b, "thinking", "") or getattr(b, "text", "")}
    if name == "ToolUseBlock":
        return {
            "kind": "tool_use",
            "id": getattr(b, "id", ""),
            "name": getattr(b, "name", ""),
            "input": _jsonsafe(getattr(b, "input", {})),
        }
    if name == "ToolResultBlock":
        return {
            "kind": "tool_result",
            "tool_use_id": getattr(b, "tool_use_id", ""),
            "content": _jsonsafe(getattr(b, "content", None)),
            "is_error": bool(getattr(b, "is_error", False)),
        }
    # Unknown block: surface its text if any.
    return {"kind": "text", "text": str(getattr(b, "text", b))}


def _normalize(msg: Any) -> dict:
    name = type(msg).__name__
    if name == "SystemMessage":
        data = getattr(msg, "data", None) or {}
        sid = getattr(msg, "session_id", None)
        if sid is None and isinstance(data, dict):
            sid = data.get("session_id")
        tools = data.get("tools", []) if isinstance(data, dict) else []
        mcps = data.get("mcp_servers", []) if isinstance(data, dict) else []
        if mcps and isinstance(mcps[0], dict):
            mcps = [m.get("name", "") for m in mcps]
        return {
            "type": "system",
            "subtype": getattr(msg, "subtype", None),
            "session_id": sid,
            "model": getattr(msg, "model", None),
            "tools": [t if isinstance(t, str) else t.get("name", "") for t in tools],
            "mcp_servers": mcps,
        }
    if name == "AssistantMessage":
        return {"type": "assistant", "blocks": [_block(b) for b in getattr(msg, "content", [])]}
    if name == "ResultMessage":
        return {
            "type": "result",
            "subtype": getattr(msg, "subtype", ""),
            "result": getattr(msg, "result", None),
            "session_id": getattr(msg, "session_id", None),
            "num_turns": int(getattr(msg, "num_turns", 0) or 0),
            "cost_usd": getattr(msg, "total_cost_usd", None),
            "usage": _usage(getattr(msg, "usage", None)),
        }
    return {"type": "unknown", "repr": repr(msg)}


# ----- options construction -------------------------------------------------

_SDK_OPTION_KEYS = (
    "system_prompt",
    "allowed_tools",
    "disallowed_tools",
    "permission_mode",
    "cwd",
    "add_dirs",
    "env",
    "setting_sources",
    "model",
    "fallback_model",
    "max_turns",
    "continue_conversation",
    "resume",
    "fork_session",
    "include_partial_messages",
    "effort",
    "thinking",
)


def _filter_kwargs(cls: Any, kw: dict) -> dict:
    """Drop kwargs the installed SDK's options object doesn't accept, so the
    wrapper survives version skew without a 400 at construction."""
    try:
        params = inspect.signature(cls).parameters
    except (TypeError, ValueError):
        return kw
    # VAR_KEYWORD (**kwargs) means everything is accepted.
    if any(p.kind == inspect.Parameter.VAR_KEYWORD for p in params.values()):
        return kw
    return {k: v for k, v in kw.items() if k in params}


def _external_mcp(spec: Any) -> Optional[dict]:
    """Translate an atomr `McpServerConfig` dict into the SDK `mcp_servers`
    value. Returns `None` for in-process markers (those are injected here)."""
    if not isinstance(spec, dict):
        return spec
    transport = spec.get("transport")
    if transport == "stdio":
        return {"command": spec.get("command"), "args": spec.get("args", []), "env": spec.get("env", {})}
    if transport == "sse":
        return {"type": "sse", "url": spec.get("url"), "headers": spec.get("headers", {})}
    if transport == "http":
        return {"type": "http", "url": spec.get("url"), "headers": spec.get("headers", {})}
    if transport == "in_process":
        return None
    return spec


def _agents_from_config(agents: dict) -> dict:
    """Map atomr `SubagentDef` dicts to SDK `AgentDefinition` objects (or pass
    the dicts straight through if the SDK accepts dicts)."""
    AgentDefinition = getattr(_sdk, "AgentDefinition", None)
    if AgentDefinition is None:
        return agents
    out = {}
    for name, d in agents.items():
        out[name] = AgentDefinition(
            description=d.get("description", ""),
            prompt=d.get("prompt", ""),
            tools=d.get("tools") or None,
            model=d.get("model"),
        )
    return out


def _policy_to_callback(policy: dict) -> Callable:
    """Turn an atomr `PermissionPolicy` data descriptor into a `can_use_tool`
    callback."""
    mode = (policy or {}).get("mode", "allow_all")
    allow = set((policy or {}).get("tools", []))

    def _result(allowed: bool, tool_name: str):
        allow_cls = getattr(_sdk, "PermissionResultAllow", None)
        deny_cls = getattr(_sdk, "PermissionResultDeny", None)
        if allowed:
            return allow_cls() if allow_cls else {"behavior": "allow"}
        return deny_cls(message=f"denied by atomr policy: {tool_name}") if deny_cls else {"behavior": "deny"}

    async def can_use(tool_name, tool_input, context):  # noqa: ANN001
        if mode == "allow_all":
            return _result(True, tool_name)
        if mode == "deny_all":
            return _result(False, tool_name)
        if mode == "allow_list":
            return _result(tool_name in allow, tool_name)
        return _result(True, tool_name)  # "ask" → defer to SDK default

    return can_use


# ----- atomr tools → in-process SDK MCP server ------------------------------


def _wrap_callable_tool(t: Any):
    """Wrap a plain Python callable / class as an SDK `@tool`."""
    name = getattr(t, "__name__", "tool")
    doc = (getattr(t, "__doc__", None) or name).strip()
    schema = getattr(t, "__atomr_tool_schema__", {})

    @_sdk_tool(name, doc, schema)
    async def _adapter(args, _t=t):  # noqa: ANN001
        inst = _t() if isinstance(_t, type) else _t
        fn = getattr(inst, "invoke", None) or inst
        res = fn(args, {}) if _takes_two(fn) else fn(args)
        if inspect.iscoroutine(res):
            res = await res
        if isinstance(res, dict) and "content" in res:
            return res
        return {"content": [{"type": "text", "text": json.dumps(res)}]}

    return _adapter


def _wrap_key_tool(key: str):
    """Wrap a registered atomr tool key as an SDK `@tool`, executed through the
    Rust `Tool` machinery via the native `invoke_tool` bridge."""
    if _agent_sdk_native is None:
        raise RuntimeError("native extension not built; cannot bridge atomr tool keys")

    @_sdk_tool(key, f"atomr tool {key}", {})
    async def _adapter(args, _key=key):  # noqa: ANN001
        result = await _agent_sdk_native.invoke_tool(_key, args)
        if isinstance(result, dict) and "content" in result:
            return result
        return {"content": [{"type": "text", "text": json.dumps(result)}]}

    return _adapter


def _takes_two(fn: Any) -> bool:
    try:
        params = [
            p
            for p in inspect.signature(fn).parameters.values()
            if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD)
        ]
        return len(params) >= 2
    except (TypeError, ValueError):
        return False


def tools_to_sdk_server(tools: Iterable[Any], *, server_name: str = "atomr", version: str = "0.1.0"):
    """Build an in-process SDK MCP server from a mix of atomr tool keys
    (``str``) and Python callables/classes. Each becomes ``mcp__<server>__<name>``.
    """
    _require_sdk()
    sdk_tools = []
    for t in tools:
        if isinstance(t, str):
            sdk_tools.append(_wrap_key_tool(t))
        elif callable(t) or isinstance(t, type):
            sdk_tools.append(_wrap_callable_tool(t))
        else:
            # Assume it's already an SDK tool object.
            sdk_tools.append(t)
    return create_sdk_mcp_server(name=server_name, version=version, tools=sdk_tools)


# ----- the wrapper backend --------------------------------------------------


class ClaudeAgentSDKBackend:
    """Drives `claude-agent-sdk`. Registered under the ``agent_sdk`` guest kind;
    the Rust `PythonAgentSdkBackend` calls these methods over the PyO3 bridge.
    """

    def __init__(
        self,
        *,
        tools: Optional[Iterable[Any]] = None,
        can_use_tool: Optional[Callable] = None,
        hooks: Optional[Any] = None,
        agents: Optional[Any] = None,
        server_name: str = "atomr",
    ) -> None:
        self._tools = list(tools) if tools else []
        self._can_use_tool = can_use_tool
        self._hooks = hooks
        self._agents = agents
        self._server_name = server_name
        self._sessions: dict[str, Any] = {}

    # -- options --------------------------------------------------------------

    def _build_options(self, config: dict):
        _require_sdk()
        config = config or {}
        kw = {k: config[k] for k in _SDK_OPTION_KEYS if config.get(k) is not None}

        if self._can_use_tool is not None:
            kw["can_use_tool"] = self._can_use_tool
        elif config.get("permission_policy"):
            kw["can_use_tool"] = _policy_to_callback(config["permission_policy"])

        if self._hooks is not None:
            kw["hooks"] = self._hooks
        if self._agents is not None:
            kw["agents"] = self._agents
        elif config.get("agents"):
            kw["agents"] = _agents_from_config(config["agents"])

        mcp: dict[str, Any] = {}
        for name, spec in (config.get("mcp_servers") or {}).items():
            ext = _external_mcp(spec)
            if ext is not None:
                mcp[name] = ext
        if self._tools:
            mcp[self._server_name] = tools_to_sdk_server(self._tools, server_name=self._server_name)
        if mcp:
            kw["mcp_servers"] = mcp

        return ClaudeAgentOptions(**_filter_kwargs(ClaudeAgentOptions, kw))

    # -- one-shot query -------------------------------------------------------

    async def run(self, config: dict, prompt: str):
        _require_sdk()
        options = self._build_options(config)
        async for msg in _sdk.query(prompt=prompt, options=options):
            yield _normalize(msg)

    # -- interactive session --------------------------------------------------

    async def open_session(self, config: dict) -> str:
        _require_sdk()
        client = ClaudeSDKClient(options=self._build_options(config))
        await client.connect()
        token = uuid.uuid4().hex
        self._sessions[token] = client
        return token

    async def session_send(self, token: str, prompt: str) -> None:
        await self._sessions[token].query(prompt)

    async def session_stream(self, token: str):
        async for msg in self._sessions[token].receive_response():
            yield _normalize(msg)

    async def session_interrupt(self, token: str) -> None:
        await self._sessions[token].interrupt()

    async def session_set_mode(self, token: str, mode: str) -> None:
        fn = getattr(self._sessions[token], "set_permission_mode", None)
        if fn is None:
            raise RuntimeError("this claude-agent-sdk version has no set_permission_mode")
        await fn(mode)

    async def session_set_model(self, token: str, model: str) -> None:
        fn = getattr(self._sessions[token], "set_model", None)
        if fn is None:
            raise RuntimeError("this claude-agent-sdk version has no set_model")
        await fn(model)

    async def session_close(self, token: str) -> None:
        client = self._sessions.pop(token, None)
        if client is not None:
            await client.disconnect()


# ----- registration + convenience builders ----------------------------------


def agent_sdk_backend(
    name: str = "default",
    *,
    tools: Optional[Iterable[Any]] = None,
    can_use_tool: Optional[Callable] = None,
    hooks: Optional[Any] = None,
    agents: Optional[Any] = None,
) -> ClaudeAgentSDKBackend:
    """Create and register a :class:`ClaudeAgentSDKBackend` under *name* in the
    process-wide guest registry. Reference it via
    ``AgentSdkHarness.from_python_backend(name)``.
    """
    backend = ClaudeAgentSDKBackend(
        tools=tools, can_use_tool=can_use_tool, hooks=hooks, agents=agents
    )
    if _guest is not None:
        _guest.register_agent_sdk_factory(name, backend)
    return backend


def harness(
    *,
    name: str = "default",
    spec: Optional[dict] = None,
    tools: Optional[Iterable[Any]] = None,
    can_use_tool: Optional[Callable] = None,
    hooks: Optional[Any] = None,
    agents: Optional[Any] = None,
):
    """One-liner: register a backend and build the native harness over it.

    Returns an :class:`atomr_agents._native.agent_sdk.AgentSdkHarness`.
    """
    if AgentSdkHarness is None:
        raise RuntimeError(
            "native extension not built — run `maturin develop` or "
            "`pip install -e .[agent-sdk]`"
        )
    agent_sdk_backend(name, tools=tools, can_use_tool=can_use_tool, hooks=hooks, agents=agents)
    return AgentSdkHarness.from_python_backend(name, spec)
