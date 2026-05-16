"""Branching / checkpoints for the agent-host runtime (M10).

Each agent gets a ``state/checkpoints/`` directory on disk where the host
persists append-only JSON snapshots of "working memory" + the last-seen
thread head, namespaced by *branch id*. Forking a branch copies the
latest snapshot of the source branch under a new id; switching points
the ``CURRENT`` pointer file at a branch.

This module is intentionally minimal and pure-Python: the host neither
inspects nor mutates the working-memory dict it persists — callers
(curator / chat callable) own its shape. The host just writes, lists,
reads, prunes, and diffs.

On-disk layout::

    state/checkpoints/
    ├── CURRENT                 # optional, single line: <branch-id>
    ├── main/
    │   ├── 1715800000000.json
    │   └── 1715800010000.json
    └── experiment-foo/
        └── 1715800020000.json
"""

from __future__ import annotations

import json
import re
import shutil
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from .errors import AgentSpecError
from .layout import AgentPaths

__all__ = [
    "DEFAULT_BRANCH",
    "Checkpoint",
    "branch_dir",
    "current_branch",
    "delete_branch",
    "diff_branches",
    "fork_branch",
    "latest_checkpoint",
    "list_branches",
    "list_checkpoints",
    "prune_branch",
    "switch_branch",
    "write_checkpoint",
]


DEFAULT_BRANCH = "main"

_BRANCH_ID_RE = re.compile(r"^[A-Za-z0-9_.\-]+$")
_CURRENT_FILE = "CURRENT"


@dataclass(frozen=True)
class Checkpoint:
    """A single on-disk checkpoint snapshot."""

    branch_id: str
    agent_id: str
    ts_ms: int
    path: Path
    working_memory: dict
    thread_head: dict | None = None
    parent_branch: str | None = None


# ---------- helpers ----------------------------------------------------------


def _safe_id(value: str) -> str:
    """Sanitize a branch/thread id for filesystem use (M2-style)."""
    return value.replace(":", "__").replace("/", "-").replace("\\", "-")


def _validate_branch_id(branch_id: str) -> str:
    """Reject empty / ``.`` / ``..`` / unsafe characters; return the id."""
    if not isinstance(branch_id, str) or not branch_id:
        raise AgentSpecError("branch id must be a non-empty string")
    if branch_id in {".", ".."}:
        raise AgentSpecError(f"branch id {branch_id!r} is reserved")
    if not _BRANCH_ID_RE.match(branch_id):
        raise AgentSpecError(
            f"branch id {branch_id!r} must match [A-Za-z0-9_.-]+"
        )
    return branch_id


def branch_dir(agent_paths: AgentPaths, branch_id: str) -> Path:
    """Return ``state/checkpoints/<safe-branch-id>/`` (not created)."""
    _validate_branch_id(branch_id)
    return agent_paths.checkpoints_dir / _safe_id(branch_id)


def _ckpt_filename(ts_ms: int) -> str:
    return f"{int(ts_ms)}.json"


def _parse_ts_from_name(name: str) -> int | None:
    if not name.endswith(".json"):
        return None
    stem = name[: -len(".json")]
    try:
        return int(stem)
    except ValueError:
        return None


def _read_checkpoint_file(path: Path) -> Checkpoint:
    raw = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        raise AgentSpecError(f"checkpoint {path} is not a JSON object")
    wm = raw.get("working_memory")
    if not isinstance(wm, dict):
        wm = {}
    th = raw.get("thread_head")
    if th is not None and not isinstance(th, dict):
        th = None
    pb = raw.get("parent_branch")
    if pb is not None and not isinstance(pb, str):
        pb = None
    branch_id = raw.get("branch_id")
    agent_id = raw.get("agent_id")
    ts_ms = raw.get("ts_ms")
    if not isinstance(branch_id, str) or not isinstance(agent_id, str):
        # fall back to inferring from path
        branch_id = branch_id if isinstance(branch_id, str) else path.parent.name
        agent_id = agent_id if isinstance(agent_id, str) else ""
    if not isinstance(ts_ms, int):
        parsed = _parse_ts_from_name(path.name)
        ts_ms = parsed if parsed is not None else 0
    return Checkpoint(
        branch_id=branch_id,
        agent_id=agent_id,
        ts_ms=int(ts_ms),
        path=path,
        working_memory=wm,
        thread_head=th,
        parent_branch=pb,
    )


# ---------- listing ----------------------------------------------------------


