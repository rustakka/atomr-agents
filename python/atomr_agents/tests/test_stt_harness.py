"""Smoke tests for the STT-harness bindings.

Only runs after `maturin develop`. Exercises the FFI shape end-to-end
against `MockSpeechToText` so no network or models are required.

The `_native` extension is loaded directly off disk rather than via
`import atomr_agents`, so this test does not depend on the package
facade in `atomr_agents/__init__.py`.
"""

import importlib.machinery
import importlib.util
import pathlib

import pytest


def _load_native():
    # The cdylib exports `PyInit__native`, so the loader name must be
    # exactly `_native`.
    pkg_dir = pathlib.Path(__file__).resolve().parents[1]
    candidates = sorted(pkg_dir.glob("_native*.so"))
    if not candidates:
        pytest.skip("native extension not built; run `maturin develop`")
    loader = importlib.machinery.ExtensionFileLoader("_native", str(candidates[0]))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


native = _load_native()


def test_stt_harness_module_layout() -> None:
    assert hasattr(native, "stt_harness")
    sh = native.stt_harness
    for name in (
        "SttHarnessSpec",
        "SttHarness",
        "SttConversation",
        "SttTurn",
        "SttEventStream",
        "store_roundtrip",
    ):
        assert hasattr(sh, name), f"stt_harness.{name} missing"


def test_spec_accepts_diarization_and_voice_mode() -> None:
    sh = native.stt_harness
    spec = sh.SttHarnessSpec("demo", "0.2.0", "layered_mock", "live")
    assert spec.id == "demo"
    assert spec.version == "0.2.0"
    assert spec.diarization == "layered"
    assert spec.voice_mode == "live"

    with pytest.raises(ValueError):
        sh.SttHarnessSpec("bad", "0.1.0", "nonsense")


@pytest.mark.asyncio
async def test_harness_runs_and_maps_to_turn_input() -> None:
    stt = native.stt
    sh = native.stt_harness

    backend = stt.mock_speech_to_text("hello from python", "en")
    audio = stt.audio_pcm([0.0] * 16_000, 16_000, 1)
    spec = sh.SttHarnessSpec("py-demo", "0.1.0", "layered_mock", "turn_based")
    harness = sh.SttHarness(spec, backend, audio)

    conversation = await harness.run()
    assert len(conversation.turns) == 1
    turn = conversation.turns[0]
    assert turn.text == "hello from python"
    assert turn.state == "final"
    # `layered_mock` diarization attributes the utterance to a speaker.
    assert turn.speaker_id is not None

    # The conversation bridges to the agentic turn shape.
    turn_input = conversation.to_turn_input()
    assert turn_input["user"] == "hello from python"
    assert turn_input["history"] == []

    # Speaker labels are editable in place.
    conversation.rename_speaker(turn.speaker_id, "Pythonista")
    assert conversation.effective_label(turn.speaker_id) == "Pythonista"

    # And the persistence surface round-trips.
    assert sh.store_roundtrip(conversation) == conversation.id
