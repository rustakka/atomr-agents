"""Facade over :mod:`atomr_agents._native.channel`.

Channels are provider-specific messaging transports (WhatsApp, Signal,
Discord, and the in-memory provider used for tests) that carry
**threads** — long-lived conversations between a peer and a bound
:class:`~atomr_agents.callable_.Callable`.

The Python surface today exposes:

* :class:`ChannelHarness` — orchestrator with the in-memory store,
  ``attach_memory(provider)``, ``open_thread(...)``, ``send(...)``,
  ``events()``.
* :class:`InMemoryProvider` — attach as a channel and push inbound via
  ``push_inbound(peer, provider_msg_id, text)``.
* :class:`MessageContent` — ``text("...")`` /
  ``attachment(media_ref, mime, caption=None)``.
* :class:`ChannelEventStream` — async ``recv()`` iterator over event
  dicts (``{"kind": "message_received", ...}``).
* :class:`ThreadRef` — bound thread handle (``id``, ``snapshot()``).

Production providers (WhatsApp / Signal / Discord) are wired from Rust
where their concrete configuration types can be parsed safely; this
facade focuses on the in-process loop that is sufficient for tests,
worked examples, and orchestrating Python-defined Callable targets.

Example::

    import atomr_agents as aa
    from atomr_agents.channel import ChannelHarness, InMemoryProvider, MessageContent

    harness = ChannelHarness()
    provider = InMemoryProvider("memory:dev")
    await harness.attach_memory(provider)

    # Bind a Python coroutine as a Callable
    async def echo(input_, ctx):
        return {"text": f"echo: {input_['user']}"}
    callable_ = aa.callable_.from_python(echo)
    thread = await harness.open_thread("memory:dev", "alice", callable_)

    provider.push_inbound("alice", "pmid-1", "hello")
    stream = harness.events()
    while (ev := await stream.recv()) is not None:
        print(ev["kind"], ev)
        if ev["kind"] == "message_sent":
            break
"""

from ._native import channel as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
