"""Facade over :mod:`atomr_agents._native.voice`.

Re-exports ``VoiceMode``, ``VoiceEvent``, ``VoiceSession``,
``VoiceEventIter``.

Usage::

    import asyncio
    from atomr_agents import stt, voice

    async def run():
        backend = stt.stt_deepgram(api_key="env:DEEPGRAM_API_KEY")
        session = await backend.open_stream(format="wav", language="en")
        vs = voice.VoiceSession.open(session, voice.VoiceMode.turn_based(800))
        async for ev in vs.events():
            print(ev)
"""

from ._native import voice as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
