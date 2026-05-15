"""Facade over :mod:`atomr_agents._native.stt_harness`.

Re-exports the STT-harness surface: ``SttHarnessSpec``, ``SttHarness``,
``SttConversation``, ``SttTurn``, and ``SttEventStream`` — the agentic
streaming speech-to-text pipeline that diarizes, transcribes, and
accumulates a conversation record aligned to agentic ``TurnInput``.

Example::

    from atomr_agents import stt
    from atomr_agents.stt_harness import SttHarness, SttHarnessSpec

    backend = stt.mock_speech_to_text("hello world")
    audio = stt.audio_pcm([0.0] * 16_000, 16_000, 1)
    spec = SttHarnessSpec("demo", diarization="layered_mock")
    harness = SttHarness(spec, backend, audio)
    conversation = await harness.run()
    print(conversation.turns[0].text)
"""

from ._native import stt_harness as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
