"""Eval harness wiring — M12.

Lets ``atomr-host eval <suite_id>`` exercise an assembled host agent
against a small on-disk YAML/JSON suite. The "agent under test" is the
M2 deterministic chat responder
(:func:`atomr_agents.agent_host.chat.render_chat_preview`), which is
pure-Python and does not require the native PyO3 extension — so suites
can be run wherever the loader can parse + materialize an agent (i.e.
anywhere ``maturin develop`` has built ``_native``).

A custom ``responder`` callable can be plugged into :func:`run_suite`
so that — once the real-LLM responder lands post-M9 — the swap is
local.

The on-disk layout lives at ``<root>/evals/<suite_id>.{yaml,json}``.
Each suite is a small mapping of the shape::

    id: smoke
    scorer: contains
    description: smoke check that the agent surfaces its identity
    cases:
      - id: identity
        input: hello
        expected:
          contains: ["alpha"]

The scorer name must match a key in :data:`SCORERS`.
"""

from __future__ import annotations

import asyncio
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from .chat import render_chat_preview
from .config import HostConfig
from .errors import AgentSpecError
from .loader import AgentLoader, LoadedAgent

try:  # PyYAML is optional but expected for the host
    import yaml  # type: ignore[import-untyped]

    _yaml_available = True
except ImportError:  # pragma: no cover
    yaml = None  # type: ignore[assignment]
    _yaml_available = False

__all__ = [
    "EvalCase",
    "EvalCaseResult",
    "EvalRun",
    "EvalSuite",
    "SCORERS",
    "Scorer",
    "contains_scorer",
    "evals_dir",
    "excludes_scorer",
    "list_suites",
    "load_suite",
    "regex_scorer",
    "run_suite",
    "run_suite_sync",
    "scaffold_suite",
]


# ---------- dataclasses ------------------------------------------------------


@dataclass(frozen=True)
class EvalCase:
    """One case in an :class:`EvalSuite`.

    ``input`` becomes the user message text fed through the responder;
    ``expected`` is the scorer-specific criteria dict.
    """

    id: str
    input: str
    expected: dict
    metadata: dict = field(default_factory=dict)


@dataclass(frozen=True)
class EvalSuite:
    """A loaded eval suite ready for :func:`run_suite`."""

    id: str
    scorer: str
    cases: list[EvalCase]
    description: str = ""
    source_path: Path | None = None


@dataclass(frozen=True)
class EvalCaseResult:
    """Outcome of running one :class:`EvalCase`."""

    case_id: str
    passed: bool
    score: float
    output: str
    reason: str = ""


@dataclass(frozen=True)
class EvalRun:
    """Aggregate result of running an :class:`EvalSuite`."""

    suite_id: str
    agent_id: str
    results: list[EvalCaseResult]

    @property
    def passed(self) -> int:
        return sum(1 for r in self.results if r.passed)

    @property
    def failed(self) -> int:
        return sum(1 for r in self.results if not r.passed)

    @property
    def pass_rate(self) -> float:
        if not self.results:
            return 0.0
        return self.passed / len(self.results)


# A scorer takes the responder's output text and the case's `expected`
# dict, and returns ``(passed, score, reason)``. ``score`` lives in
# ``0.0..1.0``; binary scorers report ``1.0`` on pass and ``0.0`` on
# fail.
Scorer = Callable[[str, dict], "tuple[bool, float, str]"]


# ---------- built-in scorers -------------------------------------------------


def contains_scorer(output: str, expected: dict) -> tuple[bool, float, str]:
    """All listed substrings must appear in ``output``.

    ``expected`` shape::

        {"contains": ["substr1", "substr2"]}

    Score is ``matched / len(expected["contains"])`` — i.e. partial
    credit for partial matches. ``passed`` is True iff every substring
    is present.
    """
    substrings = expected.get("contains") if isinstance(expected, dict) else None
    if not isinstance(substrings, list) or not substrings:
        return False, 0.0, "contains_scorer: expected['contains'] must be a non-empty list"
    if not all(isinstance(s, str) for s in substrings):
        return False, 0.0, "contains_scorer: every substring must be a string"
    matched = [s for s in substrings if s in output]
    missing = [s for s in substrings if s not in output]
    total = len(substrings)
    score = len(matched) / total
    if not missing:
        return True, score, f"matched {len(matched)}/{total} substring(s)"
    return (
        False,
        score,
        f"missing {len(missing)}/{total} substring(s): {missing}",
    )


def regex_scorer(output: str, expected: dict) -> tuple[bool, float, str]:
    """Output must match ``expected['regex']`` via :func:`re.search`."""
    pattern = expected.get("regex") if isinstance(expected, dict) else None
    if not isinstance(pattern, str) or not pattern:
        return False, 0.0, "regex_scorer: expected['regex'] must be a non-empty string"
    try:
        compiled = re.compile(pattern)
    except re.error as exc:
        return False, 0.0, f"regex_scorer: invalid pattern: {exc}"
    if compiled.search(output):
        return True, 1.0, f"matched /{pattern}/"
    return False, 0.0, f"no match for /{pattern}/"


