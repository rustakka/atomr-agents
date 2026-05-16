"""Tests for ``atomr_agents.agent_host.registry`` (M11 disk cache).

These tests deliberately avoid the ``_native`` extension: M11's
``pull_artifact`` operates against any object exposing the duck-typed
``.get(kind, id, version)`` / ``.latest(kind, id)`` surface, and the
suite uses a tiny in-memory fake to exercise that contract.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

from atomr_agents.agent_host.errors import AgentHostError, AgentSpecError
from atomr_agents.agent_host.layout import HostPaths
from atomr_agents.agent_host.registry import (
    ARTIFACT_KINDS,
    CachedArtifact,
    cache_artifact,
    cache_path,
    delete_artifact,
    list_artifacts,
    parse_slug,
    pull_artifact,
    resolve_artifact,
    verify_cache,
)


# ---------------------------------------------------------------------------
# Fixtures / helpers
# ---------------------------------------------------------------------------


@pytest.fixture()
def host_paths(tmp_path: Path) -> HostPaths:
    paths = HostPaths(root=tmp_path)
    paths.ensure()
    return paths


class FakeRegistry:
    """Minimal stand-in for ``_native.registry.Registry``.

    Stores artifacts keyed by ``(kind, id, version)`` and returns plain
    dicts (so the registry module's normalization path is exercised
    against dict-shaped records).
    """

    def __init__(self, entries: dict[tuple[str, str, str], dict]) -> None:
        self._entries: dict[tuple[str, str, str], dict] = dict(entries)

    def get(self, kind: str, id: str, version: str) -> dict | None:
        record = self._entries.get((kind, id, version))
        if record is None:
            return None
        return {
            "kind": kind,
            "id": id,
            "version": version,
            "payload": dict(record),
        }

    def latest(self, kind: str, id: str) -> dict | None:
        versions = [
            version
            for (k, i, version) in self._entries
            if k == kind and i == id
        ]
        if not versions:
            return None
        versions.sort()
        version = versions[-1]
        return self.get(kind, id, version)

    def set(self, kind: str, id: str, version: str, payload: dict) -> None:
        self._entries[(kind, id, version)] = dict(payload)

    def remove(self, kind: str, id: str, version: str) -> None:
        self._entries.pop((kind, id, version), None)


# ---------------------------------------------------------------------------
# parse_slug
# ---------------------------------------------------------------------------


def test_parse_slug_happy_path() -> None:
    assert parse_slug("skill:summarize@0.1.0") == ("skill", "summarize", "0.1.0")
    assert parse_slug("tool_set:web@1.2.3-beta") == ("tool_set", "web", "1.2.3-beta")


@pytest.mark.parametrize(
    "slug",
    [
        "",
        "skill-summarize@0.1.0",  # no colon
        "skill:summarize",  # no @version
        "skill:@0.1.0",  # empty id
        ":summarize@0.1.0",  # empty kind
        "skill:summarize@",  # empty version
        "skill:summarize@0.1.0@extra",  # trailing garbage
    ],
)
def test_parse_slug_bad_inputs(slug: str) -> None:
    with pytest.raises(AgentSpecError):
        parse_slug(slug)


def test_parse_slug_rejects_non_string() -> None:
    with pytest.raises(AgentSpecError):
        parse_slug(None)  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# cache_path
# ---------------------------------------------------------------------------


def test_cache_path_layout(host_paths: HostPaths) -> None:
    path = cache_path(host_paths, "skill", "summarize", "0.1.0")
    expected = host_paths.registry_dir / "skill" / "summarize" / "0.1.0.json"
    assert path == expected


# ---------------------------------------------------------------------------
# cache_artifact
# ---------------------------------------------------------------------------


def test_cache_artifact_writes_parseable_json(host_paths: HostPaths) -> None:
    payload = {"prompt": "summarize this", "model": "gpt-4o"}
    before_ms = int(time.time() * 1000)
    artifact = cache_artifact(
        host_paths,
        kind="skill",
        id="summarize",
        version="0.1.0",
        payload=payload,
    )
    after_ms = int(time.time() * 1000)

    assert isinstance(artifact, CachedArtifact)
    assert artifact.kind == "skill"
    assert artifact.id == "summarize"
    assert artifact.version == "0.1.0"
    assert artifact.path.is_file()
    assert artifact.path == cache_path(host_paths, "skill", "summarize", "0.1.0")
    assert artifact.payload == payload
    assert artifact.slug == "skill:summarize@0.1.0"

    envelope = json.loads(artifact.path.read_text(encoding="utf-8"))
    assert envelope["kind"] == "skill"
    assert envelope["id"] == "summarize"
    assert envelope["version"] == "0.1.0"
    assert envelope["payload"] == payload
    assert before_ms <= envelope["cached_at_ms"] <= after_ms


def test_cache_artifact_rejects_unknown_kind(host_paths: HostPaths) -> None:
    with pytest.raises(AgentSpecError):
        cache_artifact(
            host_paths,
            kind="not_a_real_kind",
            id="x",
            version="0.1.0",
            payload={},
        )


def test_cache_artifact_rejects_non_dict_payload(host_paths: HostPaths) -> None:
    with pytest.raises(AgentSpecError):
        cache_artifact(
            host_paths,
            kind="skill",
            id="x",
            version="0.1.0",
            payload="not a dict",  # type: ignore[arg-type]
        )


def test_cache_artifact_accepts_every_known_kind(host_paths: HostPaths) -> None:
    for kind in ARTIFACT_KINDS:
        artifact = cache_artifact(
            host_paths,
            kind=kind,
            id=f"sample-{kind}",
            version="0.0.1",
            payload={"k": kind},
        )
        assert artifact.kind == kind
        assert artifact.path.is_file()


# ---------------------------------------------------------------------------
# resolve_artifact
# ---------------------------------------------------------------------------


def test_resolve_artifact_reads_back_payload(host_paths: HostPaths) -> None:
    payload = {"hello": "world"}
    cache_artifact(
        host_paths,
        kind="skill",
        id="summarize",
        version="0.1.0",
        payload=payload,
    )
    resolved = resolve_artifact(
        host_paths, kind="skill", id="summarize", version="0.1.0"
    )
    assert resolved is not None
    assert resolved.payload == payload
    assert resolved.version == "0.1.0"


def test_resolve_artifact_missing_returns_none(host_paths: HostPaths) -> None:
    assert (
        resolve_artifact(host_paths, kind="skill", id="nope", version="0.1.0") is None
    )
    # And with version=None when nothing at all is cached for that id.
    assert resolve_artifact(host_paths, kind="skill", id="nope") is None


def test_resolve_artifact_newest_version(host_paths: HostPaths) -> None:
    cache_artifact(
        host_paths,
        kind="skill",
        id="summarize",
        version="0.1.0",
        payload={"v": "old"},
    )
    cache_artifact(
        host_paths,
        kind="skill",
        id="summarize",
        version="0.2.0",
        payload={"v": "new"},
    )
    resolved = resolve_artifact(host_paths, kind="skill", id="summarize")
    assert resolved is not None
    assert resolved.version == "0.2.0"
    assert resolved.payload == {"v": "new"}


# ---------------------------------------------------------------------------
# list_artifacts
# ---------------------------------------------------------------------------


def test_list_artifacts_empty_cache(host_paths: HostPaths) -> None:
    assert list_artifacts(host_paths) == []


def test_list_artifacts_sorted_and_filtered(host_paths: HostPaths) -> None:
    cache_artifact(
        host_paths, kind="skill", id="b-skill", version="0.1.0", payload={"id": "b"}
    )
    cache_artifact(
        host_paths, kind="skill", id="a-skill", version="0.1.0", payload={"id": "a"}
    )
    cache_artifact(
        host_paths, kind="skill", id="a-skill", version="0.2.0", payload={"id": "a2"}
    )
    cache_artifact(
        host_paths, kind="agent", id="z-agent", version="0.1.0", payload={"id": "z"}
    )

    all_artifacts = list_artifacts(host_paths)
    keys = [(a.kind, a.id, a.version) for a in all_artifacts]
    assert keys == [
        ("agent", "z-agent", "0.1.0"),
        ("skill", "a-skill", "0.1.0"),
        ("skill", "a-skill", "0.2.0"),
        ("skill", "b-skill", "0.1.0"),
    ]

    skills_only = list_artifacts(host_paths, kind="skill")
    skill_keys = [(a.kind, a.id, a.version) for a in skills_only]
    assert skill_keys == [
        ("skill", "a-skill", "0.1.0"),
        ("skill", "a-skill", "0.2.0"),
        ("skill", "b-skill", "0.1.0"),
    ]


def test_list_artifacts_ignores_stray_directories(host_paths: HostPaths) -> None:
    # A directory that doesn't match any known artifact kind shouldn't
    # cause list_artifacts to throw.
    (host_paths.registry_dir / "unknown_kind").mkdir(parents=True)
    cache_artifact(
        host_paths, kind="skill", id="x", version="0.1.0", payload={}
    )
    artifacts = list_artifacts(host_paths)
    assert [a.kind for a in artifacts] == ["skill"]


# ---------------------------------------------------------------------------
# pull_artifact
# ---------------------------------------------------------------------------


def test_pull_artifact_with_explicit_version(host_paths: HostPaths) -> None:
    registry = FakeRegistry(
        {("skill", "summarize", "0.1.0"): {"prompt": "summarize"}}
    )
    artifact = pull_artifact(
        host_paths, registry, kind="skill", id="summarize", version="0.1.0"
    )
    assert artifact.payload == {"prompt": "summarize"}
    assert artifact.path.is_file()

    # And reads back via resolve.
    resolved = resolve_artifact(
        host_paths, kind="skill", id="summarize", version="0.1.0"
    )
    assert resolved is not None
    assert resolved.payload == {"prompt": "summarize"}


def test_pull_artifact_with_version_none_uses_latest(host_paths: HostPaths) -> None:
    registry = FakeRegistry(
        {
            ("skill", "summarize", "0.1.0"): {"v": "old"},
            ("skill", "summarize", "0.2.0"): {"v": "new"},
        }
    )
    artifact = pull_artifact(host_paths, registry, kind="skill", id="summarize")
    assert artifact.version == "0.2.0"
    assert artifact.payload == {"v": "new"}


def test_pull_artifact_missing_raises(host_paths: HostPaths) -> None:
    registry = FakeRegistry({})
    with pytest.raises(AgentHostError):
        pull_artifact(
            host_paths, registry, kind="skill", id="nope", version="0.1.0"
        )
    with pytest.raises(AgentHostError):
        pull_artifact(host_paths, registry, kind="skill", id="nope")


def test_pull_artifact_with_object_record(host_paths: HostPaths) -> None:
    """``pull_artifact`` should also accept native-style record objects."""

    class _Record:
        def __init__(self, kind: str, id: str, version: str, payload: dict) -> None:
            self.kind = kind
            self.id = id
            self.version = version
            self.payload = payload

    class _ObjRegistry:
        def get(self, kind: str, id: str, version: str) -> _Record | None:
            if (kind, id, version) == ("skill", "summarize", "0.3.0"):
                return _Record(kind, id, version, {"obj": True})
            return None

        def latest(self, kind: str, id: str) -> _Record | None:
            return self.get(kind, id, "0.3.0")

    artifact = pull_artifact(
        host_paths, _ObjRegistry(), kind="skill", id="summarize", version="0.3.0"
    )
    assert artifact.payload == {"obj": True}


# ---------------------------------------------------------------------------
# verify_cache
# ---------------------------------------------------------------------------


def test_verify_cache_consistent(host_paths: HostPaths) -> None:
    registry = FakeRegistry(
        {("skill", "summarize", "0.1.0"): {"prompt": "summarize"}}
    )
    pull_artifact(host_paths, registry, kind="skill", id="summarize", version="0.1.0")
    assert verify_cache(host_paths, registry) == []


def test_verify_cache_reports_mismatch(host_paths: HostPaths) -> None:
    registry = FakeRegistry(
        {("skill", "summarize", "0.1.0"): {"prompt": "summarize"}}
    )
    pull_artifact(host_paths, registry, kind="skill", id="summarize", version="0.1.0")
    # Mutate the registry side so the payload no longer matches the
    # cached one.
    registry.set("skill", "summarize", "0.1.0", {"prompt": "DIFFERENT"})
    discrepancies = verify_cache(host_paths, registry)
    assert len(discrepancies) == 1
    artifact, reason = discrepancies[0]
    assert reason == "mismatch"
    assert artifact.slug == "skill:summarize@0.1.0"


def test_verify_cache_reports_missing(host_paths: HostPaths) -> None:
    registry = FakeRegistry(
        {("skill", "summarize", "0.1.0"): {"prompt": "summarize"}}
    )
    pull_artifact(host_paths, registry, kind="skill", id="summarize", version="0.1.0")
    registry.remove("skill", "summarize", "0.1.0")
    discrepancies = verify_cache(host_paths, registry)
    assert len(discrepancies) == 1
    artifact, reason = discrepancies[0]
    assert reason == "missing"
    assert artifact.slug == "skill:summarize@0.1.0"


# ---------------------------------------------------------------------------
# delete_artifact
# ---------------------------------------------------------------------------


def test_delete_artifact_removes_file(host_paths: HostPaths) -> None:
    cache_artifact(
        host_paths, kind="skill", id="summarize", version="0.1.0", payload={"x": 1}
    )
    path = cache_path(host_paths, "skill", "summarize", "0.1.0")
    assert path.is_file()
    assert (
        delete_artifact(host_paths, kind="skill", id="summarize", version="0.1.0")
        is True
    )
    assert not path.is_file()


def test_delete_artifact_missing_returns_false(host_paths: HostPaths) -> None:
    assert (
        delete_artifact(host_paths, kind="skill", id="absent", version="0.1.0")
        is False
    )
