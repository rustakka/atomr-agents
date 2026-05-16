"""Tests for the atomr-host CLI.

Exercises ``init``, ``agent new``, ``agent list``, ``agent show``,
``agent rm`` against a tmp-scoped host root. The CLI is invoked via
:py:func:`atomr_agents.agent_host.cli.main` so we can capture stdout
and stderr without spawning subprocesses.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from atomr_agents.agent_host import HostConfig
from atomr_agents.agent_host.cli import main

pytest.importorskip("yaml")


def _run(capsys: pytest.CaptureFixture[str], *argv: str) -> tuple[int, str, str]:
    code = main(list(argv))
    out, err = capsys.readouterr()
    return code, out, err


def test_init_scaffolds_root_and_default_agent(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, out, err = _run(capsys, "--root", str(tmp_path), "init")
    assert code == 0
    assert "initialized host root" in out
    assert "seeded agent" in out
    cfg = HostConfig.load(tmp_path)
    assert cfg.paths.config_yaml.is_file()
    assert cfg.paths.agents_md.is_file()
    assert (cfg.paths.agents_dir / "default" / "agent.yaml").is_file()
    assert (cfg.paths.agents_dir / "default" / "SOUL.md").is_file()


def test_init_no_default_agent(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code, out, err = _run(capsys, "--root", str(tmp_path), "init", "--no-default-agent")
    assert code == 0
    cfg = HostConfig.load(tmp_path)
    assert not (cfg.paths.agents_dir / "default").exists()


def test_init_is_idempotent(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    # Mutate a file; a non-forced second init must not overwrite it.
    seed_path = tmp_path / "agents" / "default" / "SOUL.md"
    seed_path.write_text("custom soul", encoding="utf-8")
    _run(capsys, "--root", str(tmp_path), "init")
    assert seed_path.read_text(encoding="utf-8") == "custom soul"


def test_init_force_overwrites(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    seed_path = tmp_path / "agents" / "default" / "SOUL.md"
    seed_path.write_text("custom soul", encoding="utf-8")
    _run(capsys, "--root", str(tmp_path), "init", "--force")
    assert seed_path.read_text(encoding="utf-8") != "custom soul"


def test_agent_new_then_list(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init", "--no-default-agent")
    code, out, _ = _run(capsys, "--root", str(tmp_path), "agent", "new", "scout")
    assert code == 0
    assert "scout" in out

    code, out, _ = _run(capsys, "--root", str(tmp_path), "agent", "list")
    assert code == 0
    assert out.strip().splitlines() == ["scout"]

    code, out, _ = _run(capsys, "--root", str(tmp_path), "agent", "list", "--format", "json")
    assert code == 0
    assert json.loads(out) == ["scout"]


def test_agent_show_pretty_and_json(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    code, out, _ = _run(capsys, "--root", str(tmp_path), "agent", "show", "default")
    assert code == 0
    assert "agent: default" in out
    assert "skillset:" in out

    code, out, _ = _run(
        capsys, "--root", str(tmp_path), "agent", "show", "default", "--format", "json"
    )
    assert code == 0
    data = json.loads(out)
    assert data["agent_id"] == "default"
    assert data["spec"]["max_iterations"] == 8


def test_agent_show_unknown_returns_error(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    _run(capsys, "--root", str(tmp_path), "init", "--no-default-agent")
    code, _out, err = _run(capsys, "--root", str(tmp_path), "agent", "show", "nope")
    assert code == 2
    assert "no agent" in err


def test_agent_rm_force(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    code, out, _ = _run(capsys, "--root", str(tmp_path), "agent", "rm", "default", "--force")
    assert code == 0
    assert "removed" in out
    assert not (tmp_path / "agents" / "default").exists()


def test_agent_rm_unknown_returns_error(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    _run(capsys, "--root", str(tmp_path), "init", "--no-default-agent")
    code, _out, err = _run(capsys, "--root", str(tmp_path), "agent", "rm", "nope", "--force")
    assert code == 2
    assert "no agent" in err
