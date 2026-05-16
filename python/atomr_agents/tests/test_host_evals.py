"""Tests for the M12 eval harness.

The eval surface is pure-Python — it uses the M2 deterministic
``render_chat_preview`` responder by default — so these tests do NOT
need ``atomr_agents._native``. They must pass under both Python 3.12
and 3.14 even when the native channel harness panics on construction.
"""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

pytest.importorskip("yaml")

from atomr_agents.agent_host import AgentLoader, HostConfig
from atomr_agents.agent_host.errors import AgentSpecError
from atomr_agents.agent_host.evals import (
    EvalCase,
    EvalCaseResult,
    EvalRun,
    EvalSuite,
    SCORERS,
    contains_scorer,
    evals_dir,
    excludes_scorer,
    list_suites,
    load_suite,
    regex_scorer,
    run_suite_sync,
    scaffold_suite,
)


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


# ---------- helpers ---------------------------------------------------------


def _copy_fixture_host(dst: Path) -> Path:
    """Copy the alpha fixture host into ``dst`` and return ``dst``."""
    shutil.copytree(FIXTURE_ROOT, dst)
    return dst


def _write_suite(root: Path, suite_id: str, body: str) -> Path:
    evals = root / "evals"
    evals.mkdir(parents=True, exist_ok=True)
    path = evals / f"{suite_id}.yaml"
    path.write_text(body, encoding="utf-8")
    return path


# ---------- scorer unit tests ----------------------------------------------


def test_contains_scorer_all_present() -> None:
    passed, score, _ = contains_scorer("alpha beta gamma", {"contains": ["alpha", "gamma"]})
    assert passed is True
    assert score == pytest.approx(1.0)


def test_contains_scorer_partial() -> None:
    passed, score, reason = contains_scorer(
        "alpha beta", {"contains": ["alpha", "beta", "gamma"]}
    )
    assert passed is False
    assert score == pytest.approx(2 / 3)
    assert "gamma" in reason


def test_contains_scorer_empty_list_is_error() -> None:
    passed, score, reason = contains_scorer("anything", {"contains": []})
    assert passed is False
    assert score == 0.0
    assert "non-empty" in reason


def test_regex_scorer_match() -> None:
    passed, score, _ = regex_scorer("[alpha] hello", {"regex": r"^\[\w+\]"})
    assert passed is True
    assert score == pytest.approx(1.0)


def test_regex_scorer_no_match() -> None:
    passed, score, reason = regex_scorer("nothing here", {"regex": r"^\d+$"})
    assert passed is False
    assert score == 0.0
    assert "no match" in reason


def test_excludes_scorer_absent() -> None:
    passed, score, _ = excludes_scorer("nice answer", {"excludes": ["error", "fail"]})
    assert passed is True
    assert score == pytest.approx(1.0)


def test_excludes_scorer_present() -> None:
    passed, score, reason = excludes_scorer(
        "this has an error in it", {"excludes": ["error", "fail"]}
    )
    assert passed is False
    assert score == pytest.approx(0.5)
    assert "error" in reason


def test_scorers_registry_contains_builtins() -> None:
    assert set(SCORERS) >= {"contains", "regex", "excludes"}


# ---------- load_suite / list_suites / scaffold ---------------------------


def test_load_suite_yaml_round_trip(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "smoke",
        """\
id: smoke
scorer: contains
description: identity check
cases:
  - id: identity
    input: hello
    expected:
      contains: ["alpha"]
  - id: rules
    input: anything
    expected:
      contains: ["rules:"]
""",
    )
    cfg = HostConfig.load(root)
    suite = load_suite(cfg, "smoke")
    assert isinstance(suite, EvalSuite)
    assert suite.id == "smoke"
    assert suite.scorer == "contains"
    assert suite.description == "identity check"
    assert [c.id for c in suite.cases] == ["identity", "rules"]
    assert suite.cases[0].expected == {"contains": ["alpha"]}
    assert suite.source_path is not None
    assert suite.source_path.name == "smoke.yaml"


