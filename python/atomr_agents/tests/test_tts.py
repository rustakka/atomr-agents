"""Smoke tests for the text-to-speech + realtime surface.

Only runs after `maturin develop`. Exercises the FFI shape end-to-end
against `MockTextToSpeech` so no network or models are required.
"""

import asyncio

import pytest

native = pytest.importorskip("atomr_agents._native")


def test_tts_module_layout() -> None:
    assert hasattr(native, "tts")
    tts = native.tts
    for name in (
        "TextToSpeech",
        "Capabilities",
        "VoiceRef",
        "SynthesisRequest",
        "AudioOutput",
        "AudioChunk",
        "SynthesisStream",
        "RealtimeEvent",
        "RealtimeSession",
        "voice_library",
        "voice_described",
        "voice_cloned",
        "tts_request",
        "sfx_request",
        "dialogue_request",
        "mock_tts",
    ):
        assert hasattr(tts, name), f"tts.{name} missing"


def test_facade_reexports() -> None:
    from atomr_agents import tts

    assert tts.mock_tts is native.tts.mock_tts
    assert tts.tts_request is native.tts.tts_request


def test_capabilities_round_trip_to_dict() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    caps = backend.capabilities()
    d = caps.to_dict()
    assert isinstance(d, dict)
    assert d["plain_tts"] is True
    assert d["voicegen_from_text"] is True
    assert d["sound_effects"] is True
    assert d["realtime_bidirectional"] is True
    assert d["voice_cloning"]["kind"] == "zero_shot"
    assert d["dialogue_multispeaker"] == 5

    # Property accessors should agree.
    assert caps.plain_tts is True
    assert caps.realtime_bidirectional is True
    assert caps.requires_network is False


def test_backend_kind_and_transport_kind_are_strings() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    assert backend.backend_kind() == "mock"
    assert backend.transport_kind() == "local_model"


def test_batch_synthesize_returns_audio_output() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    req = tts.tts_request("Hello world from atomr-agents", voice=tts.voice_library("alpha"))

    async def go():
        return await backend.synthesize(req)

    out = asyncio.run(go())
    assert out.backend == "mock"
    assert out.voice_id_used == "alpha"
    assert out.duration_secs > 0.0
    assert out.characters_processed > 0
    samples = out.samples()
    assert isinstance(samples, list)
    assert len(samples) > 0
    assert out.sample_rate == 16000


def test_streaming_synthesis_async_iter() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    req = tts.tts_request(
        "Streaming TTS test sentence with a few words",
        voice=tts.voice_library("alpha"),
    )

    async def go():
        s = await backend.synthesize_stream(req)
        chunks = []
        async for c in s.events():
            chunks.append((c.seq, c.is_final, len(c.bytes())))
            if c.is_final:
                break
        return chunks

    chunks = asyncio.run(go())
    assert len(chunks) > 0
    assert chunks[-1][1] is True  # last chunk is_final
    assert all(b > 0 or final for (_, final, b) in chunks[:-1])


def test_sfx_request_constructs_and_synthesizes() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    req = tts.sfx_request("rain falling on a tin roof", duration_secs=5.0)

    async def go():
        return await backend.synthesize(req)

    out = asyncio.run(go())
    assert out.backend == "mock"
    assert out.duration_secs > 0.0


def test_dialogue_request_constructs_and_synthesizes() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()
    req = tts.dialogue_request(
        script=[("S1", "Hello there!"), ("S2", "Hi back, friend.")],
        speakers=[
            ("S1", tts.voice_library("alpha")),
            ("S2", tts.voice_library("beta")),
        ],
    )

    async def go():
        return await backend.synthesize(req)

    out = asyncio.run(go())
    assert out.duration_secs > 0.0
    assert out.characters_processed > 0


def test_voice_factories() -> None:
    from atomr_agents import tts, stt

    lib = tts.voice_library("alpha")
    desc = tts.voice_described("warm and slow with a touch of vibrato")
    audio_in = stt.audio_bytes(b"\x00\x00" * 100, "wav")
    cloned = tts.voice_cloned(audio_in)
    # Just ensure all three constructors return non-None.
    assert lib is not None
    assert desc is not None
    assert cloned is not None


def test_realtime_session_round_trip() -> None:
    from atomr_agents import tts

    backend = tts.mock_tts()

    async def go():
        s = await backend.open_realtime(voice_id="alpha", instructions="be brief")
        await s.push_text("hello")
        seen = []
        async for ev in s.events():
            seen.append(ev.kind)
            if ev.kind == "done":
                break
        return seen

    kinds = asyncio.run(go())
    assert "outbound_text" in kinds
    assert "audio_out" in kinds
    assert "done" in kinds


def test_backend_constructors_instantiate() -> None:
    """All eight TTS backend constructors should succeed without
    contacting the network or loading any model. Each should report
    its `BackendKind` correctly."""
    from atomr_agents import tts

    cases = [
        ("openai",          lambda: tts.tts_openai("sk-test", model="tts-1-hd")),
        ("elevenlabs",      lambda: tts.tts_elevenlabs("xi-test", voice="rachel")),
        ("openai_realtime", lambda: tts.tts_openai_realtime("sk-test", voice="alloy")),
        ("gemini_live",     lambda: tts.tts_gemini_live("ya29.test", voice="Puck")),
        ("piper", lambda: tts.tts_piper(voices=[
            {"id": "en-us", "onnx_path": "/tmp/x.onnx",
             "config_path": "/tmp/x.json", "language": "en-us"}
        ])),
        ("kokoro",  lambda: tts.tts_kokoro("/tmp/k.onnx", "/tmp/v.bin",
                                          default_voice="af_alloy")),
        ("moss_tts", lambda: tts.tts_moss(model_variant="local_1_7b")),
        ("xtts_v2",  lambda: tts.tts_xtts(default_language="en")),
    ]
    for expected, ctor in cases:
        backend = ctor()
        assert backend.backend_kind() == expected, (
            f"{expected}: got {backend.backend_kind()!r}"
        )
        # capabilities() must return a dict-able object even before
        # any model is loaded.
        d = backend.capabilities().to_dict()
        assert isinstance(d, dict)
        assert "plain_tts" in d


def test_secret_ref_env_prefix_resolves() -> None:
    """`api_key='env:VARNAME'` should defer key lookup to the env var."""
    import os
    from atomr_agents import tts

    os.environ["ATOMR_FAKE_TTS_KEY"] = "sk-from-env"
    try:
        backend = tts.tts_openai("env:ATOMR_FAKE_TTS_KEY")
        assert backend.backend_kind() == "openai"
    finally:
        os.environ.pop("ATOMR_FAKE_TTS_KEY", None)
