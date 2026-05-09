"""End-to-end Python test for the local whisper backend.

Runs only when:

- The native extension was built with the `stt-whisper-cpp` feature
  (`maturin develop --features stt-whisper-cpp`), and
- `STT_WHISPER_MODEL` env var points to a ggml/gguf weights file, and
- The `jfk.wav` fixture is present at `crates/stt-core/tests/fixtures/`.
"""

import asyncio
import os
from pathlib import Path

import pytest

native = pytest.importorskip("atomr_agents._native")

REPO_ROOT = Path(__file__).resolve().parents[3]
JFK = REPO_ROOT / "crates/stt-core/tests/fixtures/jfk.wav"
MODEL = os.environ.get("STT_WHISPER_MODEL")


@pytest.mark.skipif(
    not MODEL or not Path(MODEL).exists(),
    reason="set STT_WHISPER_MODEL to a ggml/gguf path to run",
)
@pytest.mark.skipif(not JFK.exists(), reason="missing jfk.wav fixture")
def test_whisper_local_transcribe_jfk() -> None:
    from atomr_agents import stt

    backend = stt.stt_whisper(MODEL, language="en")
    caps = backend.capabilities().to_dict()
    assert caps["batch"] is True
    assert caps["requires_network"] is False
    assert backend.backend_kind() == "whisper_local"
    assert backend.transport_kind() == "local_model"

    async def go() -> str:
        t = await backend.transcribe(stt.audio_file(str(JFK)))
        return t.text

    text = asyncio.run(go())
    assert "country" in text.lower(), text