def test_load_suite_missing_file_raises(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    cfg = HostConfig.load(root)
    with pytest.raises(AgentSpecError):
        load_suite(cfg, "does-not-exist")


def test_load_suite_bad_scorer_raises(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "bad",
        """\
id: bad
scorer: not-a-real-scorer
cases:
  - id: c1
    input: hi
    expected:
      contains: ["x"]
""",
    )
    cfg = HostConfig.load(root)
    with pytest.raises(AgentSpecError) as ei:
        load_suite(cfg, "bad")
    assert "scorer" in str(ei.value).lower()


def test_load_suite_missing_case_input_raises(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "broken",
        """\
id: broken
scorer: contains
cases:
  - id: c1
    expected:
      contains: ["x"]
""",
    )
    cfg = HostConfig.load(root)
    with pytest.raises(AgentSpecError):
        load_suite(cfg, "broken")


def test_scaffold_suite_writes_loadable(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    cfg = HostConfig.load(root)
    path = scaffold_suite(cfg, "first")
    assert path.is_file()
    assert path == evals_dir(cfg) / "first.yaml"
    # Re-running without force is idempotent — content stays put.
    original = path.read_text(encoding="utf-8")
    path2 = scaffold_suite(cfg, "first")
    assert path2 == path
    assert path.read_text(encoding="utf-8") == original
    # And the file round-trips through load_suite.
    suite = load_suite(cfg, "first")
    assert suite.id == "first"
    assert suite.scorer == "contains"
    assert suite.cases


def test_scaffold_suite_force_overwrites(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    cfg = HostConfig.load(root)
    path = scaffold_suite(cfg, "x")
    path.write_text("# stale\n", encoding="utf-8")
    scaffold_suite(cfg, "x", force=True)
    assert "scorer: contains" in path.read_text(encoding="utf-8")


def test_list_suites_sorted(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    cfg = HostConfig.load(root)
    assert list_suites(cfg) == []
    _write_suite(root, "zzz", "id: zzz\nscorer: contains\ncases:\n  - id: a\n    input: x\n    expected:\n      contains: ['x']\n")
    _write_suite(root, "aaa", "id: aaa\nscorer: contains\ncases:\n  - id: a\n    input: x\n    expected:\n      contains: ['x']\n")
    _write_suite(root, "mmm", "id: mmm\nscorer: contains\ncases:\n  - id: a\n    input: x\n    expected:\n      contains: ['x']\n")
    assert list_suites(cfg) == ["aaa", "mmm", "zzz"]


# ---------- end-to-end run_suite ------------------------------------------


def _native_loader_works() -> bool:
    """Check that the loader can build a LoadedAgent (requires _native)."""
    try:
        cfg = HostConfig.load(FIXTURE_ROOT)
        AgentLoader(cfg).load("alpha")
        return True
    except Exception:
        return False


_loader_ok = _native_loader_works()


requires_loader = pytest.mark.skipif(
    not _loader_ok,
    reason="atomr_agents._native not built — AgentLoader.load() unavailable",
)


@requires_loader
def test_run_suite_against_alpha_fixture(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "smoke",
        """\
id: smoke
scorer: contains
description: M2 responder surfaces identity + summary line
cases:
  - id: identity
    input: hello
    expected:
      contains: ["alpha", "rules:"]
""",
    )
    cfg = HostConfig.load(root)
    suite = load_suite(cfg, "smoke")
    run = run_suite_sync(cfg, "alpha", suite)
    assert isinstance(run, EvalRun)
    assert run.suite_id == "smoke"
    assert run.agent_id == "alpha"
    assert run.passed >= 1
    assert run.pass_rate == pytest.approx(1.0)
    for r in run.results:
        assert isinstance(r, EvalCaseResult)
        assert r.output  # responder produced something


@requires_loader
def test_run_suite_with_custom_responder(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "custom",
        """\
id: custom
scorer: contains
cases:
  - id: c1
    input: ignored
    expected:
      contains: ["FIXED_OUTPUT"]
""",
    )
    cfg = HostConfig.load(root)
    suite = load_suite(cfg, "custom")

    def fixed(_loaded, _text):
        return "this is FIXED_OUTPUT from a custom responder"

    run = run_suite_sync(cfg, "alpha", suite, responder=fixed)
    assert run.passed == 1
    assert run.failed == 0
    assert "FIXED_OUTPUT" in run.results[0].output
    assert run.results[0].score == pytest.approx(1.0)


@requires_loader
def test_run_suite_records_failure_for_responder_exception(tmp_path: Path) -> None:
    root = _copy_fixture_host(tmp_path / "host")
    _write_suite(
        root,
        "boom",
        """\
id: boom
scorer: contains
cases:
  - id: c1
    input: anything
    expected:
      contains: ["something"]
""",
    )
    cfg = HostConfig.load(root)
    suite = load_suite(cfg, "boom")

    def explode(_loaded, _text):
        raise RuntimeError("kaboom")

    run = run_suite_sync(cfg, "alpha", suite, responder=explode)
    assert run.passed == 0
    assert run.failed == 1
    assert run.pass_rate == 0.0
    assert "kaboom" in run.results[0].reason


# ---------- EvalRun convenience properties --------------------------------


def test_eval_run_pass_rate_handles_empty() -> None:
    run = EvalRun(suite_id="empty", agent_id="alpha", results=[])
    assert run.passed == 0
    assert run.failed == 0
    assert run.pass_rate == 0.0


def test_eval_run_mixed_results() -> None:
    run = EvalRun(
        suite_id="mixed",
        agent_id="alpha",
        results=[
            EvalCaseResult(case_id="a", passed=True, score=1.0, output="x"),
            EvalCaseResult(case_id="b", passed=False, score=0.5, output="y"),
            EvalCaseResult(case_id="c", passed=True, score=1.0, output="z"),
        ],
    )
    assert run.passed == 2
    assert run.failed == 1
    assert run.pass_rate == pytest.approx(2 / 3)


def test_evalcase_dataclass_roundtrip() -> None:
    case = EvalCase(id="a", input="hi", expected={"contains": ["hi"]})
    assert case.metadata == {}
    case2 = EvalCase(
        id="b", input="bye", expected={"regex": "bye"}, metadata={"tag": "smoke"}
    )
    assert case2.metadata == {"tag": "smoke"}