def list_branches(agent_paths: AgentPaths) -> list[str]:
    """List branch ids under ``state/checkpoints/``, sorted.

    Hidden directories (``.staging``, anything starting with ``.``) are
    skipped, as are non-directory entries.
    """
    base = agent_paths.checkpoints_dir
    if not base.is_dir():
        return []
    out: list[str] = []
    for child in base.iterdir():
        if not child.is_dir():
            continue
        if child.name.startswith("."):
            continue
        out.append(child.name)
    return sorted(out)


def list_checkpoints(
    agent_paths: AgentPaths, branch_id: str
) -> list[Checkpoint]:
    """Return checkpoints for ``branch_id`` oldest → newest by ``ts_ms``.

    Returns an empty list when the branch dir does not exist or has no
    checkpoint files.
    """
    bdir = branch_dir(agent_paths, branch_id)
    if not bdir.is_dir():
        return []
    entries: list[tuple[int, Path]] = []
    for child in bdir.iterdir():
        if not child.is_file():
            continue
        ts = _parse_ts_from_name(child.name)
        if ts is None:
            continue
        entries.append((ts, child))
    entries.sort(key=lambda pair: pair[0])
    return [_read_checkpoint_file(p) for _, p in entries]


def latest_checkpoint(
    agent_paths: AgentPaths, branch_id: str
) -> Checkpoint | None:
    """Return the newest checkpoint on ``branch_id`` (by ``ts_ms``), or None."""
    ckpts = list_checkpoints(agent_paths, branch_id)
    if not ckpts:
        return None
    return ckpts[-1]


# ---------- write ------------------------------------------------------------


def write_checkpoint(
    agent_paths: AgentPaths,
    branch_id: str,
    *,
    working_memory: Mapping[str, Any],
    thread_head: Mapping[str, Any] | None = None,
    parent_branch: str | None = None,
    ts_ms: int | None = None,
) -> Checkpoint:
    """Write a JSON snapshot under ``state/checkpoints/<branch>/<ts>.json``.

    ``ts_ms`` defaults to ``int(time.time() * 1000)``. If a file with that
    name already exists (e.g. two writes inside the same millisecond), we
    bump by one so each call produces a distinct file. The branch
    directory is created on demand.
    """
    _validate_branch_id(branch_id)
    if parent_branch is not None:
        _validate_branch_id(parent_branch)

    bdir = branch_dir(agent_paths, branch_id)
    bdir.mkdir(parents=True, exist_ok=True)

    if ts_ms is None:
        ts_ms = int(time.time() * 1000)
    ts_ms = int(ts_ms)
    target = bdir / _ckpt_filename(ts_ms)
    while target.exists():
        ts_ms += 1
        target = bdir / _ckpt_filename(ts_ms)

    wm_dict = dict(working_memory)
    th_dict = dict(thread_head) if thread_head is not None else None
    payload: dict[str, Any] = {
        "branch_id": branch_id,
        "agent_id": agent_paths.agent_id,
        "ts_ms": ts_ms,
        "working_memory": wm_dict,
        "thread_head": th_dict,
        "parent_branch": parent_branch,
    }
    target.write_text(
        json.dumps(payload, ensure_ascii=False, sort_keys=True),
        encoding="utf-8",
    )
    return Checkpoint(
        branch_id=branch_id,
        agent_id=agent_paths.agent_id,
        ts_ms=ts_ms,
        path=target,
        working_memory=wm_dict,
        thread_head=th_dict,
        parent_branch=parent_branch,
    )


# ---------- forking / switching ---------------------------------------------


def fork_branch(
    agent_paths: AgentPaths,
    *,
    source_branch: str = DEFAULT_BRANCH,
    new_branch: str,
) -> Checkpoint:
    """Copy the latest checkpoint of ``source_branch`` into ``new_branch``.

    Only the *latest* snapshot is copied — history stays on the source
    branch. ``parent_branch`` on the new checkpoint is set to
    ``source_branch``. Raises :class:`AgentSpecError` when the source has
    no checkpoints to fork from.
    """
    _validate_branch_id(source_branch)
    _validate_branch_id(new_branch)
    src = latest_checkpoint(agent_paths, source_branch)
    if src is None:
        raise AgentSpecError(
            f"cannot fork: source branch {source_branch!r} has no checkpoints"
        )
    return write_checkpoint(
        agent_paths,
        new_branch,
        working_memory=src.working_memory,
        thread_head=src.thread_head,
        parent_branch=source_branch,
    )