def excludes_scorer(output: str, expected: dict) -> tuple[bool, float, str]:
    """No listed substring may appear in ``output``.

    ``expected`` shape::

        {"excludes": ["substr1", "substr2"]}
    """
    substrings = expected.get("excludes") if isinstance(expected, dict) else None
    if not isinstance(substrings, list) or not substrings:
        return False, 0.0, "excludes_scorer: expected['excludes'] must be a non-empty list"
    if not all(isinstance(s, str) for s in substrings):
        return False, 0.0, "excludes_scorer: every substring must be a string"
    present = [s for s in substrings if s in output]
    total = len(substrings)
    if not present:
        return True, 1.0, f"none of {total} forbidden substring(s) present"
    score = 1.0 - (len(present) / total)
    return (
        False,
        score,
        f"forbidden substring(s) present: {present}",
    )


SCORERS: dict[str, Scorer] = {
    "contains": contains_scorer,
    "regex": regex_scorer,
    "excludes": excludes_scorer,
}


# ---------- on-disk loader ---------------------------------------------------


def evals_dir(config: HostConfig) -> Path:
    """Return ``<root>/evals/``.

    The directory is not created here; :func:`scaffold_suite` creates
    it on demand.
    """
    return config.paths.root / "evals"


_VALID_SUFFIXES: tuple[str, ...] = (".yaml", ".yml", ".json")


def _find_suite_file(root: Path, suite_id: str) -> Path | None:
    for suffix in _VALID_SUFFIXES:
        candidate = root / f"{suite_id}{suffix}"
        if candidate.is_file():
            return candidate
    return None


