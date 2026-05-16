"""AgentLoader — thin facade over ``_native.host.AgentLoader``."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from atomr_agents._native import host as _host

from .config import HostConfig
from .errors import AgentNotFoundError, AgentSpecError
from .layout import AgentPaths
from .markdown import MarkdownDoc

try:
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover
    _native = None

__all__ = [
    "AgentDefinition",
    "AgentLoader",
    "HookDefinition",
    "LoadedAgent",
    "SkillDefinition",
]


@dataclass(frozen=True)
class SkillDefinition:
    id: str
    name: str
    instruction_fragment: str
    priority: int = 5
    keywords: list[str] = field(default_factory=list)
    tool_overlay: list[str] = field(default_factory=list)
    memory_namespace: list[str] = field(default_factory=list)
    source_path: Path | None = None


@dataclass(frozen=True)
class HookDefinition:
    event: str
    match: dict[str, Any] = field(default_factory=dict)
    call: dict[str, Any] = field(default_factory=dict)
    when: str = "post"
    budget: dict[str, Any] = field(default_factory=dict)
    source_path: Path | None = None


@dataclass(frozen=True)
class AgentDefinition:
    paths: AgentPaths
    spec_yaml: dict[str, Any]
    soul: MarkdownDoc
    rules: MarkdownDoc
    memory: MarkdownDoc
    user: MarkdownDoc
    skills: list[SkillDefinition] = field(default_factory=list)
    hooks: list[HookDefinition] = field(default_factory=list)

    @property
    def agent_id(self) -> str:
        return str(self.spec_yaml.get("id") or self.paths.agent_id)

    @property
    def model(self) -> str | None:
        m = self.spec_yaml.get("model")
        return m if isinstance(m, str) else None

    @property
    def max_iterations(self) -> int:
        return int(self.spec_yaml.get("max_iterations", 8))

    @property
    def token_budget(self) -> int:
        return int(self.spec_yaml.get("token_budget", 8000))

    @property
    def time_budget_ms(self) -> int:
        return int(self.spec_yaml.get("time_budget_ms", 60_000))

    @property
    def money_budget_usd(self) -> float:
        return float(self.spec_yaml.get("money_budget_usd", 1.0))

    @property
    def skillset_id(self) -> str:
        return str(self.spec_yaml.get("skillset_id", f"{self.agent_id}-skills"))

    @property
    def skillset_version(self) -> str:
        return str(self.spec_yaml.get("skillset_version", "0.1.0"))


@dataclass
class LoadedAgent:
    definition: AgentDefinition
    spec: Any
    skill_set: Any
    persona: Any | None
    rules: list[str]
    memory_facts: list[str]
    user_profile: str


class AgentLoader:
    def __init__(self, config: HostConfig) -> None:
        self._config = config
        try:
            native_cfg = _host.HostConfig.load(str(config.paths.root))
        except RuntimeError as exc:
            raise AgentSpecError(str(exc)) from exc
        self._native = _host.AgentLoader(native_cfg)

    @property
    def config(self) -> HostConfig:
        return self._config

    def agent_ids(self) -> list[str]:
        return self._config.paths.list_agent_ids()

    def parse(self, agent_id: str) -> AgentDefinition:
        try:
            native_defn = self._native.parse(agent_id)
        except RuntimeError as exc:
            raise _translate_parse_error(exc, agent_id) from exc
        return _wrap_definition(native_defn)

    def load(self, agent_id: str) -> LoadedAgent:
        definition = self.parse(agent_id)
        if _native is None:
            raise AgentSpecError(
                "atomr_agents._native is not built — run `maturin develop` "
                "in the repo root before calling AgentLoader.load(); "
                "use AgentLoader.parse() if you only need on-disk data"
            )
        model = definition.model or self._config.default_model
        if not model:
            raise AgentSpecError(
                f"agent {definition.agent_id}: no `model` in agent.yaml and "
                "no `default_model` in config.yaml"
            )
        spec = _native.agent.AgentSpec(
            id=definition.agent_id,
            model=model,
            max_iterations=definition.max_iterations,
            token_budget=definition.token_budget,
            time_budget_ms=definition.time_budget_ms,
            money_budget_usd=definition.money_budget_usd,
        )
        skill_set = _build_skill_set(definition)
        persona = _build_persona(definition)
        rules = _split_rules(definition.rules.body)
        memory_facts = _split_facts(definition.memory.body)
        user_profile = definition.user.body
        return LoadedAgent(
            definition=definition,
            spec=spec,
            skill_set=skill_set,
            persona=persona,
            rules=rules,
            memory_facts=memory_facts,
            user_profile=user_profile,
        )


# ---------- internal helpers -------------------------------------------------


def _translate_parse_error(exc: RuntimeError, agent_id: str) -> Exception:
    msg = str(exc)
    low = msg.lower()
    if "not found" in low or "no agent directory" in low or "no such" in low:
        return AgentNotFoundError(msg)
    return AgentSpecError(msg)


def _wrap_markdown(native: Any) -> MarkdownDoc:
    sp = getattr(native, "source_path", None)
    return MarkdownDoc(
        frontmatter=dict(native.frontmatter or {}),
        body=(native.body or "").strip(),
        source_path=Path(sp) if sp else None,
    )


def _wrap_skill(native: Any) -> SkillDefinition:
    sp = getattr(native, "source_path", None)
    return SkillDefinition(
        id=native.id,
        name=native.name,
        instruction_fragment=native.instruction_fragment,
        priority=int(native.priority),
        keywords=list(native.keywords or []),
        tool_overlay=list(native.tool_overlay or []),
        memory_namespace=list(native.memory_namespace or []),
        source_path=Path(sp) if sp else None,
    )


def _wrap_hook(native: Any) -> HookDefinition:
    sp = getattr(native, "source_path", None)
    return HookDefinition(
        event=native.event,
        match=dict(native.match_ or {}),
        call=dict(native.call or {}),
        when=native.when,
        budget=dict(native.budget or {}),
        source_path=Path(sp) if sp else None,
    )


def _wrap_definition(native: Any) -> AgentDefinition:
    return AgentDefinition(
        paths=AgentPaths._wrap(native.paths),
        spec_yaml=dict(native.spec_yaml or {}),
        soul=_wrap_markdown(native.soul),
        rules=_wrap_markdown(native.rules),
        memory=_wrap_markdown(native.memory),
        user=_wrap_markdown(native.user),
        skills=[_wrap_skill(s) for s in (native.skills or [])],
        hooks=[_wrap_hook(h) for h in (native.hooks or [])],
    )


def _build_skill_set(definition: AgentDefinition) -> Any:
    assert _native is not None
    native_skills = []
    for sd in definition.skills:
        kwargs: dict[str, Any] = {
            "id": sd.id,
            "name": sd.name,
            "instruction_fragment": sd.instruction_fragment,
            "priority": sd.priority,
        }
        if sd.keywords:
            kwargs["keywords"] = list(sd.keywords)
        if sd.tool_overlay:
            kwargs["tool_overlay"] = list(sd.tool_overlay)
        native_skills.append(_native.skill.Skill(**kwargs))
    return _native.skill.SkillSet(
        id=definition.skillset_id,
        version=definition.skillset_version,
        skills=native_skills,
    )


def _build_persona(definition: AgentDefinition) -> Any | None:
    assert _native is not None
    if definition.soul.is_empty():
        return None

    fm = definition.soul.frontmatter or {}
    identity = str(fm.get("identity") or definition.agent_id)

    style: Any | None = None
    style_block = fm.get("style")
    if isinstance(style_block, dict):
        verbosity_raw = style_block.get("verbosity")
        verbosity = int(verbosity_raw) if isinstance(verbosity_raw, (int, float)) else None
        try:
            style = _native.persona.StyleSpec(
                tone=style_block.get("tone"),
                register=style_block.get("register"),
                verbosity=verbosity,
            )
        except AttributeError:  # pragma: no cover
            style = None

    traits: list[Any] = []
    traits_block = fm.get("traits") or []
    if isinstance(traits_block, list):
        for t in traits_block:
            if not isinstance(t, dict):
                continue
            try:
                traits.append(
                    _native.persona.TraitFragment(
                        label=str(t.get("label") or ""),
                        weight=float(t.get("weight", 1.0)),
                        description=str(t.get("description") or ""),
                    )
                )
            except AttributeError:  # pragma: no cover
                pass

    metadata: Any | None = None
    meta_block = fm.get("metadata")
    if isinstance(meta_block, dict):
        try:
            metadata = _native.persona.PersonaMetadata(framework=meta_block.get("framework"))
        except AttributeError:  # pragma: no cover
            metadata = None

    kwargs: dict[str, Any] = {"identity": identity}
    if traits:
        kwargs["salient_traits"] = traits
    if style is not None:
        kwargs["style"] = style
    if metadata is not None:
        kwargs["metadata"] = metadata
    try:
        return _native.persona.PersonaValue(**kwargs)
    except (AttributeError, TypeError):  # pragma: no cover
        return None


def _split_rules(body: str) -> list[str]:
    return list(_host.split_bullets(body))


def _split_facts(body: str) -> list[str]:
    return list(_host.split_bullets(body))
