"""Facade over :mod:`atomr_agents._native.tts`.

Re-exports the text-to-speech surface: ``TextToSpeech``,
``Capabilities``, ``VoiceRef``, ``SynthesisRequest``, ``AudioOutput``,
``AudioChunk``, ``SynthesisStream``, ``RealtimeSession``,
``RealtimeEvent``, the ``tts_request`` / ``sfx_request`` /
``dialogue_request`` factories, the ``voice_library`` /
``voice_described`` / ``voice_cloned`` voice constructors, and the
backend constructors (``mock_tts`` plus, when their runtime crates
are linked, ``tts_openai`` / ``tts_elevenlabs`` / ``tts_piper`` / …).
"""

from ._native import tts as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
