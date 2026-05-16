"""Skill selection / validation / scaffold — thin facade over ``_native.host``.

Native counterparts (``_h.select_skills_for``, ``_h.validate_skills``,
``_h.scaffold_skill``) live in ``crates/host``. They use Rust-native
``SkillDefinition`` / ``AgentPaths`` types, so this module:

* unwraps Python ``AgentPaths`` to the native handle when calling native
  helpers,
* lifts the small extra validation rules (priority range, keyword type
  and emptiness checks) that the chat-preview / CLI surface expects,
* keeps the Python ``select_skills_for`` shape so callers can pass the
  Python ``SkillDefinition`` dataclass produced by ``AgentLoader.parse``
  (native ``SkillDefinition`` has no Python constructor).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from atomr_agents._native import host as _h

from .errors import AgentSpecError
from .layout import AgentPaths
from .loader import SkillDefinition

try:  # native is optional for build_keyword_skill_strategy
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover - native extension missing
    _native = None


__all__ = [
    "SkillValidationReport",
    "build_keyword_skill_strategy",
    "report_to_dict",
    "scaffold_skill",
    "select_skills_for",
    "validate_skills",
]


# ---------- selection -------------------------------------------------------


def select_skills_for(
    skills: list[SkillDefinition],
    user_message: str,
) -> list[SkillDefinition]:
    """Return skills whose ``keywords`` appear (case-insensitive) in ``user_message``.

    Mirrors the semantics of the native keyword strategy in
    ``crates/host``: substring match, case-insensitive, sorted by
    ``(-priority, id)``. The native ``select_skills_for`` requires
    native ``SkillDefinition`` objects (which lack a Python
    constructor), so callers passing the Python dataclass go through
    this in-process implementation.
    """
    if not user_message:
        return []
    needle = user_message.lower()
    matched: list[SkillDefinition] = []
    for sd in skills:
        for kw in sd.keywords:
            if kw and kw.lower() in needle:
                matched.append(sd)
                break
    matched.sort(key=lambda s: (-s.priority, s.id))
    return matched


def build_keyword_skill_strategy(loaded: Any) -> Any:
    """Wrap a :class:`LoadedAgent` into a native ``KeywordSkillStrategy``."""
    if _native is None:
        raise AgentSpecError(
            "atomr_agents._native is not built — run `maturin develop` "
            "before building a keyword skill strategy"
        )
    defn = getattr(loaded, "definition", None)
    if defn is None or not getattr(defn, "skills", None):
        raise AgentSpecError("loaded agent has no skill definitions")
    native_skills = [
        _native.skill.Skill(
            id=sd.id,
            name=sd.name,
            instruction_fragment=sd.instruction_fragment,
            priority=sd.priority,
            keywords=list(sd.keywords) or None,
        )
        for sd in defn.skills
    ]
    keyword_map: dict[str, list[str]] = {
        sd.name: list(sd.keywords) for sd in defn.skills if sd.keywords
    }
    return _native.skill.keyword_skill_strategy(native_skills, keyword_map)


# ---------- validation ------------------------------------------------------


@dataclass(frozen=True)
class SkillValidationReport:
    """Per-skill validation outcome.

    Mirrors ``_native.host.SkillValidationReport`` field-for-field. The
    native type has no Python constructor, so the facade builds reports
    via this dataclass; callers reading ``skill_id`` / ``path`` /
    ``errors`` / ``warnings`` / ``ok`` work transparently against both.
    """

    skill_id: str
    path: Path
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors


def _agent_paths_inner(paths: AgentPaths | Any) -> Any:
    """Return the native ``AgentPaths`` handle wrapped by ``paths``."""
    inner = getattr(paths, "_inner", None)
    return inner if inner is not None else paths


def _python_validate_skill_md(skill_dir: Path) -> tuple[list[str], list[str]]:
    """Apply the Python-level frontmatter checks (priority/keywords/body)."""
    from .markdown import read_markdown

    md = skill_dir / "SKILL.md"
    errors: list[str] = []
    warnings: list[str] = []
    try:
        doc = read_markdown(md)
    except Exception as exc:  # noqa: BLE001
        return [f"failed to parse: {exc}"], warnings

    fm = doc.frontmatter or {}
    if "keywords" in fm and not isinstance(fm["keywords"], list):
        errors.append("`keywords` must be a list of strings")
    if "tool_overlay" in fm and not isinstance(fm["tool_overlay"], list):
        errors.append("`tool_overlay` must be a list of strings")
    if "memory_namespace" in fm and not isinstance(fm["memory_namespace"], list):
        errors.append("`memory_namespace` must be a list of strings")
    if "priority" in fm:
        try:
            pri = int(fm["priority"])
            if pri < 0 or pri > 10:
                warnings.append(f"`priority` outside 0..10 (got {pri})")
        except (TypeError, ValueError):
            errors.append("`priority` must be an integer")

    if not doc.body.strip():
        warnings.append("body is empty — `instruction_fragment` will be blank")

    kw = fm.get("keywords") or []
    if isinstance(kw, list) and not any(isinstance(k, str) and k.strip() for k in kw):
        warnings.append("no keywords — skill will never trigger via keyword strategy")

    return errors, warnings


def validate_skills(agent_paths: AgentPaths) -> list[SkillValidationReport]:
    """Validate every ``skills/<id>/SKILL.md`` under an agent directory.

    Delegates the "is the file there?" check to ``_h.validate_skills``
    and layers the Python frontmatter checks on top. Returns one
    :class:`SkillValidationReport` per skill directory, in sorted id
    order. An agent with no ``skills/`` directory yields ``[]``.
    """
    skills_dir = agent_paths.skills_dir
    if not skills_dir.is_dir():
        return []

    native_reports = _h.validate_skills(_agent_paths_inner(agent_paths))
    by_id: dict[str, Any] = {r.skill_id: r for r in native_reports}

    out: list[SkillValidationReport] = []
    for child in sorted(skills_dir.iterdir()):
        if not child.is_dir() or child.name.startswith("."):
            continue
        md = child / "SKILL.md"
        native = by_id.get(child.name)

        if not md.is_file():
            # Normalize native's "missing SKILL.md at <path>" to the
            # message the CLI/tests expect.
            out.append(
                SkillValidationReport(
                    skill_id=child.name,
                    path=md,
                    errors=["SKILL.md missing"],
                )
            )
            continue

        errors, warnings = _python_validate_skill_md(child)
        if native is not None:
            errors = list(native.errors) + errors
            warnings = list(native.warnings) + warnings

        out.append(
            SkillValidationReport(
                skill_id=child.name,
                path=md,
                errors=errors,
                warnings=warnings,
            )
        )
    return out


# ---------- scaffold helpers -----------------------------------------------


SKILL_TEMPLATE = """---
name: {name}
priority: {priority}
keywords:
{keyword_lines}
tool_overlay: []
memory_namespace: []
---

