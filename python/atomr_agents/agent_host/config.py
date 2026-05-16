"""HostConfig — thin facade over ``_native.host.HostConfig``."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from atomr_agents._native import host as _host

from .errors import HostConfigError
from .layout import HostPaths, default_root

__all__ = ["HostConfig", "ProviderConfig"]


@dataclass(frozen=True)
class ProviderConfig:
    name: str
    kind: str
    api_key_env: str | None = None
    base_url: str | None = None
    extra: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class HostConfig:
    paths: HostPaths
    version: int = 1
    default_agent: str | None = None
    default_model: str | None = None
    providers: dict[str, ProviderConfig] = field(default_factory=dict)
    extra: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def load_default(cls) -> HostConfig:
        return cls.load(default_root())

    @classmethod
    def load(cls, root: Path | str) -> HostConfig:
        try:
            native = _host.HostConfig.load(str(root))
        except RuntimeError as exc:
            raise HostConfigError(str(exc)) from exc
        return cls._from_native(native)

    @classmethod
    def from_mapping(cls, raw: dict[str, Any], *, paths: HostPaths) -> HostConfig:
        version = int(raw.get("version", 1))
        default_agent = raw.get("default_agent")
        default_model = raw.get("default_model")
        providers_raw = raw.get("providers") or {}
        if not isinstance(providers_raw, dict):
            raise HostConfigError("`providers` must be a mapping of name → provider config")
        providers: dict[str, ProviderConfig] = {}
        for name, body in providers_raw.items():
            if not isinstance(body, dict):
                raise HostConfigError(f"provider `{name}` must be a mapping")
            kind = body.get("kind")
            if not isinstance(kind, str) or not kind:
                raise HostConfigError(f"provider `{name}` is missing a string `kind`")
            providers[name] = ProviderConfig(
                name=name,
                kind=kind,
                api_key_env=body.get("api_key_env"),
                base_url=body.get("base_url"),
                extra={
                    k: v
                    for k, v in body.items()
                    if k not in {"kind", "api_key_env", "base_url"}
                },
            )
        extra = {
            k: v
            for k, v in raw.items()
            if k not in {"version", "default_agent", "default_model", "providers"}
        }
        return cls(
            paths=paths,
            version=version,
            default_agent=default_agent if isinstance(default_agent, str) else None,
            default_model=default_model if isinstance(default_model, str) else None,
            providers=providers,
            extra=extra,
        )

    @classmethod
    def _from_native(cls, native: Any) -> HostConfig:
        providers = {
            name: ProviderConfig(
                name=p.name,
                kind=p.kind,
                api_key_env=p.api_key_env,
                base_url=p.base_url,
                extra=dict(p.extra or {}),
            )
            for name, p in (native.providers or {}).items()
        }
        return cls(
            paths=HostPaths._wrap(native.paths),
            version=int(native.version),
            default_agent=native.default_agent,
            default_model=native.default_model,
            providers=providers,
            extra=dict(native.extra or {}),
        )

    def to_mapping(self) -> dict[str, Any]:
        providers: dict[str, Any] = {}
        for name, p in self.providers.items():
            entry: dict[str, Any] = {"kind": p.kind}
            if p.api_key_env is not None:
                entry["api_key_env"] = p.api_key_env
            if p.base_url is not None:
                entry["base_url"] = p.base_url
            entry.update(p.extra)
            providers[name] = entry
        out: dict[str, Any] = {"version": self.version}
        if self.default_agent is not None:
            out["default_agent"] = self.default_agent
        if self.default_model is not None:
            out["default_model"] = self.default_model
        if providers:
            out["providers"] = providers
        out.update(self.extra)
        return out
