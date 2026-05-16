"""Tests for AgentLoader — fixture-driven parse + (native-gated) materialize.

The parse path runs without the PyO3 extension. The load path requires
``atomr_agents._native`` and is skipped when the extension is absent.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from atomr_agents.agent_host import AgentLoader, HostConfig
from atomr_agents.agent_host.errors import AgentNotFoundError, AgentSpecError

pytest.importorskip("yaml")

FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


def _loader() -> AgentLoader:
    return AgentLoader(HostConfig.load(FIXTURE_ROOT))


# ---------- parse-phase tests ----------------------------------------------


def test_agent_ids_from_fixture() -> None:
    loader = _loader()
    assert loader.agent_ids() == ["alpha"]


def test_parse_alpha_spec_fields() -> None:
    defn = _loader().parse("alpha")
    assert defn.agent_id == "alpha"
    assert defn.model == "gpt-4o"
    assert defn.max_iterations == 6
    assert defn.token_budget == 4096
    assert defn.time_budget_ms == 45_000
    assert defn.money_budget_usd == pytest.approx(0.50)
    assert defn.skillset_id == "alpha-skills"
    assert defn.skillset_version == "0.2.0"


def test_parse_alpha_persona_frontmatter() -> None:
    defn = _loader().parse("alpha")
    assert defn.soul.frontmatter["identity"].startswith("A pragmatic")
    assert defn.soul.frontmatter["style"]["tone"] == "dry"
    assert defn.soul.frontmatter["style"]["verbosity"] == 1
    traits = defn.soul.frontmatter["traits"]
    labels = [t["label"] for t in traits]
    assert "rigorous" in labels
    assert "terse" in labels


def test_parse_alpha_rules_and_memory() -> None:
    defn = _loader().parse("alpha")
    assert "failing test" in defn.rules.body
    assert "primary language is Rust" in defn.memory.body
    assert "Name: Matt" in defn.user.body


def test_parse_alpha_skill() -> None:
    defn = _loader().parse("alpha")
    assert len(defn.skills) == 1
    sk = defn.skills[0]
    assert sk.id == "summarize"
    assert sk.name == "Summarize"
    assert sk.priority == 7
    assert sk.keywords == ["summarize", "tldr", "condense"]
    assert sk.tool_overlay == ["text.summarize"]
    assert sk.memory_namespace == ["alpha", "skill", "summarize"]
    assert "3-bullet TL;DR" in sk.instruction_fragment


def test_parse_alpha_hook() -> None:
    defn = _loader().parse("alpha")
    assert len(defn.hooks) == 1
    h = defn.hooks[0]
    assert h.event == "on_tool_call"
    assert h.match == {"tool": "shell.exec"}
    assert h.call == {"kind": "skill", "id": "redact_secrets"}
    assert h.when == "pre"
    assert h.budget == {"tokens": 2000, "ms": 5000}


def test_parse_unknown_agent_raises() -> None:
    with pytest.raises(AgentNotFoundError):
        _loader().parse("does_not_exist")


def test_parse_agent_missing_agent_yaml(tmp_path: Path) -> None:
    # Build a host root with one bare agent directory.
    root = tmp_path / "host"
    (root / "agents" / "broken").mkdir(parents=True)
    cfg = HostConfig.load(root)
    loader = AgentLoader(cfg)
    with pytest.raises(AgentSpecError):
        loader.parse("broken")


def test_parse_invalid_skill_keywords(tmp_path: Path) -> None:
    root = tmp_path / "host"
    agent = root / "agents" / "agent_with_bad_skill"
    skill_dir = agent / "skills" / "broken"
    skill_dir.mkdir(parents=True)
    (agent / "agent.yaml").write_text("id: agent_with_bad_skill\nmodel: gpt-4o\n", encoding="utf-8")
    (skill_dir / "SKILL.md").write_text(
        "---\nkeywords: not-a-list\n---\nbody\n",
        encoding="utf-8",
    )
    cfg = HostConfig.load(root)
    loader = AgentLoader(cfg)
    with pytest.raises(AgentSpecError):
        loader.parse("agent_with_bad_skill")


# ---------- materialize-phase tests (native-gated) -------------------------

try:
    from atomr_agents import _native as _native_pkg  # noqa: F401

    _native_available = True
except ImportError:
    _native_available = False

requires_native = pytest.mark.skipif(
    not _native_available, reason="atomr_agents._native not built (run maturin develop)"
)


@requires_native
def test_load_alpha_builds_native_agent_spec() -> None:
    loaded = _loader().load("alpha")
    assert loaded.spec.id == "alpha"
    assert loaded.spec.model == "gpt-4o"
    assert loaded.spec.max_iterations == 6
    assert loaded.spec.token_budget == 4096


@requires_native
def test_load_alpha_builds_skill_set() -> None:
    loaded = _loader().load("alpha")
    ss = loaded.skill_set
    # SkillSet supports len() per crates/py-bindings/src/skill.rs
    assert len(ss) == 1


@requires_native
def test_load_alpha_rules_split() -> None:
    loaded = _loader().load("alpha")
    assert len(loaded.rules) >= 3
    assert any("failing test" in r for r in loaded.rules)
    # Headings shouldn't leak in.
    assert not any(r.startswith("#") for r in loaded.rules)


@requires_native
def test_load_alpha_memory_facts() -> None:
    loaded = _loader().load("alpha")
    facts = loaded.memory_facts
    assert any("Rust" in f for f in facts)
    assert any("maturin" in f for f in facts)


@requires_native
def test_load_alpha_persona() -> None:
    loaded = _loader().load("alpha")
    # PersonaValue is a native struct; we don't assert its exact shape,
    # only that it was built rather than left None.
    assert loaded.persona is not None
