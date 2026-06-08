"""Tests for the Claude Agent SDK harness Python surface.

Three tiers:

* **Pure-logic** — exercise the wrapper's normalization / options helpers with
  no native extension and no `claude-agent-sdk`. Always run.
* **Native** — `AgentSdkHarness.mock()` round-trips through the Rust harness
  over `MockBackend`. Skipped when the native extension isn't built.
* **Live** — drives the real `claude-agent-sdk` against an in-process atomr
  tool. Skipped unless `ANTHROPIC_API_KEY`, the `claude` CLI, and the SDK are
  all present. **Uses real Anthropic credits** — kept tiny.
"""

from __future__ import annotations

import os
import shutil

import pytest

from atomr_agents import agent_sdk as asdk

# ----- tier 1: pure logic (no native, no SDK) -------------------------------


class SystemMessage:  # noqa: D401 - fake SDK message, matched by class name
    def __init__(self):
        self.subtype = "init"
        self.session_id = "sess-1"
        self.model = "claude-opus-4-8"
        self.data = {"tools": ["Read", "Bash"], "mcp_servers": [{"name": "atomr"}]}


class TextBlock:
    def __init__(self, text):
        self.text = text


class ToolUseBlock:
    def __init__(self):
        self.id = "tu-1"
        self.name = "add"
        self.input = {"a": 2, "b": 3}


class AssistantMessage:
    def __init__(self):
        self.content = [TextBlock("hello"), ToolUseBlock()]


class ResultMessage:
    def __init__(self):
        self.subtype = "success"
        self.result = "done"
        self.session_id = "sess-1"
        self.num_turns = 2
        self.total_cost_usd = 0.01
        self.usage = {"input_tokens": 10, "output_tokens": 5}


def test_normalize_system():
    n = asdk._normalize(SystemMessage())
    assert n["type"] == "system"
    assert n["session_id"] == "sess-1"
    assert n["tools"] == ["Read", "Bash"]
    assert n["mcp_servers"] == ["atomr"]


def test_normalize_assistant_blocks():
    n = asdk._normalize(AssistantMessage())
    assert n["type"] == "assistant"
    kinds = [b["kind"] for b in n["blocks"]]
    assert kinds == ["text", "tool_use"]
    assert n["blocks"][1]["name"] == "add"
    assert n["blocks"][1]["input"] == {"a": 2, "b": 3}


def test_normalize_result():
    n = asdk._normalize(ResultMessage())
    assert n["type"] == "result"
    assert n["subtype"] == "success"
    assert n["num_turns"] == 2
    assert n["cost_usd"] == 0.01
    assert n["usage"]["input_tokens"] == 10


def test_external_mcp_translation():
    stdio = asdk._external_mcp({"transport": "stdio", "command": "npx", "args": ["-y"]})
    assert stdio == {"command": "npx", "args": ["-y"], "env": {}}
    assert asdk._external_mcp({"transport": "in_process", "name": "atomr"}) is None
    sse = asdk._external_mcp({"transport": "sse", "url": "https://x/y"})
    assert sse["type"] == "sse"


def test_jsonsafe_falls_back_to_repr():
    assert asdk._jsonsafe({"a": 1}) == {"a": 1}
    obj = object()
    assert isinstance(asdk._jsonsafe(obj), str)


def test_usage_handles_dict_and_object():
    assert asdk._usage({"input_tokens": 3})["input_tokens"] == 3
    assert asdk._usage(None)["output_tokens"] == 0


# ----- tier 2: native (mock backend) ----------------------------------------

_NATIVE = asdk.AgentSdkHarness is not None
native_only = pytest.mark.skipif(not _NATIVE, reason="native extension not built")


@native_only
async def test_mock_harness_run():
    h = asdk.AgentSdkHarness.mock()
    assert h.backend_name == "mock"
    result = await h.run({"prompt": "ping"})
    assert result["subtype"] == "success"


@native_only
async def test_mock_session_lifecycle():
    h = asdk.AgentSdkHarness.mock()
    sess = await h.session({})
    assert sess.session_id
    assert h.live_count() == 1
    await sess.close()
    assert h.live_count() == 0


# ----- tier 3: live SDK (real credits) --------------------------------------

_LIVE = (
    _NATIVE
    and asdk.sdk_available()
    and bool(os.environ.get("ANTHROPIC_API_KEY"))
    and shutil.which("claude") is not None
)
live_only = pytest.mark.skipif(
    not _LIVE, reason="needs ANTHROPIC_API_KEY + `claude` CLI + claude-agent-sdk"
)


@live_only
async def test_live_in_process_tool(tmp_path):
    """Drive the real SDK against an in-process atomr tool. Tiny + budgeted."""

    async def add(args):
        return {"content": [{"type": "text", "text": str(args["a"] + args["b"])}]}

    h = asdk.harness(tools=[add], spec={"default_max_turns": 2})
    result = await h.run(
        {
            "prompt": "Use the add tool to compute 2 + 3 and report only the number.",
            "cwd": str(tmp_path),
            "allowed_tools": ["mcp__atomr__add"],
            "max_cost_usd": 0.10,
        }
    )
    assert result["is_error"] is False
    assert result.get("cost_usd") is not None
