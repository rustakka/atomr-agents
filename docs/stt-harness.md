# STT harness

The STT harness drives **speech-to-text as an agentic streaming
pipeline**. It bridges the workspace's STT stack (`stt-core`,
`stt-audio`, `stt-diarize-sherpa`, the `stt-runtime-*` backends) and its
agentic stack (`harness`, `agent`, `callable`, `observability`,
`state`): it opens a streaming STT session, pumps audio, diarizes, and
folds the stream into an `SttConversation` that maps cleanly to and from
the agentic `TurnInput` / `Message` types — so an STT interaction can
feed an agent turn directly.

## The problem it solves

`stt-voice` coalesces a `StreamingSession` into `VoiceEvent`s, but
nothing in the workspace drove STT as a *harness-style loop*,
accumulated a *conversation record* aligned to agentic structures, or
exposed that record for human review and speaker correction. The
`stt-harness` crate is that missing layer; `stt-harness-web` is the
optional review UI on top of it.

## Shape

`SttHarness` follows the workspace `BoxedX` pattern, exactly like
`atomr-agents-harness`:

- `SttHarness<L, T>` — typed, monomorphic over the loop and termination
  strategies.
- `BoxedSttHarness` — the type-erased twin (`Box<dyn SttLoopStrategy>` /
  `Box<dyn SttTermination>`), for Python loaders and registries.
- Both funnel into one shared loop body (`run_impl`).
- `SttHarnessRef` is the public handle and implements
  [`Callable`](../crates/callable) — so an STT harness drops into a
  `Pipeline`, a workflow step, or a team routing target.

```rust
use std::sync::Arc;
use atomr_agents_stt_core::{MockSpeechToText, PcmBuffer};
use atomr_agents_stt_harness::{
    AudioSource, SttHarness, SttHarnessSpec, StreamingLoop, StreamEndTermination,
};

let backend = Arc::new(MockSpeechToText::new().with_text("hello world"));
let audio = AudioSource::Pcm(PcmBuffer::new(vec![0.0; 16_000], 16_000, 1));
let harness = SttHarness::new(
    SttHarnessSpec::new("demo"),
    backend,
    audio,
    StreamingLoop::default(),
    StreamEndTermination,
);
let conversation = harness.run().await?;          // -> SttConversation
let turn_input = conversation.to_turn_input(&Default::default()); // -> agentic TurnInput
```

## Pipeline

1. **Audio source** — `AudioSource::{Mic, File, Bytes, Pcm}` builds an
   `AudioPump`. `File` / `Bytes` need the `decode` feature; `Mic` needs
   `mic`. `Pcm` is the zero-I/O path used by tests.
2. **Session task** — a dedicated task owns the live `StreamingSession`
   (whose `push_audio` and `events()` both take `&mut self`, so they
   cannot run concurrently from one caller). It pumps audio in and
   forwards `StreamEvent`s out over a channel. The harness loop never
   touches the session directly.
3. **Loop** — `run_impl` mirrors `atomr-agents-harness`'s loop:
   termination check → `SttLoopStrategy::step` → emit
   `Event::HarnessIteration` to the `EventBus` (so STT runs show up in
   the `RunTree` as `RunKind::Harness`). The default `StreamingLoop`
   folds each event burst into the conversation.
4. **Diarization** — `DiarizationPolicy` decides speaker attribution:
   `Off` (no speakers), `Backend` (trust the backend's tags, for
   backends whose `Capabilities::diarization` is not `None`), or
   `Layered(Arc<dyn Diarizer>)` (retain the utterance PCM, run a local
   diarizer when it commits, stitch the spans on by maximum overlap). A
   policy that contradicts the backend's capabilities emits a
   `DiarizationWarning` and the run continues.
5. **Termination** — `StreamEndTermination` (the default — run until the
   stream ends), `UtteranceCapTermination`, `AudioSecsTermination`,
   `BudgetTermination`, or `CompositeTermination` (first to fire wins).

## The conversation record

`SttConversation` is **pure, serializable data** — the harness's
"working memory" and the value `run()` returns:

