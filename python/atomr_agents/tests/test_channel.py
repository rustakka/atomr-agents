"""Smoke tests for the channel harness Python bindings.

Run after `maturin develop`. Exercises the FFI shape and the
end-to-end in-memory loop: inbound → bound `Callable` → outbound.
"""

import asyncio
import importlib.machinery
import importlib.util
import pathlib
import sys

import pytest


def _load_native():
    pkg_dir = pathlib.Path(__file__).resolve().parents[1]
    tag = f"cpython-{sys.version_info.major}{sys.version_info.minor}"
    candidates = sorted(
        p for p in pkg_dir.glob("_native*.so") if tag in p.name
    )
    if not candidates:
        candidates = sorted(pkg_dir.glob("_native*.so"))
    if not candidates:
        pytest.skip("native extension not built; run `maturin develop`")
    loader = importlib.machinery.ExtensionFileLoader("_native", str(candidates[-1]))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


native = _load_native()


def test_channel_module_layout() -> None:
    assert hasattr(native, "channel")
    ch = native.channel
    for name in (
        "ChannelHarness",
        "ChannelEventStream",
        "InMemoryProvider",
        "MessageContent",
        "ThreadRef",
    ):
        assert hasattr(ch, name), f"missing {name}"


def test_message_content_helpers() -> None:
    text = native.channel.MessageContent.text("hi")
    assert text.kind == "text"
    assert text.as_text == "hi"

    att = native.channel.MessageContent.attachment("ref-1", "image/png", "look")
    assert att.kind == "attachment"


def test_in_memory_provider_push_inbound() -> None:
    # Should not raise even before the provider is attached — the inbox
    # buffer accepts messages until the start task drains them.
    p = native.channel.InMemoryProvider("memory:py")
    assert p.channel_id == "memory:py"
    p.push_inbound("alice", "pmid-x", "preroll")


def test_end_to_end_inbound_invokes_callable_and_outbound_records() -> None:
    """Build a harness, bind a Python callable, push inbound, expect
    a sent event and a persisted outbound record."""

    async def go():
        harness = native.channel.ChannelHarness()
        provider = native.channel.InMemoryProvider("memory:py-e2e")
        await harness.attach_memory(provider)

        # Use a sync callable — the harness inbound loop runs on tokio
        # worker threads where Python's asyncio event loop isn't current,
        # so an `async def` target would need extra plumbing to await.
        def echo(input_, ctx):  # noqa: ARG001
            return {"text": "echo: " + (input_.get("user") or "")}

        echo_callable = native.callable.Callable.from_callable(echo, "py_echo")
        thread = await harness.open_thread(
            "memory:py-e2e", "alice", echo_callable
        )
        assert thread.id

        stream = harness.events()
        provider.push_inbound("alice", "pmid-1", "hello")

        # Walk events until we see message_sent. The harness emits a
        # bounded number of events per turn — cap the loop accordingly.
        saw_received = False
        saw_sent = False
        for _ in range(20):
            ev = await stream.recv()
            if ev is None:
                break
            if ev["kind"] == "message_received":
                saw_received = True
            elif ev["kind"] == "message_sent":
                saw_sent = True
                break
        assert saw_received, "MessageReceived not observed"
        assert saw_sent, "MessageSent not observed"

        records = await harness.list_messages(thread.id, 0)
        assert len(records) >= 2, records

        await harness.shutdown()

    asyncio.run(go())


def test_admin_send_via_harness() -> None:
    async def go():
        harness = native.channel.ChannelHarness()
        provider = native.channel.InMemoryProvider("memory:py-admin")
        await harness.attach_memory(provider)

        def echo(input_, ctx):  # noqa: ARG001
            return {"text": "x"}

        callable_ = native.callable.Callable.from_callable(echo, "py_echo")
        thread = await harness.open_thread(
            "memory:py-admin", "bob", callable_
        )

        ack = await harness.send(
            thread.id, native.channel.MessageContent.text("manual")
        )
        assert ack["provider_msg_id"]

        await harness.shutdown()

    asyncio.run(go())
