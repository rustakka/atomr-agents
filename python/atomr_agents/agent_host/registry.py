"""Disk cache for registry artifacts (M11).

The Rust ``crates/registry`` (and its Python facade
``atomr_agents._native.registry.Registry``) is an in-memory index of
``ArtifactRecord`` rows.  This module is the host-side complement: a
thin **disk cache** rooted at ``<host_root>/registry/`` so the agent
host can resolve previously-pulled artifacts without re-fetching them.

Cache layout::

    <host_root>/registry/<kind>/<id>/<version>.json

Each file is a JSON document with the fields::

    {
      "kind":          "skill",
      "id":            "summarize",
      "version":       "0.1.0",
      "payload":       { ...arbitrary artifact body... },
      "cached_at_ms":  1715731200000
    }

M11 ships the cache + resolver layer.  The actual over-the-network pull
(e.g. fetching from an HTTP "ClawHub" registry) is deferred — for now
``pull_artifact`` takes a ``Registry`` instance (or any object that
exposes ``.get(kind, id, version)`` / ``.latest(kind, id)``) and copies
the row into the on-disk cache.
"""

from __future__ import annotations

import json
import re
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .config import HostConfig  # noqa: F401 — re-exported / typing hint
from .errors import AgentHostError, AgentSpecError
from .layout import HostPaths

try:  # pragma: no cover - the native extension is optional at import time
    from atomr_agents import _native as _native_pkg

    _native: Any | None = _native_pkg
except ImportError:  # pragma: no cover
    _native = None

__all__ = [
    "ARTIFACT_KINDS",
    "CachedArtifact",
    "cache_artifact",
    "cache_path",
    "delete_artifact",
    "list_artifacts",
    "parse_slug",
    "pull_artifact",
    "resolve_artifact",
    "verify_cache",
]

ARTIFACT_KINDS: tuple[str, ...] = (
    "tool_set",
    "skill",
    "persona",
    "agent",
    "workflow",
    "harness",
    "channel",
)

# Slug form: ``<kind>:<id>@<version>``.  We accept the same characters
# the rest of the codebase uses for identifiers (alphanumerics plus a
# small set of separators) so the regex catches obvious typos but
# tolerates the loose semver strings the registry stores.
_SLUG_RE = re.compile(
    r"^(?P<kind>[A-Za-z][A-Za-z0-9_]*):"
    r"(?P<id>[A-Za-z0-9][A-Za-z0-9_.\-/]*)@"
    r"(?P<version>[A-Za-z0-9][A-Za-z0-9_.\-+]*)$"
)


@dataclass(frozen=True)
class CachedArtifact:
    """A single artifact materialized on disk.

    ``path`` is the absolute JSON file under
    ``<root>/registry/<kind>/<id>/<version>.json`` and ``payload`` is
    the parsed artifact body (the ``payload`` field of the on-disk JSON,
    *not* the wrapper).
    """

    kind: str
    id: str
    version: str
    path: Path
    payload: dict

    @property
    def slug(self) -> str:
        """Return the canonical ``<kind>:<id>@<version>`` slug."""
        return f"{self.kind}:{self.id}@{self.version}"


# ---------------------------------------------------------------------------
# Slug + path helpers
# ---------------------------------------------------------------------------


def parse_slug(slug: str) -> tuple[str, str, str]:
    """Parse ``<kind>:<id>@<version>`` into a ``(kind, id, version)`` tuple.

    Raises :class:`AgentSpecError` on malformed input.
    """
    if not isinstance(slug, str) or not slug:
        raise AgentSpecError("artifact slug must be a non-empty string")
    match = _SLUG_RE.match(slug)
    if match is None:
        raise AgentSpecError(
            f"invalid artifact slug {slug!r}; expected '<kind>:<id>@<version>'"
        )
    kind = match.group("kind")
    artifact_id = match.group("id")
    version = match.group("version")
    if not kind or not artifact_id or not version:
        raise AgentSpecError(
            f"invalid artifact slug {slug!r}; kind/id/version must be non-empty"
        )
    return kind, artifact_id, version


def cache_path(host_paths: HostPaths, kind: str, id: str, version: str) -> Path:
    """Return the on-disk path for a cached artifact.

    The path is *not* created; callers that intend to write should use
    :func:`cache_artifact`.
    """
    return host_paths.registry_dir / kind / id / f"{version}.json"


# ---------------------------------------------------------------------------
# Normalization
# ---------------------------------------------------------------------------


def _validate_kind(kind: str) -> None:
    if kind not in ARTIFACT_KINDS:
        raise AgentSpecError(
            f"unknown artifact kind {kind!r}; expected one of {ARTIFACT_KINDS}"
        )


def _validate_identifier(name: str, *, label: str) -> None:
    if not isinstance(name, str) or not name:
        raise AgentSpecError(f"artifact {label} must be a non-empty string")


