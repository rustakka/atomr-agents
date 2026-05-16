# FR-STT-001 — Migrate STT runtimes under atomr-infer's ModelRunner

- **Status:** Proposed
- **Filed against:** `atomr-infer` (currently v0.8.0)
- **Filed by:** `atomr-agents` maintainers
- **Type:** Architecture / feature request (non-blocking)
- **Target surface:** `atomr-infer-core::ModelRunner`, `RuntimeKind`, `atomr-infer-runtime`

---

## 1. Motivation

`atomr-infer` exposes a single, well-shaped contract for model execution: a
`ModelRunner` trait that consumes `ExecuteBatch` payloads, returns a
`RunHandle`, and dispatches by `RuntimeKind`. Today that contract covers
text- and image-in, token-stream-out LLMs (Anthropic, OpenAI, Gemini).

A growing class of consumers also needs **speech-to-text** as a first-class
inference modality — transcription, diarization, real-time captioning. Those
calls are model calls in every sense that matters operationally: they hit
remote providers, they have token/audio-second cost, they need rate limiting,
they fail and need retrying, they emit streamed chunks, and downstream
pipelines want to compose them with LLM calls (e.g., "transcribe this clip,
then summarize the transcript" as a single observable unit).

Right now those STT calls bypass `atomr-infer` entirely, which means:

- **Two HTTP surfaces.** LLM traffic is routed, retried, and instrumented by
  `atomr-infer`. STT traffic is not. Observability dashboards have to union
  two sources to answer "how much did this pipeline cost?"
- **No shared rate-limiter.** When the same OpenAI key is used for both
  `gpt-4o` and `gpt-4o-transcribe`, the two clients fight for quota
  independently.
- **No cross-modal batching.** A future runner could group transcribe-then-
  summarize into a single scheduling unit, but only if both stages live in
  the same dispatcher.
- **Duplicated client plumbing.** Each STT provider crate re-implements
  reqwest/tungstenite setup, auth header injection, backoff, and
  cancellation semantics that `atomr-infer-runtime` already solved for LLMs.

The architectural invariant we want to hold in `atomr-agents` is **"all model
calls flow through `atomr-infer`."** Today STT is the lone exception, and
this FR is the request to close that gap.

This is **non-blocking** for `atomr-agents` — the existing STT crates work.
It is a coherence-and-hygiene ask, with concrete operational wins once
landed.

---

## 2. Current state in atomr-agents

The following STT runtime crates ship in `atomr-agents` today, each with its
own HTTP or WebSocket client and no awareness of `atomr-infer`:

| Crate                              | Backend                          | Transport      | Streaming |
| ---------------------------------- | -------------------------------- | -------------- | --------- |
| `crates/stt-core`                  | trait + types (`SpeechToText`, `AudioInput`, `Transcript`) | n/a            | n/a       |
| `crates/stt-runtime-openai`        | Whisper / `gpt-4o-transcribe`    | HTTPS (batch)  | No        |
| `crates/stt-runtime-whisper`       | local whisper.cpp via FFI        | in-process     | No        |
| `crates/stt-runtime-deepgram`      | Deepgram                         | WebSocket      | Yes       |
| `crates/stt-runtime-assemblyai`    | AssemblyAI                       | WebSocket      | Yes       |

Each crate independently owns its retry logic, auth, framing, and
cancellation. There is no shared scheduler and no `ModelRunner` integration.

---

## 3. Proposed atomr-infer surface

The shape below is sketched at the level needed for an effort estimate — the
exact field names should be settled during Phase A review.

### 3.1 New `RuntimeKind` variant

```rust
pub enum RuntimeKind {
    Anthropic,
    OpenAI,
    Gemini,
    // ...
    /// Speech-to-text runtimes. Dispatches `AudioBatch`, not `ExecuteBatch`.
    SpeechToText,
}
```

The dispatcher distinguishes the payload type per variant, so adding a new
modality variant does not require breaking `ExecuteBatch` consumers.

### 3.2 `AudioBatch` payload

The existing `ExecuteBatch` is LLM-shaped (messages, tools, sampling params)
and is the wrong fit for audio. We propose a sibling type:

```rust
pub struct AudioBatch {
    /// Caller-supplied id for correlation across logs, traces, and retries.
    pub request_id: String,

    /// Provider-specific model identifier (e.g. "whisper-1",
    /// "gpt-4o-transcribe", "nova-2", "best").
    pub model: String,

    /// The audio bytes or stream. Re-export of
    /// `atomr_agents_stt_core::AudioInput` (or a relocated equivalent).
    pub audio_input: AudioInput,

    /// If true, the runtime opens a streaming session and emits
    /// `TranscriptChunk`s as they arrive. If false, the runtime returns a
    /// single final chunk.
    pub stream: bool,

    /// Transcription-specific knobs (see below).
    pub options: TranscribeOptions,

    /// Caller's best guess at token cost, for scheduling. Optional, but
    /// useful for the shared rate-limiter when audio duration is known.
    pub estimated_tokens: u32,
}

pub struct TranscribeOptions {
    pub language: Option<String>,        // BCP-47 hint, None = auto-detect
    pub diarize: bool,                   // speaker labels
    pub punctuation: bool,
    pub keywords: Vec<String>,           // bias terms / vocabulary
    pub profanity_filter: Option<bool>,
    pub vad: Option<VadConfig>,          // see open questions
}
```

### 3.3 Streaming output

LLM consumers already drive results via `RunHandle::into_stream()`. We want
STT to use the same shape, so consumers can write modality-agnostic glue:

```rust
pub struct TranscriptChunk {
    pub is_final: bool,
    pub text: String,
    pub words: Vec<Word>,           // may be empty if provider lacks word timing
    pub ts_start_ms: u64,
    pub ts_end_ms: u64,
    pub speaker_id: Option<String>, // populated when diarize = true
}

pub struct Word {
    pub text: String,
    pub ts_start_ms: u64,
    pub ts_end_ms: u64,
    pub confidence: Option<f32>,
}
```

Batch-only providers (OpenAI, local Whisper) emit exactly one chunk with
`is_final = true`. Streaming providers (Deepgram, AssemblyAI) emit interim
chunks with `is_final = false` followed by finals.

### 3.4 Dispatch entry point

```rust
impl dyn ModelRunner {
    async fn execute_audio(&self, batch: AudioBatch) -> Result<RunHandle<TranscriptChunk>>;
}
```

or, if you prefer to keep `ModelRunner` single-method, a sibling trait
`AudioRunner` that shares the same `RunHandle` machinery. Either is fine
from the caller's side — the constraint is that the returned handle behaves
identically to the LLM one (cancellation, backpressure, completion signal).

---

## 4. Backwards compatibility

This is purely **additive**:

- Existing `ExecuteBatch` callers and existing `RuntimeKind` variants are
  untouched.
- New `RuntimeKind::SpeechToText` is opt-in; consumers that do not link an
  STT runtime never see it.
- Provider crates land behind new cargo features in `atomr-infer-runtime`:
  `stt-openai`, `stt-whisper`, `stt-deepgram`, `stt-assemblyai`. The
  `atomr-infer-runtime` default feature set stays as it is today.
- The `AudioInput` type can either be re-exported from a small shared crate
  or relocated into `atomr-infer-core`. Either choice keeps the wire-level
  payload stable.

No existing `atomr-infer` version needs to be yanked.

---

## 5. Migration plan

We propose three phases. Each is independently reviewable and shippable.

### Phase A — Core surface

- Land `AudioBatch`, `TranscribeOptions`, `TranscriptChunk`, `Word` in
  `atomr-infer-core` behind a `stt` feature flag.
- Add `RuntimeKind::SpeechToText` and the dispatch entry point.
- Wire up the existing `RunHandle` plumbing for the new payload type.
- No provider implementations yet — Phase A is the contract.

### Phase B — Reference provider (OpenAI)

- Port `atomr-agents/crates/stt-runtime-openai` into
  `atomr-infer-runtime` under the `stt-openai` feature.
- OpenAI is batch-only, single-request, no WebSocket — it is the smallest
  surface that exercises the contract end-to-end.
- This phase validates that retries, auth, and rate-limiting share the LLM
  path correctly.

### Phase C — Streaming providers + local

- Port Deepgram and AssemblyAI (both WebSocket-streaming).
- Port local `whisper.cpp` last; it has FFI bindgen and a build script. The
  build must continue to support both `x86_64-unknown-linux-gnu` and
  `aarch64-unknown-linux-gnu` (the `atomr-agents` wheel matrix requires
  aarch64 Linux coverage today and must not regress).

Once Phase C is in `atomr-infer`, `atomr-agents` will delete its in-tree
`stt-runtime-*` crates and depend on `atomr-infer-runtime` with the
appropriate features.

---

## 6. Trade-offs

### Keep a thin `atomr-agents-stt-core` shim?

Today downstream `atomr-agents` consumers code against the `SpeechToText`
trait in `crates/stt-core`. If `atomr-infer` becomes the source of truth,
those consumers have two options:

1. **Rewrite to `ModelRunner` directly.** Clean, but every downstream caller
   churns at the same time.
2. **Keep `atomr-agents-stt-core` as a shim** whose `SpeechToText`
   implementation is a thin adapter over `ModelRunner::execute_audio`.
   Downstream callers see no API change; the shim disappears at its own
   pace.

**Recommendation: option 2.** The shim is ~100 lines, keeps churn local to
`atomr-agents`, and lets us migrate one consumer at a time. Once everyone is
on `ModelRunner` directly, the shim is deleted.

### `AudioInput` ownership

Two reasonable homes:

- Keep it in a shared `atomr-stt-types` crate that both workspaces depend
  on. Lowest disruption.
- Relocate it into `atomr-infer-core`. Cleaner long-term but means
  `atomr-agents` re-exports from `atomr-infer-core` for backcompat.

Either works; we lean toward the shared types crate so neither workspace
becomes a hard dependency root for audio bytes.

---

## 7. Open questions

1. **Diarization as a sub-feature or its own `RuntimeKind`?** Some
   providers expose diarization-only endpoints (no transcription). Should
   that be `RuntimeKind::Diarize` with a `DiarizeBatch`, or do we keep it
   as `TranscribeOptions::diarize = true` and accept that callers asking
   for diarization-only get a transcript they discard?

2. **Voice activity detection (VAD).** Some providers do server-side VAD;
   others expect the client to gate the stream. Should `VadConfig` be in
   `TranscribeOptions`, a separate `AudioPreprocessing` struct, or out of
   scope for v1 of this surface?

3. **Word-level timing: in-chunk or sidecar?** `TranscriptChunk.words` is
   convenient but inflates payload size for providers that always emit
   word timing. Should word timing live on a separate channel
   (`RunHandle::words_stream()`) that callers opt into, with `TranscriptChunk`
   carrying only sentence-level text + bounds?

4. **Cost accounting unit.** LLM cost is "tokens." Audio cost is more
   naturally "seconds" or "characters of output." Does `RunHandle`'s
   existing cost-reporting hook need a new unit, or do we coerce
   audio-seconds into `estimated_tokens` for the scheduler?

5. **Local runtime scheduling.** `stt-runtime-whisper` runs in-process via
   FFI — it has no remote rate limit but does have a CPU/GPU contention
   profile. Should the shared scheduler treat it as `RuntimeKind::Local`
   for budgeting, or is per-runtime concurrency config enough?

---

## 8. References

The relevant code in `atomr-agents` that this FR proposes absorbing:

- `crates/stt-core/src/trait_.rs` — the `SpeechToText` trait and core types
- `crates/stt-core/src/lib.rs` — `AudioInput`, `Transcript`
- `crates/stt-runtime-openai/` — Whisper / `gpt-4o-transcribe` (batch HTTPS)
- `crates/stt-runtime-whisper/` — local whisper.cpp FFI runtime (x86_64 +
  aarch64 Linux required)
- `crates/stt-runtime-deepgram/` — Deepgram WebSocket runtime
- `crates/stt-runtime-assemblyai/` — AssemblyAI WebSocket runtime

A walking tour of any one of the runtime crates is enough to see the
duplicated client/auth/retry shape; the OpenAI one is the smallest if you
want a single read.