def _parse_suite_file(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    suffix = path.suffix.lower()
    if suffix == ".json":
        try:
            raw = json.loads(text or "{}")
        except json.JSONDecodeError as exc:
            raise AgentSpecError(f"invalid JSON in {path}: {exc}") from exc
    else:
        if not _yaml_available:
            raise AgentSpecError(
                f"PyYAML is required to read {path} — install atomr-agents[host]"
            )
        try:
            raw = yaml.safe_load(text) or {}
        except yaml.YAMLError as exc:  # type: ignore[union-attr]
            raise AgentSpecError(f"invalid YAML in {path}: {exc}") from exc
    if not isinstance(raw, dict):
        raise AgentSpecError(f"{path}: top-level must be a mapping")
    return raw


def load_suite(config: HostConfig, suite_id: str) -> EvalSuite:
    """Read ``<root>/evals/<suite_id>.{yaml,yml,json}`` into an EvalSuite.

    Validates the scorer name against :data:`SCORERS` and ensures each
    case has an ``id``, ``input``, and ``expected`` mapping.

    Raises :class:`AgentSpecError` on missing files or malformed input.
    """
    if not suite_id or "/" in suite_id or "\\" in suite_id:
        raise AgentSpecError(f"invalid suite id: {suite_id!r}")
    root = evals_dir(config)
    path = _find_suite_file(root, suite_id)
    if path is None:
        raise AgentSpecError(
            f"no eval suite at {root}/{suite_id}.(yaml|yml|json)"
        )
    raw = _parse_suite_file(path)

    file_id = raw.get("id", suite_id)
    if not isinstance(file_id, str) or not file_id:
        raise AgentSpecError(f"{path}: `id` must be a non-empty string")

    scorer_name = raw.get("scorer")
    if not isinstance(scorer_name, str) or scorer_name not in SCORERS:
        valid = sorted(SCORERS.keys())
        raise AgentSpecError(
            f"{path}: `scorer` must be one of {valid}, got {scorer_name!r}"
        )

    description = raw.get("description", "")
    if not isinstance(description, str):
        raise AgentSpecError(f"{path}: `description` must be a string")

    cases_raw = raw.get("cases")
    if not isinstance(cases_raw, list) or not cases_raw:
        raise AgentSpecError(f"{path}: `cases` must be a non-empty list")

    cases: list[EvalCase] = []
    for idx, entry in enumerate(cases_raw):
        if not isinstance(entry, dict):
            raise AgentSpecError(f"{path}: case #{idx} must be a mapping")
        case_id = entry.get("id")
        if not isinstance(case_id, str) or not case_id:
            raise AgentSpecError(
                f"{path}: case #{idx} is missing a non-empty `id`"
            )
        case_input = entry.get("input")
        if not isinstance(case_input, str):
            raise AgentSpecError(
                f"{path}: case {case_id!r} is missing a string `input`"
            )
        expected = entry.get("expected")
        if not isinstance(expected, dict):
            raise AgentSpecError(
                f"{path}: case {case_id!r} is missing an `expected` mapping"
            )
        metadata = entry.get("metadata") or {}
        if not isinstance(metadata, dict):
            raise AgentSpecError(
                f"{path}: case {case_id!r} `metadata` must be a mapping"
            )
        cases.append(
            EvalCase(
                id=case_id,
                input=case_input,
                expected=dict(expected),
                metadata=dict(metadata),
            )
        )

    return EvalSuite(
        id=file_id,
        scorer=scorer_name,
        cases=cases,
        description=description,
        source_path=path,
    )


def list_suites(config: HostConfig) -> list[str]:
    """Return sorted suite ids found under ``<root>/evals/``.

    A suite id is the stem of any YAML or JSON file in that directory.
    Returns an empty list if the directory does not exist.
    """
    root = evals_dir(config)
    if not root.is_dir():
        return []
    ids: set[str] = set()
    for child in root.iterdir():
        if not child.is_file():
            continue
        if child.suffix.lower() not in _VALID_SUFFIXES:
            continue
        ids.add(child.stem)
    return sorted(ids)


_DEFAULT_SUITE_BODY = """\
# Default eval suite generated by `atomr-host eval init`.
#
# Each case sends `input` through the agent's deterministic responder
# and checks the output against the scorer-specific `expected` block.
id: {suite_id}
scorer: contains
description: Smoke check that the agent surfaces its identity.
cases:
  - id: identity
    input: hello
    expected:
      contains:
        - "{suite_id}"
"""


def scaffold_suite(
    config: HostConfig,
    suite_id: str,
    *,
    force: bool = False,
) -> Path:
    """Write a minimal default suite under ``<root>/evals/<suite_id>.yaml``.

    Idempotent unless ``force`` — an existing file is left alone.
    Returns the path that was (or would have been) written.
    """
    if not suite_id or "/" in suite_id or "\\" in suite_id:
        raise AgentSpecError(f"invalid suite id: {suite_id!r}")
    root = evals_dir(config)
    root.mkdir(parents=True, exist_ok=True)
    path = root / f"{suite_id}.yaml"
    if path.exists() and not force:
        return path
    path.write_text(
        _DEFAULT_SUITE_BODY.format(suite_id=suite_id), encoding="utf-8"
    )
    return path


# ---------- runner -----------------------------------------------------------


def _default_responder(loaded: LoadedAgent, text: str) -> str:
    """Default responder used when none is supplied.

    Delegates to :func:`render_chat_preview` — pure-Python, no native
    extension required.
    """
    return render_chat_preview(loaded, text)


async def run_suite(
    config: HostConfig,
    agent_id: str,
    suite: EvalSuite,
    *,
    responder: Callable[[LoadedAgent, str], str] | None = None,
) -> EvalRun:
    """Run ``suite`` against ``agent_id``, returning an :class:`EvalRun`.

    The "agent under test" is the M2 deterministic responder
    (:func:`render_chat_preview`) unless ``responder`` is provided.
    Each case's ``input`` is passed to the responder; the returned
    string is then scored by ``SCORERS[suite.scorer]``.

    Failures during the responder become
    :class:`EvalCaseResult` ``(passed=False, score=0.0, reason="error: ...")``.
    """
    scorer = SCORERS.get(suite.scorer)
    if scorer is None:
        raise AgentSpecError(
            f"unknown scorer {suite.scorer!r}; valid: {sorted(SCORERS)}"
        )
    loaded = AgentLoader(config).load(agent_id)
    actual_responder = responder if responder is not None else _default_responder

    results: list[EvalCaseResult] = []
    for case in suite.cases:
        try:
            output = actual_responder(loaded, case.input)
        except BaseException as exc:  # noqa: BLE001 — surface every failure
            results.append(
                EvalCaseResult(
                    case_id=case.id,
                    passed=False,
                    score=0.0,
                    output="",
                    reason=f"error: {exc!r}",
                )
            )
            continue
        if not isinstance(output, str):
            output = str(output)
        try:
            passed, score, reason = scorer(output, case.expected)
        except BaseException as exc:  # noqa: BLE001 — scorer bugs surface
            results.append(
                EvalCaseResult(
                    case_id=case.id,
                    passed=False,
                    score=0.0,
                    output=output,
                    reason=f"scorer error: {exc!r}",
                )
            )
            continue
        results.append(
            EvalCaseResult(
                case_id=case.id,
                passed=bool(passed),
                score=float(score),
                output=output,
                reason=reason,
            )
        )

    return EvalRun(suite_id=suite.id, agent_id=agent_id, results=results)


def run_suite_sync(
    config: HostConfig,
    agent_id: str,
    suite: EvalSuite,
    *,
    responder: Callable[[LoadedAgent, str], str] | None = None,
) -> EvalRun:
    """Synchronous wrapper around :func:`run_suite`.

    Convenient for CLI entry points that don't already have an event
    loop.
    """
    return asyncio.run(
        run_suite(config, agent_id, suite, responder=responder)
    )
