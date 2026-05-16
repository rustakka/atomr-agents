"""Facade over :mod:`atomr_agents._native.avatar` (x86_64 only).

The avatar capability gives an agent a real-time, visually rendered
embodiment — typically an Unreal Engine 5 MetaHuman driven from outside
UE via a CBOR-framed UDP Live Link stream of audio + ARKit blendshape
weights.

This module is **x86_64-only** at the deployment layer: NVIDIA
Audio2Face is x86_64+NVIDIA-GPU-only and UE5's tooling pipeline is
x86_64-first. On aarch64 wheels the native submodule is absent and
:obj:`__all__` is an empty list — so ``import atomr_agents.avatar``
itself still works, it just exposes nothing.

Typical use::

    from atomr_agents.avatar import AvatarHarness, CapturingSink

    # The inference callable should be ``async def fn(batch_dict) -> str``
    # that routes through atomr-infer (or any LLM client you wrap).
    async def my_inference(batch):
        # ... call atomr-infer here ...
        return '{"response_text": "Hello!", "emotion_delta": {"valence": 0.4}}'

    from atomr_agents.tts import TextToSpeech  # your TTS backend
    tts = TextToSpeech.mock()
    harness = AvatarHarness(my_inference, tts, "alloy")

    sink = CapturingSink()
    await harness.attach_sink(sink.as_sink())
    await harness.user_said("hello there")
    frames = await sink.drain()
    print(len(frames), "frames captured")

The :class:`LiveLinkSink` factory is only present when the wheel was
built with the ``avatar-livelink`` cargo feature.
"""

from __future__ import annotations

try:
    from ._native import avatar as _sub  # type: ignore[attr-defined]
except (ImportError, AttributeError):  # arm64 wheel, or feature disabled
    _sub = None

if _sub is not None:
    globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
    __all__ = sorted(k for k in dir(_sub) if not k.startswith("_"))
else:
    __all__ = []


def is_available() -> bool:
    """Returns True if the native avatar submodule is loaded — i.e. the
    wheel was built with ``--features avatar`` (or ``avatar-livelink`` /
    ``avatar-a2f``) AND the host is x86_64.

    Callers can branch on this to skip avatar features cleanly on arm64
    without an ``AttributeError`` at import time.
    """
    return _sub is not None
