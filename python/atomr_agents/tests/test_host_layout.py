"""Tests for the agent_host layout module — pure-Python, no native required."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from atomr_agents.agent_host import AgentPaths, HostPaths, default_root
from atomr_agents.agent_host.layout import ENV_ROOT


def test_default_root_respects_env(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv(ENV_ROOT, str(tmp_path / "custom"))
    assert default_root() == (tmp_path / "custom").resolve()


def test_default_root_falls_back_to_home(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(ENV_ROOT, raising=False)
    expected = Path.home() / ".atomr" / "host"
    assert default_root() == expected


def test_host_paths_layout(tmp_path: Path) -> None:
    paths = HostPaths(root=tmp_path)
    assert paths.config_yaml == tmp_path / "config.yaml"
    assert paths.agents_md == tmp_path / "AGENTS.md"
    assert paths.agents_dir == tmp_path / "agents"
    assert paths.channels_dir == tmp_path / "channels"
    assert paths.crons_dir == tmp_path / "crons"
    assert paths.tools_dir == tmp_path / "tools"
    assert paths.registry_dir == tmp_path / "registry"
    assert paths.events_jsonl == tmp_path / "events.jsonl"


def test_host_paths_ensure_creates_skeleton(tmp_path: Path) -> None:
    paths = HostPaths(root=tmp_path / "host")
    paths.ensure()
    for d in (paths.root, paths.agents_dir, paths.channels_dir, paths.crons_dir):
        assert d.is_dir(), f"{d} should exist"
    # `ensure()` is idempotent.
    paths.ensure()
    assert paths.root.is_dir()


def test_agent_paths_layout(tmp_path: Path) -> None:
    ap = AgentPaths(root=tmp_path, agent_id="bob")
    assert ap.dir == tmp_path / "agents" / "bob"
    assert ap.agent_yaml == ap.dir / "agent.yaml"
    assert ap.soul_md == ap.dir / "SOUL.md"
    assert ap.rules_md == ap.dir / "RULES.md"
    assert ap.memory_md == ap.dir / "MEMORY.md"
    assert ap.user_md == ap.dir / "USER.md"
    assert ap.skills_dir == ap.dir / "skills"
    assert ap.hooks_dir == ap.dir / "hooks"
    assert ap.state_dir == ap.dir / "state"
    assert ap.threads_dir == ap.state_dir / "threads"
    assert ap.checkpoints_dir == ap.state_dir / "checkpoints"


def test_list_agent_ids_when_root_missing(tmp_path: Path) -> None:
    paths = HostPaths(root=tmp_path / "does_not_exist")
    assert paths.list_agent_ids() == []


def test_list_agent_ids_sorted_and_skips_dotfiles(tmp_path: Path) -> None:
    paths = HostPaths(root=tmp_path)
    paths.ensure()
    (paths.agents_dir / "zebra").mkdir()
    (paths.agents_dir / "alpha").mkdir()
    (paths.agents_dir / ".staging").mkdir()
    (paths.agents_dir / "not_a_dir").write_text("ignored", encoding="utf-8")

    assert paths.list_agent_ids() == ["alpha", "zebra"]
