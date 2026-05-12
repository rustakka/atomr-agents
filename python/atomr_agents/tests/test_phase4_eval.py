"""Smoke tests for Phase 4.1 eval bindings."""

from __future__ import annotations

from atomr_agents import eval as eval_mod


def test_verdict_factories():
    a = eval_mod.Verdict("approved")
    r = eval_mod.Verdict("rejected")
    assert a.name == "approved"
    assert r.name == "rejected"


def test_pairwise_choice_factories():
    a = eval_mod.PairwiseChoice("a")
    b = eval_mod.PairwiseChoice("b")
    assert a.name == "a"
    assert b.name == "b"


def test_in_memory_annotation_queue_constructs():
    if not hasattr(eval_mod, "in_memory_annotation_queue"):
        return
    q = eval_mod.in_memory_annotation_queue()
    assert q is not None
