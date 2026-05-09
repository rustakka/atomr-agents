"""Smoke tests for the speech-to-text + voice surface.

Only runs after `maturin develop`. Exercises the FFI shape end-to-end
against `MockSpeechToText` so no network or models are required.
"""

import asyncio
from pathlib import Path

import pytest

native = pytest.importorskip("atomr_agents._native")


def test_stt_module_layout() -> None:
    assert hasattr(native, "stt")
    assert hasattr(native, "voice")
    stt = native.stt
    voice = native.voice
    for name in (
        "SpeechToText",
        "Capabilities",
        "Transcript",
        "StreamingSession",
        "StreamEvent",
        "audio_file",
        "audio_bytes",
        "audio_pcm",
        "mock_speech_to_text",
        "stt_openai",
        "stt_deepgram",
        "stt_assemblyai",
        "stt_whisper",
    ):
        assert hasattr(stt, name), f"stt.{name} missing"
    for name in ("VoiceMode", "VoiceEvent", "VoiceSession"):
        assert hasattr(voice, name), f"voice.{name} missing"


def test_facade_reexports() -> None:
    from atomr_agents import stt, voice

    assert stt.mock_speech_to_text is native.stt.mock_speech_to_text
    assert voice.VoiceMode is native.voice.VoiceMode


def test_capabilities_round_trip_to_dict() -> None:
    from atomr_agents import stt

    backend = stt.mock_speech_to_text()
    caps = backend.capabilities()
    d = caps.to_dict()
    assert isinstance(d, dict)
    assert d["batch"] is True
    assert d["streaming_push"] is True
    assert d["diarization"] == "speaker_count"
    assert "supported_audio_formats" in d
    # Property accessors should agree with the dict view.
    assert caps.batch is True
    assert caps.streaming_push is True
    assert caps.diarization == "speaker_count"
    assert caps.requires_network is False


def test_backend_kind_and_transport_kind_are_strings() -> None:
    from atomr_agents import stt

    backend = stt.mock_speech_to_text()
    assert backend.backend_kind() == "mock"
    assert backend.transport_kind() == "local_model"


def test_batch_transcribe_against_mock(tmp_path: Path) -> None:
    """`mock_speech_to_text` reads the input length to derive its
    deterministic transcript text. We give it a real file so the
    file-metadata path runs."""
    from atomr_agents import stt

    audio_path = tmp_path / "fake.wav"
    audio_path.write_bytes(b"RIFF" * 8)  # 32 bytes
    backend = stt.mock_speech_to_text(text="hello there")

    async def go() -> None:
        t = await backend.transcribe(stt.audio_file(str(audio_path)), language="en")
        assert t.text == "hello there"
        assert t.language == "en"
        assert t.backend == "mock"
        d = t.to_dict()
        assert d["text"] == "hello there"
        assert d["backend"] == "mock"

    asyncio.run(go())


def test_audio_bytes_factory() -> None:
    from atomr_agents import stt

    inp = stt.audio_bytes(b"\x00\x00" * 100, "wav")
    assert inp is not None  # opaque PyAudioInput, just verify it constructs


def test_streaming_session_async_iter() -> None:
    """Open a streaming session against the mock backend, push a
    chunk, finish, then drain partial+final events via `async for`."""
    from atomr_agents import stt

    backend = stt.mock_speech_to_text(text="streamed mock")

    async def go() -> list:
        session = await backend.open_stream()
        await session.push_audio(b"chunk")
        await session.finish()
        events = []
        # Pull a bounded number to avoid hanging if the channel
        # never closes.
        async for ev in session.events():
            events.append(ev)
            if ev.kind == "final":
                break
        return events

    events = asyncio.run(go())
    kinds = [e.kind for e in events]
    assert "partial" in kinds
    assert "final" in kinds
    final = next(e for e in events if e.kind == "final")
    d = final.to_dict()
    assert d["kind"] == "final"
    assert d["segment"]["text"] == "streamed mock"


def test_voice_session_turn_based() -> None:
    """Wrap the mock streaming session in a TurnBased VoiceSession
    and verify a UserTurn is emitted."""
    from atomr_agents import stt, voice

    backend = stt.mock_speech_to_text(text="hello voice world")

    async def go() -> dict:
        session = await backend.open_stream()
        await session.push_audio(b"chunk")
        await session.finish()
        vs = voice.VoiceSession.open(session, voice.VoiceMode.turn_based(800))
        async for ev in vs.events():
            if ev.kind == "user_turn":
                return ev.to_dict()
        return {}

    d = asyncio.run(go())
    assert d["kind"] == "user_turn"
    assert d["text"] == "hello voice world"


def test_voice_mode_factories() -> None:
    from atomr_agents import voice

    live = voice.VoiceMode.live()
    assert live.kind == "live"
    assert live.silence_ms is None

    tb = voice.VoiceMode.turn_based(1000)
    assert tb.kind == "turn_based"
    assert tb.silence_ms == 1000


def test_streaming_session_consumed_by_voice() -> None:
    """After `VoiceSession.open` consumes a streaming session, the
    underlying session should refuse further `push_audio` calls."""
    from atomr_agents import stt, voice

    backend = stt.mock_speech_to_text()

    async def go() -> str:
        session = await backend.open_stream()
        voice.VoiceSession.open(session, voice.VoiceMode.live())
        try:
            await session.push_audio(b"too late")
            return "no-error"
        except RuntimeError as e:
            return str(e)

    msg = asyncio.run(go())
    assert "consumed" in msg.lower()