# {name}

Replace this paragraph with the skill body. It becomes the skill's
`instruction_fragment` and is concatenated into the system prompt
when the skill is selected by the keyword strategy.
"""


def scaffold_skill(
    agent_paths: AgentPaths,
    skill_id: str,
    *,
    name: str | None = None,
    priority: int = 5,
    keywords: list[str] | None = None,
    force: bool = False,
) -> Path:
    """Create ``agents/<id>/skills/<skill_id>/SKILL.md`` with a default body.

    Returns the path written. Idempotent unless ``force``.

    Native ``_h.scaffold_skill`` always (re)writes the file and uses a
    short template; we keep the richer Python template (matches what
    the CLI emits) and add the idempotent / ``force`` semantics on
    top.
    """
    skill_dir = agent_paths.skills_dir / skill_id
    skill_dir.mkdir(parents=True, exist_ok=True)
    target = skill_dir / "SKILL.md"
    if target.is_file() and not force:
        return target
    keyword_lines = "\n".join(f"  - {k}" for k in (keywords or [skill_id])) or "  - "
    target.write_text(
        SKILL_TEMPLATE.format(
            name=name or skill_id.replace("_", " ").title(),
            priority=priority,
            keyword_lines=keyword_lines,
        ),
        encoding="utf-8",
    )
    return target


def report_to_dict(report: SkillValidationReport) -> dict[str, Any]:
    """JSON-friendly representation for CLI ``--format=json`` output."""
    return {
        "skill_id": report.skill_id,
        "path": str(report.path),
        "ok": report.ok,
        "errors": list(report.errors),
        "warnings": list(report.warnings),
    }
