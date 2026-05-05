"""Smoke test for the native module — only runs after `maturin develop`."""

import pytest

native = pytest.importorskip("atomr_agents._native")


def test_event_bus_dispatch() -> None:
    bus = native.EventBus()
    received: list = []
    bus.subscribe(lambda ev: received.append(ev.kind))
    bus.emit_tool_invoked("calc", 0, 5, True)
    assert received == ["tool_invoked"]


def test_registry_pin_and_latest() -> None:
    r = native.Registry()
    r.publish("tool_set", "ts", "0.1.0", {"name": "ts"})
    r.publish("tool_set", "ts", "0.2.0", {"name": "ts"})
    latest = r.latest("tool_set", "ts")
    assert latest is not None
    assert latest["version"] == "0.2.0"


def test_publish_gated_blocks_regression() -> None:
    r = native.Registry()
    with pytest.raises(RuntimeError):
        r.publish_gated(
            "harness",
            "h",
            "0.1.0",
            {"id": "h"},
            current_pass_rate=0.50,
            baseline_pass_rate=0.95,
            tolerance=0.05,
        )
