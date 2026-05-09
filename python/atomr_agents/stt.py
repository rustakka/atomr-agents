"""Facade over :mod:`atomr_agents._native.stt`.

Re-exports the speech-to-text surface: ``SpeechToText``,
``Capabilities``, ``Transcript``, ``StreamingSession``,
``StreamEvent``, the ``audio_*`` factory functions, and the
backend constructors (``mock_speech_to_text`` plus, when their
runtime crates are linked, ``stt_openai``, ``stt_deepgram``,
``stt_assemblyai``, ``stt_whisper``).
"""

from ._native import stt as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
