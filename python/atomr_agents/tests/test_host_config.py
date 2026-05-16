"""Tests for HostConfig loading and serialization."""

from __future__ import annotations

import textwrap
from pathlib import Path

import pytest

from atomr_agents.agent_host import HostConfig, ProviderConfig
from atomr_agents.agent_host.errors import HostConfigError

pytest.importorskip("yaml")


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "agent_host"


def test_load_fixture_config() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    assert cfg.version == 1
    assert cfg.default_agent == "alpha"
    assert cfg.default_model == "gpt-4o"
    assert set(cfg.providers) == {"openai", "anthropic"}
    openai = cfg.providers["openai"]
    assert openai.kind == "openai"
    assert openai.api_key_env == "OPENAI_API_KEY"
    assert openai.base_url == "https://api.openai.com/v1"


def test_load_missing_config_returns_bare(tmp_path: Path) -> None:
    cfg = HostConfig.load(tmp_path)
    assert cfg.version == 1
    assert cfg.default_agent is None
    assert cfg.providers == {}
    assert cfg.paths.root == tmp_path.resolve()


def test_load_invalid_top_level_type(tmp_path: Path) -> None:
    (tmp_path / "config.yaml").write_text("- a\n- b\n", encoding="utf-8")
    with pytest.raises(HostConfigError):
        HostConfig.load(tmp_path)


def test_load_invalid_yaml(tmp_path: Path) -> None:
    (tmp_path / "config.yaml").write_text("key: value\n  bad: indent\n", encoding="utf-8")
    with pytest.raises(HostConfigError):
        HostConfig.load(tmp_path)


def test_load_provider_missing_kind(tmp_path: Path) -> None:
    (tmp_path / "config.yaml").write_text(
        textwrap.dedent(
            """
            providers:
              foo:
                api_key_env: FOO_KEY
            """
        ).strip(),
        encoding="utf-8",
    )
    with pytest.raises(HostConfigError):
        HostConfig.load(tmp_path)


def test_to_mapping_round_trip() -> None:
    cfg = HostConfig.load(FIXTURE_ROOT)
    mapping = cfg.to_mapping()
    assert mapping["default_agent"] == "alpha"
    assert mapping["providers"]["openai"]["kind"] == "openai"
    assert mapping["providers"]["openai"]["api_key_env"] == "OPENAI_API_KEY"


def test_provider_config_extra_fields_preserved(tmp_path: Path) -> None:
    (tmp_path / "config.yaml").write_text(
        textwrap.dedent(
            """
            providers:
              custom:
                kind: custom
                api_key_env: CK
                extra_setting: hello
                timeout_ms: 1500
            """
        ).strip(),
        encoding="utf-8",
    )
    cfg = HostConfig.load(tmp_path)
    p = cfg.providers["custom"]
    assert p.extra == {"extra_setting": "hello", "timeout_ms": 1500}


def test_provider_config_dataclass_constructible() -> None:
    p = ProviderConfig(name="o", kind="openai", api_key_env="OAI")
    assert p.kind == "openai"
    assert p.extra == {}
