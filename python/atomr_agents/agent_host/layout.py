"""On-disk layout for an atomr-agents host root — thin facade over ``_native.host``."""

from __future__ import annotations

import os
from pathlib import Path

from atomr_agents._native import host as _host

__all__ = ["AgentPaths", "HostPaths", "default_root", "ENV_ROOT"]

ENV_ROOT = "ATOMR_HOST_ROOT"


def default_root() -> Path:
    env = os.environ.get(ENV_ROOT)
    if env:
        return Path(env).expanduser().resolve()
    return Path(_host.default_root())


class AgentPaths:
    __slots__ = ("_inner",)

    def __init__(self, root: Path | str, agent_id: str) -> None:
        if isinstance(root, _host.HostPaths):
            self._inner = root.agent(agent_id)
        elif isinstance(root, _host.AgentPaths):
            self._inner = root
        else:
            self._inner = _host.HostPaths(root=str(root)).agent(agent_id)

    @classmethod
    def _wrap(cls, inner: object) -> AgentPaths:
        obj = cls.__new__(cls)
        obj._inner = inner
        return obj

    @property
    def root(self) -> Path:
        return Path(self._inner.root)

    @property
    def agent_id(self) -> str:
        return self._inner.agent_id

    @property
    def dir(self) -> Path:
        return Path(self._inner.dir)

    @property
    def agent_yaml(self) -> Path:
        return Path(self._inner.agent_yaml)

    @property
    def soul_md(self) -> Path:
        return Path(self._inner.soul_md)

    @property
    def rules_md(self) -> Path:
        return Path(self._inner.rules_md)

    @property
    def memory_md(self) -> Path:
        return Path(self._inner.memory_md)

    @property
    def user_md(self) -> Path:
        return Path(self._inner.user_md)

    @property
    def skills_dir(self) -> Path:
        return Path(self._inner.skills_dir)

    @property
    def hooks_dir(self) -> Path:
        return Path(self._inner.hooks_dir)

    @property
    def state_dir(self) -> Path:
        return Path(self._inner.state_dir)

    @property
    def threads_dir(self) -> Path:
        return Path(self._inner.threads_dir)

    @property
    def checkpoints_dir(self) -> Path:
        return Path(self._inner.checkpoints_dir)

    @property
    def memory_db(self) -> Path:
        return Path(self._inner.memory_db)

    def ensure(self) -> None:
        self._inner.ensure()

    def __eq__(self, other: object) -> bool:
        if isinstance(other, AgentPaths):
            return self.root == other.root and self.agent_id == other.agent_id
        return NotImplemented

    def __hash__(self) -> int:
        return hash((self.root, self.agent_id))

    def __repr__(self) -> str:
        return f"AgentPaths(root={self.root!r}, agent_id={self.agent_id!r})"


class HostPaths:
    __slots__ = ("_inner",)

    def __init__(self, root: Path | str) -> None:
        self._inner = _host.HostPaths(root=str(root))

    @classmethod
    def _wrap(cls, inner: object) -> HostPaths:
        obj = cls.__new__(cls)
        obj._inner = inner
        return obj

    @property
    def root(self) -> Path:
        return Path(self._inner.root)

    @property
    def config_yaml(self) -> Path:
        return Path(self._inner.config_yaml)

    @property
    def agents_md(self) -> Path:
        return Path(self._inner.agents_md)

    @property
    def agents_dir(self) -> Path:
        return Path(self._inner.agents_dir)

    @property
    def channels_dir(self) -> Path:
        return Path(self._inner.channels_dir)

    @property
    def crons_dir(self) -> Path:
        return Path(self._inner.crons_dir)

    @property
    def tools_dir(self) -> Path:
        return Path(self._inner.tools_dir)

    @property
    def registry_dir(self) -> Path:
        return Path(self._inner.registry_dir)

    @property
    def events_jsonl(self) -> Path:
        return Path(self._inner.events_jsonl)

    def agent(self, agent_id: str) -> AgentPaths:
        return AgentPaths._wrap(self._inner.agent(agent_id))

    def list_agent_ids(self) -> list[str]:
        return list(self._inner.list_agent_ids())

    def ensure(self) -> None:
        self._inner.ensure()

    def __eq__(self, other: object) -> bool:
        if isinstance(other, HostPaths):
            return self.root == other.root
        return NotImplemented

    def __hash__(self) -> int:
        return hash(self.root)

    def __repr__(self) -> str:
        return f"HostPaths(root={self.root!r})"