def _coerce_record(record: Any) -> tuple[str, str, str, dict] | None:
    """Normalize a registry record into ``(kind, id, version, payload)``.

    ``record`` may be a plain dict (as returned by an in-memory fake) or
    an object that mirrors the native ``ArtifactRecord`` surface
    (attributes ``kind``, ``id``, ``version``, ``payload``).  Returns
    ``None`` if ``record`` is ``None`` so callers can use this as a
    convenient existence check.
    """
    if record is None:
        return None
    if isinstance(record, dict):
        kind = record.get("kind")
        artifact_id = record.get("id")
        version = record.get("version")
        payload = record.get("payload")
    else:
        kind = getattr(record, "kind", None)
        artifact_id = getattr(record, "id", None)
        version = getattr(record, "version", None)
        payload = getattr(record, "payload", None)
    if kind is None or artifact_id is None or version is None:
        raise AgentHostError(
            "registry record is missing kind/id/version; got "
            f"kind={kind!r} id={artifact_id!r} version={version!r}"
        )
    if payload is None:
        payload = {}
    if not isinstance(payload, dict):
        raise AgentHostError(
            f"registry record payload must be a dict; got {type(payload).__name__}"
        )
    return str(kind), str(artifact_id), str(version), dict(payload)


# ---------------------------------------------------------------------------
# Write / read
# ---------------------------------------------------------------------------


def cache_artifact(
    host_paths: HostPaths,
    *,
    kind: str,
    id: str,
    version: str,
    payload: dict,
) -> CachedArtifact:
    """Write ``payload`` to the cache and return a :class:`CachedArtifact`.

    The parent directory is created as needed.  ``kind`` is validated
    against :data:`ARTIFACT_KINDS`; ``id`` / ``version`` must be
    non-empty strings.  ``payload`` is written verbatim, wrapped in the
    on-disk envelope so we can later look up the cached_at timestamp.
    """
    _validate_kind(kind)
    _validate_identifier(id, label="id")
    _validate_identifier(version, label="version")
    if not isinstance(payload, dict):
        raise AgentSpecError(
            f"artifact payload must be a dict; got {type(payload).__name__}"
        )

    path = cache_path(host_paths, kind, id, version)
    path.parent.mkdir(parents=True, exist_ok=True)
    envelope = {
        "kind": kind,
        "id": id,
        "version": version,
        "payload": payload,
        "cached_at_ms": int(time.time() * 1000),
    }
    # Write through a temp file to keep concurrent readers from seeing
    # half-written JSON.
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(envelope, sort_keys=True, indent=2), encoding="utf-8")
    tmp.replace(path)
    return CachedArtifact(
        kind=kind,
        id=id,
        version=version,
        path=path,
        payload=dict(payload),
    )


def _read_envelope(path: Path) -> CachedArtifact | None:
    """Read and decode a cache file.  Returns ``None`` for missing files."""
    if not path.is_file():
        return None
    try:
        envelope = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise AgentHostError(f"failed to read cached artifact at {path}: {exc}") from exc
    if not isinstance(envelope, dict):
        raise AgentHostError(f"cached artifact at {path} is not a JSON object")
    kind = envelope.get("kind")
    artifact_id = envelope.get("id")
    version = envelope.get("version")
    payload = envelope.get("payload", {})
    if not isinstance(kind, str) or not isinstance(artifact_id, str) or not isinstance(
        version, str
    ):
        raise AgentHostError(
            f"cached artifact at {path} is missing kind/id/version fields"
        )
    if not isinstance(payload, dict):
        raise AgentHostError(
            f"cached artifact at {path} has non-object payload"
        )
    return CachedArtifact(
        kind=kind,
        id=artifact_id,
        version=version,
        path=path,
        payload=payload,
    )


# ---------------------------------------------------------------------------
# Pull / resolve / list
# ---------------------------------------------------------------------------


def pull_artifact(
    host_paths: HostPaths,
    registry: Any,
    *,
    kind: str,
    id: str,
    version: str | None = None,
) -> CachedArtifact:
    """Look ``<kind>/<id>@<version>`` up in ``registry`` and cache it.

    ``registry`` is any object exposing ``.get(kind, id, version)`` and
    ``.latest(kind, id)``.  When ``version`` is ``None`` we resolve via
    ``.latest(...)`` first.  Raises :class:`AgentHostError` when the
    registry has nothing matching.
    """
    _validate_kind(kind)
    _validate_identifier(id, label="id")

    target_version = version
    if target_version is None:
        latest = getattr(registry, "latest", None)
        if not callable(latest):
            raise AgentHostError(
                "registry object has no callable `.latest(kind, id)`; pass an explicit version"
            )
        record = latest(kind, id)
        normalized = _coerce_record(record)
        if normalized is None:
            raise AgentHostError(
                f"registry has no entries for {kind}:{id}"
            )
        _, _, target_version, payload = normalized
        # Already have the payload in hand; skip the second lookup.
        return cache_artifact(
            host_paths,
            kind=kind,
            id=id,
            version=target_version,
            payload=payload,
        )

    _validate_identifier(target_version, label="version")
    get = getattr(registry, "get", None)
    if not callable(get):
        raise AgentHostError("registry object has no callable `.get(kind, id, version)`")
    record = get(kind, id, target_version)
    normalized = _coerce_record(record)
    if normalized is None:
        raise AgentHostError(
            f"registry has no entry for {kind}:{id}@{target_version}"
        )
    _, _, resolved_version, payload = normalized
    # Honor the registry's reported version (it may differ from the
    # request if the registry normalizes it).
    return cache_artifact(
        host_paths,
        kind=kind,
        id=id,
        version=resolved_version,
        payload=payload,
    )


