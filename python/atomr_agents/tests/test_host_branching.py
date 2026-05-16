"""Tests for the agent-host branching/checkpoints module (M10).

Pure-Python, no ``_native`` dependency. Each test sets up an agent
directory under ``tmp_path`` with ``AgentPaths.ensure()`` so the layout
exists, then exercises the branching API directly.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from atomr_agents.agent_host.branching import (
    DEFAULT_BRANCH,
    branch_dir,
    current_branch,
    delete_branch,
    diff_branches,
    fork_branch,
    latest_checkpoint,
    list_branches,
    list_checkpoints,
    prune_branch,
    switch_branch,
    write_checkpoint,
)
from atomr_agents.agent_host.errors import AgentSpecError
from atomr_agents.agent_host.layout import AgentPaths


@pytest.fixture
def paths(tmp_path: Path) -> AgentPaths:
    p = AgentPaths(root=tmp_path, agent_id="alpha")
    p.ensure()
    return p


# 1. write_checkpoint produces a parseable JSON file at the expected path.
def test_write_checkpoint_writes_parseable_json(paths: AgentPaths) -> None:
    ckpt = write_checkpoint(
        paths,
        DEFAULT_BRANCH,
        working_memory={"hello": "world", "count": 1},
        thread_head={"role": "user", "content": "hi"},
        ts_ms=1_700_000_000_000,
    )
    expected = paths.checkpoints_dir / DEFAULT_BRANCH / "1700000000000.json"
    assert ckpt.path == expected
    assert expected.is_file()
    raw = json.loads(expected.read_text(encoding="utf-8"))
    assert raw["branch_id"] == DEFAULT_BRANCH
    assert raw["agent_id"] == "alpha"
    assert raw["ts_ms"] == 1_700_000_000_000
    assert raw["working_memory"] == {"hello": "world", "count": 1}
    assert raw["thread_head"] == {"role": "user", "content": "hi"}
    assert raw["parent_branch"] is None


# 2. list_checkpoints returns checkpoints sorted by ts.
def test_list_checkpoints_sorted_by_ts(paths: AgentPaths) -> None:
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"n": 1}, ts_ms=2000)
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"n": 2}, ts_ms=1000)
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"n": 3}, ts_ms=3000)
    ckpts = list_checkpoints(paths, DEFAULT_BRANCH)
    assert [c.ts_ms for c in ckpts] == [1000, 2000, 3000]
    assert [c.working_memory["n"] for c in ckpts] == [2, 1, 3]


# 3. latest_checkpoint returns the newest.
def test_latest_checkpoint_returns_newest(paths: AgentPaths) -> None:
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"x": "a"}, ts_ms=100)
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"x": "b"}, ts_ms=200)
    latest = latest_checkpoint(paths, DEFAULT_BRANCH)
    assert latest is not None
    assert latest.ts_ms == 200
    assert latest.working_memory == {"x": "b"}


# 4. latest_checkpoint returns None for a branch with no snapshots.
def test_latest_checkpoint_none_when_empty(paths: AgentPaths) -> None:
    assert latest_checkpoint(paths, DEFAULT_BRANCH) is None
    assert latest_checkpoint(paths, "does-not-exist") is None


# 5. list_branches after writing to two branches returns both ids sorted.
def test_list_branches_two_branches_sorted(paths: AgentPaths) -> None:
    write_checkpoint(paths, "zeta", working_memory={"a": 1}, ts_ms=1)
    write_checkpoint(paths, "alpha", working_memory={"a": 2}, ts_ms=2)
    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"a": 3}, ts_ms=3)
    # Hidden directories must not be reported.
    (paths.checkpoints_dir / ".staging").mkdir()
    assert list_branches(paths) == ["alpha", DEFAULT_BRANCH, "zeta"]


# 6. fork_branch copies the latest source snapshot into a new branch; new
#    branch's working_memory equals the source's, parent_branch == source.
def test_fork_branch_copies_latest_and_sets_parent(paths: AgentPaths) -> None:
    write_checkpoint(
        paths, DEFAULT_BRANCH, working_memory={"v": 1}, ts_ms=100
    )
    write_checkpoint(
        paths, DEFAULT_BRANCH, working_memory={"v": 2}, ts_ms=200,
        thread_head={"role": "assistant", "content": "hi"},
    )
    forked = fork_branch(
        paths, source_branch=DEFAULT_BRANCH, new_branch="experiment"
    )
    assert forked.branch_id == "experiment"
    assert forked.working_memory == {"v": 2}
    assert forked.thread_head == {"role": "assistant", "content": "hi"}
    assert forked.parent_branch == DEFAULT_BRANCH
    # Only one snapshot lives on the forked branch — history was not copied.
    assert len(list_checkpoints(paths, "experiment")) == 1


# 7. fork_branch on an empty source raises AgentSpecError.
def test_fork_branch_empty_source_raises(paths: AgentPaths) -> None:
    with pytest.raises(AgentSpecError):
        fork_branch(paths, source_branch=DEFAULT_BRANCH, new_branch="x")


# 8. switch_branch writes CURRENT, current_branch reads it back.
def test_switch_and_current_branch_roundtrip(paths: AgentPaths) -> None:
    write_checkpoint(paths, "feature-a", working_memory={"q": 1}, ts_ms=1)
    ckpt = switch_branch(paths, "feature-a")
    assert ckpt.branch_id == "feature-a"
    current_file = paths.checkpoints_dir / "CURRENT"
    assert current_file.is_file()
    assert current_file.read_text(encoding="utf-8").strip() == "feature-a"
    assert current_branch(paths) == "feature-a"


# 9. current_branch defaults to "main" when CURRENT is missing.
def test_current_branch_defaults_to_main(paths: AgentPaths) -> None:
    assert current_branch(paths) == DEFAULT_BRANCH
    # Also: an empty CURRENT file falls back to the default.
    (paths.checkpoints_dir / "CURRENT").write_text("", encoding="utf-8")
    assert current_branch(paths) == DEFAULT_BRANCH


# 10. diff_branches reports added/removed/changed keys + thread_head_diff.
def test_diff_branches_reports_keys_and_thread_diff(paths: AgentPaths) -> None:
    write_checkpoint(
        paths, "a",
        working_memory={"shared": 1, "only_a": True, "changed": "old"},
        thread_head={"role": "user", "content": "hello"},
        ts_ms=10,
    )
    write_checkpoint(
        paths, "b",
        working_memory={"shared": 1, "only_b": "yes", "changed": "new"},
        thread_head={"role": "user", "content": "world"},
        ts_ms=20,
    )
    diff = diff_branches(paths, "a", "b")
    assert diff["added_keys"] == ["only_b"]
    assert diff["removed_keys"] == ["only_a"]
    assert diff["changed_keys"] == [
        {"key": "changed", "a": "old", "b": "new"}
    ]
    assert diff["thread_head_diff"] == "different"

    # And when only one side has a thread_head, we report missing on the other.
    write_checkpoint(paths, "c", working_memory={}, ts_ms=30)
    diff_missing = diff_branches(paths, "a", "c")
    assert diff_missing["thread_head_diff"] == "missing_b"
    diff_missing_a = diff_branches(paths, "c", "a")
    assert diff_missing_a["thread_head_diff"] == "missing_a"


# 11. prune_branch deletes oldest checkpoints, leaves `keep` newest.
def test_prune_branch_keeps_newest(paths: AgentPaths) -> None:
    for ts in range(1, 6):  # 5 snapshots
        write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"i": ts}, ts_ms=ts)
    removed = prune_branch(paths, DEFAULT_BRANCH, keep=2)
    assert removed == 3
    remaining = list_checkpoints(paths, DEFAULT_BRANCH)
    assert [c.ts_ms for c in remaining] == [4, 5]
    # Pruning again with keep >= len is a no-op.
    assert prune_branch(paths, DEFAULT_BRANCH, keep=10) == 0


# 12. delete_branch removes the dir; refuses `main` without force.
def test_delete_branch_removes_dir_and_protects_main(paths: AgentPaths) -> None:
    write_checkpoint(paths, "scratch", working_memory={"k": 1}, ts_ms=1)
    assert branch_dir(paths, "scratch").is_dir()
    delete_branch(paths, "scratch")
    assert not branch_dir(paths, "scratch").exists()

    write_checkpoint(paths, DEFAULT_BRANCH, working_memory={"k": 1}, ts_ms=1)
    with pytest.raises(AgentSpecError):
        delete_branch(paths, DEFAULT_BRANCH)
    assert branch_dir(paths, DEFAULT_BRANCH).is_dir()
    delete_branch(paths, DEFAULT_BRANCH, force=True)
    assert not branch_dir(paths, DEFAULT_BRANCH).exists()


# 13. Branch-id validation: empty / ".." / special chars raise AgentSpecError.
@pytest.mark.parametrize("bad", ["", ".", "..", "has space", "has/slash", "weird:colon", "x*y"])
def test_branch_id_validation_rejects_unsafe(paths: AgentPaths, bad: str) -> None:
    with pytest.raises(AgentSpecError):
        write_checkpoint(paths, bad, working_memory={}, ts_ms=1)
