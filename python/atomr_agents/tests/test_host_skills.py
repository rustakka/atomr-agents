"""Tests for the M4 skill selection / validation / scaffold layer."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host import (
    AgentLoader,
    HostConfig,
    SkillValidationReport,
    scaffold_skill,
    select_skills_for,
    validate_skills,
)
from atomr_agents.agent_host.cli import main
from atomr_agents.agent_host.loader import SkillDefinition


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


def _loader() -> AgentLoader:
    return AgentLoader(HostConfig.load(FIXTURE_ROOT))


# ---------- select_skills_for -----------------------------------------------


def _make(*, id: str, keywords: list[str], priority: int = 5) -> SkillDefinition:
    return SkillDefinition(
        id=id,
        name=id.title(),
        instruction_fragment=f"fragment for {id}",
        priority=priority,
        keywords=keywords,
    )


def test_select_matches_substring_case_insensitive() -> None:
    skills = [_make(id="summarize", keywords=["summarize", "tldr"])]
    assert select_skills_for(skills, "Please give me a TLDR") == skills
    assert select_skills_for(skills, "no match here") == []


def test_select_returns_empty_when_message_empty() -> None:
    skills = [_make(id="x", keywords=["x"])]
    assert select_skills_for(skills, "") == []


def test_select_skips_skills_with_no_keywords() -> None:
    skills = [_make(id="passive", keywords=[]), _make(id="active", keywords=["go"])]
    assert select_skills_for(skills, "let's go") == [skills[1]]


def test_select_orders_by_priority_descending_then_id() -> None:
    a = _make(id="a", keywords=["x"], priority=3)
    b = _make(id="b", keywords=["x"], priority=9)
    c = _make(id="c", keywords=["x"], priority=9)
    out = select_skills_for([a, b, c], "x")
    assert [s.id for s in out] == ["b", "c", "a"]


def test_select_fixture_alpha_summarize() -> None:
    defn = _loader().parse("alpha")
    selected = select_skills_for(defn.skills, "please summarize this")
    assert len(selected) == 1
    assert selected[0].id == "summarize"
    selected = select_skills_for(defn.skills, "unrelated message")
    assert selected == []


# ---------- validate_skills -------------------------------------------------


def test_validate_alpha_fixture_passes() -> None:
    paths = HostConfig.load(FIXTURE_ROOT).paths.agent("alpha")
    reports = validate_skills(paths)
    assert len(reports) == 1
    assert reports[0].skill_id == "summarize"
    assert reports[0].ok
    assert reports[0].errors == []


def test_validate_missing_skill_md(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("orphan")
    paths.ensure()
    (paths.skills_dir / "no_body").mkdir()
    reports = validate_skills(paths)
    assert len(reports) == 1
    assert not reports[0].ok
    assert "SKILL.md missing" in reports[0].errors[0]


def test_validate_bad_keywords_type(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    (paths.skills_dir / "broken").mkdir()
    (paths.skills_dir / "broken" / "SKILL.md").write_text(
        "---\nkeywords: not-a-list\n---\nbody\n", encoding="utf-8"
    )
    reports = validate_skills(paths)
    assert reports[0].errors
    assert "keywords" in reports[0].errors[0]


def test_validate_warns_on_empty_keywords(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    (paths.skills_dir / "empty_kw").mkdir()
    (paths.skills_dir / "empty_kw" / "SKILL.md").write_text(
        "---\nname: empty_kw\npriority: 5\nkeywords: []\n---\nbody\n", encoding="utf-8"
    )
    reports = validate_skills(paths)
    assert reports[0].ok
    assert any("no keywords" in w for w in reports[0].warnings)


def test_validate_warns_on_priority_out_of_range(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    (paths.skills_dir / "hi_pri").mkdir()
    (paths.skills_dir / "hi_pri" / "SKILL.md").write_text(
        "---\npriority: 42\nkeywords: [hi]\n---\nbody\n", encoding="utf-8"
    )
    reports = validate_skills(paths)
    assert reports[0].ok
    assert any("priority" in w for w in reports[0].warnings)


# ---------- scaffold --------------------------------------------------------


def test_scaffold_skill_creates_file(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    target = scaffold_skill(paths, "fizz", name="Fizz", priority=7, keywords=["fizz", "buzz"])
    assert target.is_file()
    content = target.read_text(encoding="utf-8")
    assert "name: Fizz" in content
    assert "priority: 7" in content
    assert "- fizz" in content
    assert "- buzz" in content


def test_scaffold_skill_idempotent(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    target = scaffold_skill(paths, "fizz")
    first = target.read_text(encoding="utf-8")
    target2 = scaffold_skill(paths, "fizz")
    second = target2.read_text(encoding="utf-8")
    assert first == second


def test_scaffold_skill_force_overwrites(tmp_path: Path) -> None:
    paths = HostConfig.load(tmp_path).paths.agent("a")
    paths.ensure()
    target = scaffold_skill(paths, "fizz")
    target.write_text("custom body", encoding="utf-8")
    scaffold_skill(paths, "fizz", force=True, name="Renamed")
    new_content = target.read_text(encoding="utf-8")
    assert "name: Renamed" in new_content


# ---------- CLI ------------------------------------------------------------


def _run(capsys: pytest.CaptureFixture[str], *argv: str) -> tuple[int, str, str]:
    code = main(list(argv))
    out, err = capsys.readouterr()
    return code, out, err


def test_cli_skill_new_then_ls(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    code, out, _ = _run(
        capsys, "--root", str(tmp_path), "skill", "new", "default", "summarize",
        "--keywords", "summarize,tldr", "--priority", "8",
    )
    assert code == 0
    assert "SKILL.md" in out

    code, out, _ = _run(capsys, "--root", str(tmp_path), "skill", "ls", "default")
    assert code == 0
    assert "summarize" in out
    assert "priority=8" in out

    code, out, _ = _run(
        capsys, "--root", str(tmp_path), "skill", "ls", "default", "--format", "json"
    )
    data = json.loads(out)
    assert data[0]["id"] == "summarize"
    assert data[0]["priority"] == 8
    assert data[0]["keywords"] == ["summarize", "tldr"]


def test_cli_skill_validate_ok(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    _run(capsys, "--root", str(tmp_path), "skill", "new", "default", "summarize",
         "--keywords", "go", "--priority", "5")
    code, out, _ = _run(capsys, "--root", str(tmp_path), "skill", "validate", "default")
    assert code == 0
    assert "[OK]" in out


def test_cli_skill_validate_reports_failure(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    _run(capsys, "--root", str(tmp_path), "init")
    skill_dir = tmp_path / "agents" / "default" / "skills" / "broken"
    skill_dir.mkdir(parents=True)
    (skill_dir / "SKILL.md").write_text(
        "---\nkeywords: bad\n---\nbody\n", encoding="utf-8"
    )
    code, out, err = _run(capsys, "--root", str(tmp_path), "skill", "validate", "default")
    assert code == 1
    assert "[FAIL]" in out
    assert "failed validation" in err


def test_cli_skill_new_unknown_agent(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    _run(capsys, "--root", str(tmp_path), "init", "--no-default-agent")
    code, _out, err = _run(capsys, "--root", str(tmp_path), "skill", "new", "nope", "x")
    assert code == 2
    assert "no agent" in err


# ---------- build_keyword_skill_strategy ----------------------------------


def _native_ok() -> bool:
    try:
        from atomr_agents import _native
        _native.skill.keyword_skill_strategy([], {})
        return True
    except BaseException:
        return False


requires_native = pytest.mark.skipif(not _native_ok(), reason="native skill module unavailable")


@requires_native
def test_build_keyword_skill_strategy_for_alpha() -> None:
    from atomr_agents.agent_host import build_keyword_skill_strategy

    loaded = _loader().load("alpha")
    strategy = build_keyword_skill_strategy(loaded)
    # The native handle is opaque; we just confirm a non-None object came back.
    assert strategy is not None
