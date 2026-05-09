# Audio test fixtures

Place test audio files here. The cloud-backend integration tests
(`crates/stt-runtime-{openai,deepgram,assemblyai}/tests/cloud.rs`)
expect at least one of:

- `jfk.wav` — the 11-second JFK speech clip from whisper.cpp's
  repository (`samples/jfk.wav`). Mono, 16 kHz, signed 16-bit PCM.
  Used by all three cloud backends' integration tests; they assert
  the transcript contains the word "country".

To download:

```sh
curl -L https://github.com/ggerganov/whisper.cpp/raw/master/samples/jfk.wav \
  -o crates/stt-core/tests/fixtures/jfk.wav
```

Fixtures are intentionally **not** committed to git (gitignored
below) so the repo doesn't grow with binary blobs.
