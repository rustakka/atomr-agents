---
name: atomr-agents-stt-harness
description: Use when driving speech-to-text as an agentic streaming pipeline in atomr-agents — building an `SttHarness`, accumulating an `SttConversation`, feeding it to an agent, choosing a `DiarizationPolicy`, persisting conversations via a `ConversationStore`, or serving the diarized-transcript review UI (`stt-harness-web`). Triggers on `SttHarness`, `SttConversation`, `AudioSource`, `DiarizationPolicy`, "diarize a recording", "editable speaker labels", "transcript review UI", `atomr_agents.stt_harness`.
---

# STT harness

`atomr-agents-stt-harness` drives speech-to-text as a harness-style
loop. It opens a streaming STT session, pumps audio, diarizes, and
folds the stream into an `SttConversation` that maps to/from the
agentic `TurnInput` / `Message` types. `atomr-agents-stt-harness-web` is
the optional axum + React review UI on top of it.

## When to use it

- You have audio (mic / file / bytes / PCM) and want a **diarized,
  structured transcript** — not just raw text.
- You want that transcript to **feed an agent** (`to_turn_input`) or to
  compose in a workflow (`SttHarnessRef` is `Callable`).
- You want a human to **review and correct speaker attribution** — use
  `stt-harness-web`.
- **Don't** use it for one-shot batch transcription with no speakers —
  call `SpeechToText::transcribe` directly. **Don't** use it for live
  bidirectional voice (STT↔agent↔TTS) — that's `tts-voice::Conversation`.

## Build an `SttHarness`

```rust
use std::sync::Arc;
use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_harness::{
    AudioSource, DiarizationPolicy, SttHarness, SttHarnessSpec,
    StreamingLoop, StreamEndTermination,
};

let backend = Arc::new(MockSpeechToText::new().with_text("hello world"));
let audio = AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1));

let harness = SttHarness::new(
    SttHarnessSpec::new("call-review")
        .with_diarization(DiarizationPolicy::Backend),
    backend,                       // any Arc<dyn SpeechToText>
    audio,
    StreamingLoop::default(),       // SttLoopStrategy
    StreamEndTermination,           // SttTermination
);

let mut events = harness.events();  // subscribe BEFORE run()
let conversation = harness.run().await?;   // -> SttConversation
```

`SttHarnessSpec::into_harness(...)` instead returns a type-erased
`SttHarnessRef` (use for registries / Python / workflow composition).

## Pick the pieces

**Audio source** (`AudioSource`):
- `Pcm(PcmBuffer)` — always available, the test/CI path.
- `File(path)` / `Bytes { data, format }` — need the `decode` feature.
- `Mic(MicOptions)` — needs the `mic` feature.

**Diarization** (`DiarizationPolicy`):
- `Off` — turns carry no speaker.
- `Backend` — trust the backend's speaker tags. Use when
  `backend.capabilities().diarization` is `SpeakerCount` / `NamedSpeakers`
  (Deepgram, AssemblyAI).
- `Layered(Arc<dyn Diarizer>)` — run a local diarizer over the
  utterance PCM. Use when the backend's diarization is `None` (Whisper,
  OpenAI). `MockDiarizer` is the deterministic test double.
- A policy that contradicts the backend's caps emits a
  `DiarizationWarning` event; the run continues.

**Loop / termination**: `StreamingLoop::new(voice_mode)` (`VoiceMode::Live`
surfaces partials as events; `TurnBased` buffers them). Termination:
`StreamEndTermination` (default), `UtteranceCapTermination`,
`AudioSecsTermination`, `BudgetTermination`, `CompositeTermination`.

## The `SttConversation`

```rust
// Feed an agent: last turn -> `user`, the rest -> `history`.
let map = SpeakerMap::default();              // speaker 0..n => User
if let Some(turn_input) = conversation.to_turn_input(&map) {
    let result = agent_ref.turn(turn_input.user, ctx).await?;
    conversation.append_agent_reply(result.text);   // keep a full record
}

// Editable speaker labels — the numeric id is stable, the label is not.
conversation.rename_speaker(0, "Alice");
assert_eq!(conversation.effective_label(0), "Alice"); // override > tag label > "speaker_0"
```

`SttTurn` carries `speaker: SpeakerRef` (`Diarized { tag }` /
`Role { role }` / `Unknown`), `text`, timing, `words`, `state`.

## Persistence

```rust
use atomr_agents_stt_harness::{ConversationStore, InMemoryConversationStore};

let store = InMemoryConversationStore::new();      // volatile default
store.put(&conversation).await?;
store.rename_speaker(&conversation.id, 0, "Alice".into()).await?; // persists
```

With the `state` feature, `CheckpointerConversationStore::new(checkpointer)`
routes through `crates/state`'s configured `Checkpointer`, so edits
survive a restart through the in-memory / SQLite / Postgres backend.

## Events

`harness.events()` returns an `SttEventStream`; `recv().await` yields
`SttHarnessEvent` (`Started`, `Partial`, `UtteranceCommitted`,
`SpeakerChanged`, `UtteranceEnd`, `DiarizationWarning`, `Finished`,
`Error`). The harness also emits `Event::HarnessIteration` to its
`EventBus` so runs show up in the `RunTree`.

## The review UI

`atomr-agents-stt-harness-web` (optional crate): `WebServer::new(config,
store)` builds an axum router (`router()` is public for
`tower::oneshot` tests). Routes: `GET /api/conversations`,
`GET|DELETE /api/conversations/:id`,
`PUT /api/conversations/:id/speakers/:speaker_id`,
`GET /ws`. Forward `harness.events()` into `server.event_sender()` via
`ws::forward_events` so the UI streams live.

- Build the SPA: `cargo xtask stt-web-build` (or
  `npm --prefix crates/stt-harness-web/ui run build`).
- Serve it: `cargo run -p atomr-agents-cli --features stt-web -- serve`.
- Embed the SPA in the binary: build the crate with `--features embed-ui`.

## Python

`atomr_agents.stt_harness` exposes `SttHarnessSpec`, `SttHarness`,
`SttConversation`, `SttTurn`, `SttEventStream`:

```python
from atomr_agents import stt
from atomr_agents.stt_harness import SttHarness, SttHarnessSpec

backend = stt.mock_speech_to_text("hello world")
audio = stt.audio_pcm([0.0] * 16_000, 16_000, 1)
spec = SttHarnessSpec("demo", diarization="layered_mock")  # off|backend|layered_mock
conversation = await SttHarness(spec, backend, audio).run()
print(conversation.to_turn_input())
```

## Gotchas

- Call `harness.events()` **before** `run()` — the broadcast channel
  only buffers from the subscription point.
- `run()` consumes the audio source; a second `run()` errors. Build a
  fresh harness per run.
- `File` / `Bytes` sources without the `decode` feature fail at
  `run()` with a clear config error — gate the feature.
- `Backend` diarization on a `DiarizationSupport::None` backend leaves
  every turn unattributed (and warns). Use `Layered` there.
- The session task ends the post-`finish` drain after a short quiet
  period — real WS backends close their transport; the in-process mock
  does not, so the quiet-period fallback is what stops it.
