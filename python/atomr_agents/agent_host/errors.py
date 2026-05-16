"""Exceptions raised by the agent-host package."""

from __future__ import annotations


class AgentHostError(Exception):
    """Base class for agent-host failures."""


class HostConfigError(AgentHostError):
    """Raised when the host root or ``config.yaml`` cannot be read."""


class AgentNotFoundError(AgentHostError):
    """Raised when an agent id has no directory under ``agents/``."""


class AgentSpecError(AgentHostError):
    """Raised when ``agent.yaml`` is missing fields or has invalid values."""


class MarkdownParseError(AgentHostError):
    """Raised when a SOUL/RULES/MEMORY/USER/SKILL markdown file is malformed.

    Carries the offending file path so callers can surface a useful message.
    """

    def __init__(self, message: str, *, path: str | None = None) -> None:
        super().__init__(message)
        self.path = path

    def __str__(self) -> str:  # pragma: no cover - trivial
        base = super().__str__()
        return f"{base} ({self.path})" if self.path else base
