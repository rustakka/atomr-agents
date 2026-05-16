"""Thin facade over ``_native.host`` that renders persona/rules/memory/user
fragments into the instruction prefix shipped to the model.

Public block renderers accept Python lists / loader dataclasses (tests
exercise both real ``LoadedAgent`` and lightweight stubs), so the
Markdown shaping stays Python-side. The bullet splitter delegates to
``_native.host.split_bullets`` so MEMORY.md/USER.md parsing matches the
Rust loader byte-for-byte.

:func:`build_chat_prompt_template` wraps the assembled prompt in the
native ``ChatPromptTemplateBuilder``.
"""

from __future__ import annotations

from typing import Any

from atomr_agents._native import host as _h

from .errors import AgentHostError

try:  # native is optional for the block renderers
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover - PyO3 extension missing
    _native = None

__all__ = [
    "build_chat_prompt_template",
    "build_system_prompt",
    "render_memory_block",
    "render_persona_block",
    "render_rules_block",
    "render_user_block",
]


# ---------- block renderers -------------------------------------------------


def render_rules_block(rules: list[str]) -> str:
    """``# Rules`` header + ``- <rule>`` bullets, or ``""`` if empty."""
    return _bulleted_block("Rules", rules)


def render_memory_block(facts: list[str]) -> str:
    """``# Memory`` header + ``- <fact>`` bullets, or ``""`` if empty."""
    return _bulleted_block("Memory", facts)


def render_user_block(user_facts: list[str]) -> str:
    """``# About the user`` header + ``- <fact>`` bullets, or ``""`` if empty."""
    return _bulleted_block("About the user", user_facts)


def render_persona_block(loaded: Any) -> str:
    """``# Persona`` header + identity + optional trait bullets."""
    persona = getattr(loaded, "persona", None)
    if persona is None:
        return ""
    identity = _coerce_str(getattr(persona, "identity", None))
    if not identity:
        return ""
    lines: list[str] = ["# Persona", "", identity]
    trait_lines = _format_traits(persona)
    if trait_lines:
        lines.append("")
        lines.extend(trait_lines)
    return "\n".join(lines)


# ---------- composed prompts ------------------------------------------------


def build_system_prompt(
    loaded: Any,
    *,
    user_facts: list[str] | None = None,
) -> str:
    """Compose persona / rules / memory / user blocks into the system prompt."""
    rules = list(getattr(loaded, "rules", []) or [])
    memory_facts = list(getattr(loaded, "memory_facts", []) or [])
    if user_facts is None:
        user_facts = _h.split_bullets(getattr(loaded, "user_profile", "") or "")
    else:
        user_facts = list(user_facts)

    blocks = [
        render_persona_block(loaded),
        render_rules_block(rules),
        render_memory_block(memory_facts),
        render_user_block(user_facts),
    ]
    rendered = "\n\n".join(b for b in blocks if b)
    if rendered:
        return rendered
    return f"You are {_resolve_agent_id(loaded)}."


def build_chat_prompt_template(
    loaded: Any,
    *,
    user_facts: list[str] | None = None,
) -> Any:
    """Build a native ``ChatPromptTemplate`` for a loaded agent."""
    if _native is None:
        raise AgentHostError(
            "atomr_agents._native is not built — run `maturin develop` "
            "before building chat prompt templates"
        )
    builder = _native.instruction.ChatPromptTemplateBuilder()
    builder.system(build_system_prompt(loaded, user_facts=user_facts))
    builder.user("{user_message}")
    return builder.build()


# ---------- internal helpers ------------------------------------------------


def _bulleted_block(title: str, items: list[str]) -> str:
    cleaned = [str(item).strip() for item in items if str(item).strip()]
    if not cleaned:
        return ""
    lines = [f"# {title}", ""]
    lines.extend(f"- {item}" for item in cleaned)
    return "\n".join(lines)


def _bullet_lines(body: str) -> list[str]:
    """Bullet splitter — delegates to the native ``split_bullets`` primitive."""
    return list(_h.split_bullets(body or ""))


def _format_traits(persona: Any) -> list[str]:
    traits = getattr(persona, "salient_traits", None)
    if not traits:
        return []
    lines: list[str] = []
    for trait in traits:
        label = _coerce_str(getattr(trait, "label", None))
        if not label:
            continue
        weight = getattr(trait, "weight", None)
        description = _coerce_str(getattr(trait, "description", None))
        weight_part = (
            f" (weight {_format_weight(float(weight))})"
            if isinstance(weight, (int, float))
            else ""
        )
        bullet = f"- {label}{weight_part}"
        if description:
            bullet = f"{bullet}: {description}"
        lines.append(bullet)
    return lines


def _coerce_str(value: Any) -> str:
    if value is None:
        return ""
    try:
        return str(value).strip()
    except Exception:  # pragma: no cover - defensive
        return ""


def _format_weight(weight: float) -> str:
    text = f"{weight:.6f}".rstrip("0").rstrip(".")
    return text or "0"


def _resolve_agent_id(loaded: Any) -> str:
    spec = getattr(loaded, "spec", None)
    if spec is not None:
        agent_id = getattr(spec, "id", None)
        if isinstance(agent_id, str) and agent_id:
            return agent_id
    definition = getattr(loaded, "definition", None)
    if definition is not None:
        agent_id = getattr(definition, "agent_id", None)
        if isinstance(agent_id, str) and agent_id:
            return agent_id
    return "the agent"