def resolve_artifact(
    host_paths: HostPaths,
    *,
    kind: str,
    id: str,
    version: str | None = None,
) -> CachedArtifact | None:
    """Read the cached artifact from disk; ``None`` when not cached.

    With ``version=None`` we pick the newest cached version, sorted
    lexicographically and then by modification time as a tie-breaker.

    NOTE: lexicographic ordering is a deliberate M11 shortcut — it does
    the right thing for ``"0.1.0" < "0.2.0"`` style strings but does not
    implement full semver precedence (e.g. ``"0.10.0"`` sorts before
    ``"0.2.0"``).  A proper semver comparator can be layered on later
    once we settle on the dependency footprint.
    """
    _validate_kind(kind)
    _validate_identifier(id, label="id")

    if version is not None:
        _validate_identifier(version, label="version")
        return _read_envelope(cache_path(host_paths, kind, id, version))

    artifact_dir = host_paths.registry_dir / kind / id
    if not artifact_dir.is_dir():
        return None
    candidates: list[tuple[str, float, Path]] = []
    for entry in artifact_dir.iterdir():
        if not entry.is_file() or entry.suffix != ".json":
            continue
        candidates.append((entry.stem, entry.stat().st_mtime, entry))
    if not candidates:
        return None
    candidates.sort(key=lambda c: (c[0], c[1]))
    return _read_envelope(candidates[-1][2])


def list_artifacts(
    host_paths: HostPaths,
    *,
    kind: str | None = None,
) -> list[CachedArtifact]:
    """Walk the cache and return every artifact (optionally filtered).

    Entries are sorted by ``(kind, id, version)``.
    """
    if kind is not None:
        _validate_kind(kind)

    registry_dir = host_paths.registry_dir
    if not registry_dir.is_dir():
        return []

    results: list[CachedArtifact] = []
    kind_dirs: list[Path]
    if kind is None:
        kind_dirs = [p for p in registry_dir.iterdir() if p.is_dir()]
    else:
        candidate = registry_dir / kind
        kind_dirs = [candidate] if candidate.is_dir() else []

    for kind_dir in kind_dirs:
        if kind_dir.name not in ARTIFACT_KINDS:
            # Ignore stray directories that don't correspond to a known
            # artifact kind.  This keeps `list_artifacts` from blowing up
            # when something unrelated lives under `<root>/registry/`.
            continue
        for id_dir in kind_dir.iterdir():
            if not id_dir.is_dir():
                continue
            for version_file in id_dir.iterdir():
                if not version_file.is_file() or version_file.suffix != ".json":
                    continue
                artifact = _read_envelope(version_file)
                if artifact is not None:
                    results.append(artifact)

    results.sort(key=lambda a: (a.kind, a.id, a.version))
    return results


def verify_cache(
    host_paths: HostPaths,
    registry: Any,
) -> list[tuple[CachedArtifact, str]]:
    """Compare every cached artifact against ``registry``.

    Returns a list of ``(artifact, reason)`` tuples where ``reason`` is
    ``"missing"`` (the registry no longer has that artifact) or
    ``"mismatch"`` (the registry payload differs from the cached one).
    An empty list means the cache is consistent.
    """
    get = getattr(registry, "get", None)
    if not callable(get):
        raise AgentHostError("registry object has no callable `.get(kind, id, version)`")

    discrepancies: list[tuple[CachedArtifact, str]] = []
    for artifact in list_artifacts(host_paths):
        record = get(artifact.kind, artifact.id, artifact.version)
        normalized = _coerce_record(record)
        if normalized is None:
            discrepancies.append((artifact, "missing"))
            continue
        _, _, _, payload = normalized
        if payload != artifact.payload:
            discrepancies.append((artifact, "mismatch"))
    return discrepancies


def delete_artifact(
    host_paths: HostPaths,
    *,
    kind: str,
    id: str,
    version: str,
) -> bool:
    """Remove a single cached artifact.

    Returns ``True`` iff the file existed and was removed.  Empty parent
    directories are left in place to keep this operation cheap and
    predictable.
    """
    _validate_kind(kind)
    _validate_identifier(id, label="id")
    _validate_identifier(version, label="version")
    path = cache_path(host_paths, kind, id, version)
    if not path.is_file():
        return False
    path.unlink()
    return True
