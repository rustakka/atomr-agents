"""Smoke tests for the avatar harness Python bindings.

Run after `maturin develop --features avatar` (or
`--features avatar-livelink`). On arm64 wheels the native avatar
submodule is absent and the whole test module is skipped.
"""

from __future__ import annotations

import asyncio
import importlib.machinery
import importlib.util
import pathlib
import platform
import sys

import pytest

if platform.machine() not in {"x86_64", "AMD64"}:
    pytest.skip(
        "avatar capability is x86_64-only (UE5 + NVIDIA Audio2Face deployment matrix)",
        allow_module_level=True,
    )


def _load_native():
    pkg_dir = pathlib.Path(__file__).resolve().parents[1]
    tag = f"cpython-{sys.version_info.major}{sys.version_info.minor}"
    candidates = sorted(p for p in pkg_dir.glob("_native*.so") if tag in p.name)
    if not candidates:
        candidates = sorted(pkg_dir.glob("_native*.so"))
    if not candidates:
        pytest.skip("native extension not built; run `maturin develop --features avatar`")
    loader = importlib.machinery.ExtensionFileLoader("_native", str(candidates[-1]))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


native = _load_native()

if not hasattr(native, "avatar"):
    pytest.skip(
        "wheel built without `--features avatar`; rebuild with the feature enabled",
        allow_module_level=True,
    )


def test_avatar_module_layout() -> None:
    av = native.avatar
    for name in (
        "AvatarHarness",
        "AvatarFrame",
        "AvatarSink",
        "CapturingSink",
    ):
        assert hasattr(av, name), f"missing {name}"


def test_capturing_sink_kind() -> None:
    sink = native.avatar.CapturingSink()
    assert sink.as_sink().kind() == "mock_capture"


def test_harness_drives_pipeline_end_to_end() -> None:
    """User utterance → cognition (scripted stub) → mock TTS → sync →
    capturing sink emits at least one frame."""

    async def go():
        async def stub_inference(batch_dict):  # noqa: ARG001
            return '{"response_text": "Hi.", "emotion_delta": {"valence": 0.6}}'

        tts = native.tts.TextToSpeech.mock()
        harness = native.avatar.AvatarHarness(
            stub_inference, tts, "mock", frame_rate=30
        )
        sink = native.avatar.CapturingSink()
        await harness.attach_sink(sink.as_sink())
        await harness.user_said("hello")

        # Poll for frames — pipeline is async; cap at ~3s.
        for _ in range(60):
            frames = await sink.drain()
            if frames:
                break
            await asyncio.sleep(0.05)
        else:
            frames = []
        assert frames, "no frames emitted"
        # First frame should have a timecode and weights.
        first = frames[0]
        assert first.timecode() == "00:00:00:00"
        ws = first.weights()
        assert len(ws) == 52

        intent = await harness.last_intent()
        assert intent is not None
        assert intent["response_text"] == "Hi."

        emo = harness.emotion()
        assert emo["valence"] > 0.0

        await harness.shutdown()

    asyncio.run(go())


def test_speak_text_bypasses_cognition() -> None:
    async def go():
        async def never_called(batch_dict):  # noqa: ARG001
            raise AssertionError("inference should not run for speak_text")

        tts = native.tts.TextToSpeech.mock()
        harness = native.avatar.AvatarHarness(
            never_called, tts, "mock", frame_rate=30
        )
        sink = native.avatar.CapturingSink()
        await harness.attach_sink(sink.as_sink())

        await harness.speak_text("canned line")
        # Give the sink a beat to drain.
        await asyncio.sleep(0.1)
        frames = await sink.drain()
        assert frames

        await harness.shutdown()

    asyncio.run(go())
