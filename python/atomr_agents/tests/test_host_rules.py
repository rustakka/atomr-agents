"""Tests for ``atomr_agents.agent_host.rules`` — the RULES-rendering slice
of M3.

Most assertions hit the pure-Python block renderers and ``build_system_prompt``
over an ``AgentLoader.parse(...)`` result, so they don't need the PyO3
extension. The persona-block and chat-prompt-template tests do — they're
gated by a ``requires_native`` marker that probes ``BaseException`` so
PyO3 ``PanicException`` doesn't crash collection.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host import AgentLoader, HostConfig
from atomr_agents.agent_host.errors import AgentHostError
from atomr_agents.agent_host.rules import (
    build_chat_prompt_template,
    build_system_prompt,
    render_memory_block,
    render_persona_block,
    render_rules_block,
    render_user_block,
)

FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


# ---------- native probe ----------------------------------------------------

try:
    from atomr_agents import _native as _native_pkg  # type: ignore[attr-defined]

    # PyO3 panics raise ``BaseException`` (not ``Exception``) — catch broadly.
    _native_pkg.instruction.ChatPromptTemplateBuilder()
    _native_ok = True
    _native: Any | None = _native_pkg
except BaseException:  # noqa: BLE001
    _native_ok = False
    _native = None

requires_native = pytest.mark.skipif(
    not _native_ok,
    reason="atomr_agents._native not importable / instruction module unavailable",
)


# ---------- shared loader helpers ------------------------------------------


def _loader() -> AgentLoader:
    return AgentLoader(HostConfig.load(FIXTURE_ROOT))


def _parsed_alpha():
    """Pure-Python AgentDefinition — usable without the native extension."""
    return _loader().parse("alpha")


@dataclass
class _StubLoaded:
    """Minimal stand-in for :class:`LoadedAgent` that doesn't touch native."""

    rules: list[str]
    memory_facts: list[str]
    user_profile: str
    persona: Any = None
    spec: Any = None
    definition: Any = None


def _stub_from_alpha(*, persona: Any = None) -> _StubLoaded:
    """Build a ``LoadedAgent``-shaped stub from the parsed alpha fixture.

    We reuse the loader's bullet-splitting by reading the parsed body
    via the public ``rules.body`` attribute and applying the same
    Markdown-bullet conventions the rules module enforces.
    """
    defn = _parsed_alpha()
    from atomr_agents.agent_host.loader import _split_facts, _split_rules

    @dataclass
    class _DefnLike:
        agent_id: str

    return _StubLoaded(
        rules=_split_rules(defn.rules.body),
        memory_facts=_split_facts(defn.memory.body),
        user_profile=defn.user.body,
        persona=persona,
        definition=_DefnLike(agent_id=defn.agent_id),
    )


# ---------- block renderers --------------------------------------------------


def test_render_rules_block_empty_returns_empty_string() -> None:
    assert render_rules_block([]) == ""


def test_render_rules_block_emits_header_and_bullets() -> None:
    text = render_rules_block(["a", "b"])
    assert "# Rules" in text
    assert "- a" in text
    assert "- b" in text
    # Header precedes bullets.
    assert text.index("# Rules") < text.index("- a")


def test_render_memory_block_empty_and_populated() -> None:
    assert render_memory_block([]) == ""
    text = render_memory_block(["fact-one", "fact-two"])
    assert "# Memory" in text
    assert "- fact-one" in text
    assert "- fact-two" in text


def test_render_user_block_empty_and_populated() -> None:
    assert render_user_block([]) == ""
    text = render_user_block(["Name: Matt", "Style: terse"])
    assert "# About the user" in text
    assert "- Name: Matt" in text
    assert "- Style: terse" in text


def test_render_rules_block_filters_blank_items() -> None:
    # Whitespace-only entries shouldn't survive into the bulleted output.
    text = render_rules_block(["", "   ", "real"])
    assert "- real" in text
    assert "- \n" not in text + "\n"
    # Only one bullet should appear.
    assert text.count("- ") == 1


# ---------- build_system_prompt (fixture-driven) ----------------------------


@requires_native
def test_build_system_prompt_includes_all_blocks_for_alpha() -> None:
    loaded = _loader().load("alpha")
    prompt = build_system_prompt(loaded)

    assert "# Persona" in prompt
    assert "A pragmatic engineering pair-programmer." in prompt
    assert "# Rules" in prompt
    assert "# Memory" in prompt
    assert "# About the user" in prompt
    # Persona block precedes Rules in the composed prompt.
    assert prompt.index("# Persona") < prompt.index("# Rules") < prompt.index(
        "# Memory"
    ) < prompt.index("# About the user")
    # Trait bullets surface.
    assert "rigorous" in prompt


def test_build_system_prompt_without_persona_omits_persona_block() -> None:
    # Use the parsed-only stub so this test runs even when native is absent.
    stub = _stub_from_alpha(persona=None)
    prompt = build_system_prompt(stub)
    assert "# Persona" not in prompt
    # Other blocks still render from the fixture.
    assert "# Rules" in prompt
    assert "# Memory" in prompt
    assert "# About the user" in prompt


def test_build_system_prompt_all_empty_returns_identity_fallback() -> None:
    empty = _StubLoaded(rules=[], memory_facts=[], user_profile="", persona=None)
    # No spec / definition — fallback to a generic placeholder.
    prompt = build_system_prompt(empty)
    assert prompt.strip() != ""
    assert "You are " in prompt


def test_build_system_prompt_respects_user_facts_override() -> None:
    stub = _stub_from_alpha(persona=None)
    prompt = build_system_prompt(stub, user_facts=["override: yes"])
    assert "- override: yes" in prompt
    # The override replaces the parsed bullets — fixture's "Name: Matt"
    # should not appear when user_facts is provided explicitly.
    assert "Name: Matt" not in prompt


# ---------- build_chat_prompt_template --------------------------------------


@requires_native
def test_build_chat_prompt_template_renders_user_message() -> None:
    loaded = _loader().load("alpha")
    template = build_chat_prompt_template(loaded)
    assert hasattr(template, "render")
    rendered = template.render({"user_message": "hi"})
    assert isinstance(rendered, list)
    user_msgs = [m for m in rendered if getattr(m, "role", None) == "user"]
    assert len(user_msgs) == 1
    assert user_msgs[0].content == "hi"
    # System message should carry the composed Markdown prompt.
    system_msgs = [m for m in rendered if getattr(m, "role", None) == "system"]
    assert len(system_msgs) == 1
    assert "# Rules" in system_msgs[0].content


@pytest.mark.skipif(_native_ok, reason="native is importable — fallback path not exercised")
def test_build_chat_prompt_template_raises_without_native() -> None:
    """When _native is missing the helper must raise AgentHostError."""
    stub = _StubLoaded(rules=[], memory_facts=[], user_profile="", persona=None)
    with pytest.raises(AgentHostError):
        build_chat_prompt_template(stub)
