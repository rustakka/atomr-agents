"""Facade over :mod:`atomr_agents._native.voice_extras`.

Re-exports voice-adjacent trait handles and factories:

- ``Diarizer`` / ``DiarizationSpan`` — ``mock_diarizer()``,
  ``sherpa_diarizer(...)`` (requires the
  ``stt-diarize-sherpa-onnx`` feature), ``diarizer_from_factory(key)``.
- ``Vad`` — ``energy_vad(threshold)``, ``silero_vad(...)`` (requires
  the ``stt-vad-silero`` feature), ``vad_from_factory(key)``.
- ``Phonemizer`` / ``PhonemizedText`` — ``mock_phonemizer()``,
  ``phonemizer_from_factory(key)``.

Usage::

    from atomr_agents import voice_extras

    d = voice_extras.mock_diarizer()
    spans = await d.diarize(samples=[0.0] * 16000, sample_rate=16000)

    v = voice_extras.energy_vad(threshold=0.01)
    is_speech = v.is_speech([0.0] * 480, 16000)

    p = voice_extras.mock_phonemizer()
    pt = await p.phonemize("hello world", "en-us")
    print(pt.ipa, pt.tokens)
"""

from ._native import voice_extras as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