- `turns: Vec<SttTurn>` — committed utterances, in order. Each `SttTurn`
  carries a `SpeakerRef` (`Diarized { tag }` / `Role { role }` /
  `Unknown`), text, timing, words, and `TurnState`.
- `speaker_labels: HashMap<u8, String>` — per-conversation overrides.
  This is the **editable surface**: `rename_speaker(id, label)` renames
  a speaker without touching the numeric id, and `effective_label(id)`
  resolves *override > backend label > `speaker_{id}`*.
- Mappings to agentic structures: `to_messages(&SpeakerMap)`,
  `to_turn_input(&SpeakerMap)` (last turn → `user`, the rest →
  `history`), and `append_agent_reply(text)` to keep a full record of
  the exchange.

## Events

Two layers, matching the rest of the framework:

- **Structured telemetry** — `Event::HarnessIteration` on the shared
  `EventBus`, one per loop iteration, so STT runs appear in the run
  tree.
- **Domain stream** — `SttHarnessEvent` (`Started`, `Partial`,
  `UtteranceCommitted`, `SpeakerChanged`, `UtteranceEnd`, `Metadata`,
  `DiarizationWarning`, `Finished`, `Error`) over a `tokio::broadcast`
  channel. `SttHarness::events()` returns an `SttEventStream`; subscribe
  *before* `run()`.

## Persistence

`ConversationStore` is the persistence trait: `put`, `get`, `list`,
`delete`, and `rename_speaker` (read-modify-write, so the edit lands in
whatever backend is configured). `InMemoryConversationStore` is the
always-available default. With the `state` feature,
`CheckpointerConversationStore` routes through `crates/state`'s
`Checkpointer` — the configured persistence provider — so conversations
and edited speaker labels persist through the in-memory, SQLite, or
Postgres backend the deployment wired up, and survive a restart.

## The review UI (`stt-harness-web`)

`atomr-agents-stt-harness-web` is an **optional**, feature-flagged crate
— an axum backend plus a React SPA that matches the atomr-dashboard
style. It shows the conversation list, the diarized transcript, and
**inline-editable per-conversation speaker labels** (rename "speaker_0"
to "Alice" — it re-labels every turn by that speaker at once, with an
optimistic cache patch), with live updates over `/ws`.

REST + WebSocket surface:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/conversations` | summary rows |
| `GET` | `/api/conversations/:id` | full transcript |
| `DELETE` | `/api/conversations/:id` | remove |
| `PUT` | `/api/conversations/:id/speakers/:speaker_id` | rename a speaker |
| `GET` | `/ws` | live `SttHarnessEvent` stream |

Run it via the CLI: `cargo run -p atomr-agents-cli --features stt-web --
serve --bind 127.0.0.1:7000`. The React app lives in
`crates/stt-harness-web/ui/` — see its `README.md` for the dev-server
and `embed-ui` build flow.

## Python

`atomr_agents.stt_harness` exposes `SttHarnessSpec`, `SttHarness`,
`SttConversation`, `SttTurn`, and `SttEventStream`:

```python
from atomr_agents import stt
from atomr_agents.stt_harness import SttHarness, SttHarnessSpec

backend = stt.mock_speech_to_text("hello world")
audio = stt.audio_pcm([0.0] * 16_000, 16_000, 1)
spec = SttHarnessSpec("demo", diarization="layered_mock")
conversation = await SttHarness(spec, backend, audio).run()
print(conversation.to_turn_input())   # {"user": "hello world", "history": []}
```

## Feature flags

| Crate | Feature | Pulls in |
|---|---|---|
| `stt-harness` | `decode` | symphonia decode for `File` / `Bytes` sources |
| `stt-harness` | `mic` | cpal microphone capture |
| `stt-harness` | `state` | `CheckpointerConversationStore` over `crates/state` |
| `stt-harness-web` | `embed-ui` | bake the built React SPA into the binary |
| `cli` | `stt-web` | wire `atomr-agents serve` to the real web server |

The umbrella crate exposes the whole thing behind `stt-harness`
(`atomr_agents::stt::harness`).