def _current_path(agent_paths: AgentPaths) -> Path:
    return agent_paths.checkpoints_dir / _CURRENT_FILE


def switch_branch(agent_paths: AgentPaths, branch_id: str) -> Checkpoint:
    """Point ``CURRENT`` at ``branch_id`` and return its latest checkpoint.

    Raises :class:`AgentSpecError` when the branch has no checkpoints.
    """
    _validate_branch_id(branch_id)
    ckpt = latest_checkpoint(agent_paths, branch_id)
    if ckpt is None:
        raise AgentSpecError(
            f"cannot switch: branch {branch_id!r} has no checkpoints"
        )
    base = agent_paths.checkpoints_dir
    base.mkdir(parents=True, exist_ok=True)
    _current_path(agent_paths).write_text(branch_id + "\n", encoding="utf-8")
    return ckpt


def current_branch(agent_paths: AgentPaths) -> str:
    """Read the ``CURRENT`` pointer; default to :data:`DEFAULT_BRANCH`."""
    p = _current_path(agent_paths)
    if not p.is_file():
        return DEFAULT_BRANCH
    raw = p.read_text(encoding="utf-8").strip()
    if not raw:
        return DEFAULT_BRANCH
    return raw


# ---------- diff -------------------------------------------------------------


def diff_branches(
    agent_paths: AgentPaths, branch_a: str, branch_b: str
) -> dict[str, Any]:
    """Compare the latest checkpoint of ``branch_a`` vs ``branch_b``.

    Only top-level keys of ``working_memory`` are compared — nested
    structures are returned as-is in ``changed_keys`` entries.

    ``thread_head_diff`` is one of:
        - ``"same"``           — both branches have an equal head dict (or both are None)
        - ``"different"``      — both have heads, but they differ
        - ``"missing_a"``      — branch_a has no thread_head
        - ``"missing_b"``      — branch_b has no thread_head
    """
    _validate_branch_id(branch_a)
    _validate_branch_id(branch_b)
    a = latest_checkpoint(agent_paths, branch_a)
    b = latest_checkpoint(agent_paths, branch_b)
    wm_a = a.working_memory if a is not None else {}
    wm_b = b.working_memory if b is not None else {}
    keys_a = set(wm_a.keys())
    keys_b = set(wm_b.keys())
    added = sorted(keys_b - keys_a)
    removed = sorted(keys_a - keys_b)
    changed: list[dict[str, Any]] = []
    for key in sorted(keys_a & keys_b):
        if wm_a[key] != wm_b[key]:
            changed.append({"key": key, "a": wm_a[key], "b": wm_b[key]})

    th_a = a.thread_head if a is not None else None
    th_b = b.thread_head if b is not None else None
    if th_a is None and th_b is None:
        th_diff = "same"
    elif th_a is None:
        th_diff = "missing_a"
    elif th_b is None:
        th_diff = "missing_b"
    elif th_a == th_b:
        th_diff = "same"
    else:
        th_diff = "different"

    return {
        "added_keys": added,
        "removed_keys": removed,
        "changed_keys": changed,
        "thread_head_diff": th_diff,
    }


# ---------- pruning / deletion ----------------------------------------------


def prune_branch(
    agent_paths: AgentPaths, branch_id: str, *, keep: int = 10
) -> int:
    """Delete oldest checkpoints until at most ``keep`` remain.

    Returns the count of files removed. ``keep`` must be >= 0.
    """
    if keep < 0:
        raise AgentSpecError("keep must be >= 0")
    _validate_branch_id(branch_id)
    ckpts = list_checkpoints(agent_paths, branch_id)
    if len(ckpts) <= keep:
        return 0
    to_delete = ckpts[: len(ckpts) - keep]
    removed = 0
    for ck in to_delete:
        try:
            ck.path.unlink()
            removed += 1
        except FileNotFoundError:
            pass
    return removed


def delete_branch(
    agent_paths: AgentPaths, branch_id: str, *, force: bool = False
) -> None:
    """Remove the branch directory entirely.

    Refuses to delete :data:`DEFAULT_BRANCH` unless ``force=True``. A
    missing directory is silently treated as a no-op.
    """
    _validate_branch_id(branch_id)
    if branch_id == DEFAULT_BRANCH and not force:
        raise AgentSpecError(
            f"refusing to delete default branch {branch_id!r} without force=True"
        )
    bdir = branch_dir(agent_paths, branch_id)
    if bdir.is_dir():
        shutil.rmtree(bdir)
