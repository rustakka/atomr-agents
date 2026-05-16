# FR-TTS-001 — Migrate TTS runtimes under atomr-infer's ModelRunner

Status: Proposed
Target workspace: `atomr-infer` (currently v0.8.0)
Filed by: downstream consumer (avatar / realtime-speech harness)
Priority: Non-blocking — hygiene and coherence ask

---

## 1. Motivation

`atomr-infer` is positioned as the single inference surface for model
execution: today the `ModelRunner` trait in `atomr-infer-core` accepts
`ExecuteBatch` payloads and emits token streams for text and image-in
models, with real runtimes for Anthropic, OpenAI, and Gemini.

Text-to-speech is the only remaining class of model call that lives
outside that surface for downstream consumers. Each TTS provider
currently ships its own HTTP / WebSocket / ONNX-Runtime client, none of
which share `atomr-infer`'s rate-limiter, retry policy, or telemetry.

Consolidating TTS under `ModelRunner` buys:

- **One inference surface.** Every model call — text, image, audio —
  routes through the same trait, the same dispatch, the same provider
  registry. Easier to reason about, easier to swap providers in tests.
- **Shared rate-limiting and retry.** The TTS providers (OpenAI,
  ElevenLabs) have their own quotas and 429 semantics that today each
  client re-implements. The `atomr-infer-runtime` middleware stack
  already solves this for LLM providers.
- **Unified telemetry.** Token / character counts, latency histograms,
  cost attribution: today TTS calls are invisible to whatever
  observability sink consumes `atomr-infer` events.
- **Future fusion.** Once cognition and synthesis share a runner, we
  can model "audio-aligned-token" streams natively — the LLM emits
  text, and the same runtime back-pressure-couples it to TTS, so a
  downstream avatar / voice agent gets first-audio latency bounded by
  first-token latency rather than `first-token + synth-RTT`. This is
  out of scope for this FR but the data shape proposed below leaves
  room for it.

---

## 2. Current state in the downstream consumer

The downstream workspace ships the following TTS runtime crates, each
with its own client implementation, none routed through `atomr-infer`:

| Crate | Provider | Transport | Notable surface |
|---|---|---|---|
| `tts-core` | n/a | n/a | `TextToSpeech` trait, `SynthesisRequest`, `AudioOutput`, `RealtimeSession` |
| `tts-runtime-openai` | OpenAI | HTTPS | `gpt-4o-mini-tts`, `tts-1`, `tts-1-hd`, batch PCM |
| `tts-runtime-elevenlabs` | ElevenLabs | HTTPS + WS | Voice library, voice cloning, character-level alignment over WebSocket |
| `tts-runtime-openai-realtime` | OpenAI | WSS | Bidirectional realtime session |
| `tts-runtime-gemini-live` | Google | WSS | Bidirectional realtime session |
| `tts-runtime-piper` | Local | ONNX Runtime | Batch PCM, no auth, fastest to port |
| `tts-runtime-kokoro` | Local | ONNX Runtime | Batch PCM |
| `tts-runtime-moss` | Local | MOSS-TTS | Batch PCM |
| `tts-runtime-xtts` | Local | ONNX Runtime | Coqui XTTS v2, voice cloning |

The surfaces are not uniform across providers:

- Some emit batch PCM only.
- Some stream PCM in chunks.
- Some emit character-level or phoneme-level alignment alongside
  audio (ElevenLabs in particular). Alignment is load-bearing for
  the downstream avatar / lipsync use case.
- Two providers (OpenAI Realtime, Gemini Live) are bidirectional —
  audio in, audio out, with interruption / barge-in semantics —
  which doesn't fit the request/response shape of `ExecuteBatch`.

---

## 3. Proposed atomr-infer surface

### 3.1 New `RuntimeKind` variants

Split TTS into two kinds. Batch synthesis (request → audio stream)
fits naturally into the existing `ModelRunner` shape; realtime is
fundamentally bidirectional and needs its own variant.

```rust
pub enum RuntimeKind {
    // existing
    Text,
    // new
    TextToSpeech,
    RealtimeSpeech,
}
```

### 3.2 `SpeechBatch` — the batch payload

```rust
pub struct SpeechBatch {
    pub request_id: String,
    pub model: String,
    pub text: String,
    pub voice: VoiceRef,         // see 3.4
    pub options: SynthOptions,
    pub stream: bool,
    pub emit_alignment: bool,    // when true, runner emits AlignmentDelta
    pub estimated_tokens: u32,   // for budget / rate-limit accounting
}

pub struct SynthOptions {
    pub language: Option<String>,        // BCP-47, e.g. "en-US"
    pub pitch: Option<f32>,              // semitones, -12.0..=12.0
    pub rate: Option<f32>,               // 0.5..=2.0
    pub format: AudioFormat,             // Pcm16le, OpusOgg, Mp3, …
    pub sample_rate_hz: u32,             // requested; runner may clamp
    pub style: Option<String>,           // provider-specific tag
}
```

### 3.3 Streaming output

When `stream == true` the runner emits a sequence of `SpeechChunk`s
followed by a terminal `SpeechEnd` event. Non-streaming requests
collapse to a single chunk with `is_final == true`.

```rust
pub struct SpeechChunk {
    pub audio_pcm_chunk: Bytes,
    pub sample_rate_hz: u32,
    pub alignment: Option<AlignmentDelta>,
    pub is_final: bool,
}

pub struct AlignmentDelta {
    /// Character offsets in `SpeechBatch::text`, with the start time
    /// in milliseconds since synthesis began. Length matches the
    /// number of characters covered by this chunk.
    pub char_starts_ms: Vec<u32>,
    /// Optional viseme assignment for this chunk's primary phoneme.
    /// Normalization across providers is an open question (see §8).
    pub viseme: Option<Viseme>,
}

pub enum Viseme {
    // placeholder; normalization TBD
    Oculus(u8),    // 0..=14
    Azure(u8),     // 0..=21
    Raw(String),   // provider-native tag
}
```

### 3.4 `VoiceRef` — re-exported from downstream `tts-core`

Voice selection is provider-specific but factors cleanly:

```rust
pub enum VoiceRef {
    /// Named voice from the provider's catalogue.
    Named(String),
    /// Stable provider-side voice id (ElevenLabs voice_id, OpenAI
    /// voice name, local model path, …).
    Id(String),
    /// Cloned voice — see open question on preload vs per-call.
    ClonedFrom(AudioInput),
}
```

We propose re-exporting `VoiceRef` and `AudioInput` from the
downstream `atomr-agents-tts-core` crate into `atomr-infer-core`,
so the shim in §7 doesn't introduce a parallel type hierarchy.

### 3.5 Realtime — `RealtimeBatch`

Realtime providers (OpenAI Realtime API, Gemini Live) need
bidirectional flow: the caller sends audio frames + control events,
the runtime emits audio frames + transcripts + tool-call deltas.

```rust
pub struct RealtimeBatch {
    pub request_id: String,
    pub model: String,
    pub voice: VoiceRef,
    pub options: SynthOptions,
    /// Caller-owned sender for inbound frames + control events.
    pub inbound: mpsc::Receiver<RealtimeIn>,
    /// Runtime-owned sender for outbound frames + events. Closed by
    /// the runtime when the session ends.
    pub outbound: mpsc::Sender<RealtimeOut>,
}

pub enum RealtimeIn {
    AudioFrame { pcm: Bytes, sample_rate_hz: u32 },
    Text(String),
    Commit,
    Interrupt,
    Close,
}

pub enum RealtimeOut {
    AudioFrame { pcm: Bytes, sample_rate_hz: u32 },
    Transcript { role: Role, text: String, is_final: bool },
    Alignment(AlignmentDelta),
    Error(RuntimeError),
    Done,
}
```

`mpsc` as the transport is a starting point — see open question in
§8 on whether a dedicated `Session` trait is a better fit.

---

## 4. Backwards compatibility

The change is purely additive:

- Existing `ModelRunner` impls (Anthropic, OpenAI text, Gemini)
  continue to handle `RuntimeKind::Text` exclusively. They never see
  `TextToSpeech` or `RealtimeSpeech` payloads.
- LLM callers are untouched. The new variants are gated behind new
  cargo features in `atomr-infer-runtime` so consumers that don't need
  TTS pay zero cost in build time or binary size:
  - `tts-openai`
  - `tts-elevenlabs`
  - `tts-openai-realtime`
  - `tts-gemini-live`
  - `tts-piper`
  - `tts-kokoro`
  - `tts-moss`
  - `tts-xtts`
- The existing provider-registry construction APIs grow optional
  builder methods (`with_tts_openai(...)`, `with_tts_piper(...)`, …)
  rather than changing existing signatures.

---

## 5. Migration plan

Three phases. Each leaves the tree in a working state.

### Phase A — Define the trait surface, port the simplest provider

1. Land `SpeechBatch`, `SpeechChunk`, `AlignmentDelta`, `VoiceRef`,
   `AudioInput`, `RuntimeKind::TextToSpeech` in `atomr-infer-core`.
2. Extend `ModelRunner` with a `speak(&self, batch: SpeechBatch) ->
   impl Stream<Item = Result<SpeechChunk>>` method, default-impl'd
   to return `Err(RuntimeError::Unsupported)` so existing runners
   compile unchanged.
3. Port `tts-runtime-piper` into `atomr-infer-runtime` under
   feature `tts-piper`. Piper is local (ONNX), unauthenticated, and
   has no streaming or alignment — the smallest possible
   end-to-end path through the new trait, which makes it the right
   reference impl for the trait shape itself.
4. Add an integration test in `atomr-infer-runtime` that exercises
   `speak` against the Piper runtime with fixed input.

Exit criteria: a downstream consumer can call `ModelRunner::speak`
on a Piper runtime and get PCM out.

### Phase B — Hosted batch providers

5. Port `tts-runtime-openai` (`gpt-4o-mini-tts`, `tts-1`,
   `tts-1-hd`). Streaming PCM, no alignment. Reuses
   `atomr-infer-runtime`'s existing OpenAI auth + rate-limit
   middleware.
6. Port `tts-runtime-elevenlabs`. This is the trickiest reference
   impl because of the WebSocket alignment stream — character-level
   timing arrives interleaved with audio chunks, and the
   `AlignmentDelta` schema in §3 needs to round-trip that without
   loss. ElevenLabs is also where voice cloning enters the picture,
   forcing resolution of the `VoiceRef::ClonedFrom` open question.
7. Port the remaining local providers (`kokoro`, `moss`, `xtts`)
   behind their respective features.

Exit criteria: every batch-mode TTS runtime in the downstream
workspace has an `atomr-infer-runtime` equivalent. The downstream
crates can be deprecated.

### Phase C — Realtime providers

8. Land `RuntimeKind::RealtimeSpeech`, `RealtimeBatch`,
   `RealtimeIn`, `RealtimeOut` in `atomr-infer-core`. Extend
   `ModelRunner` with a `realtime(&self, batch: RealtimeBatch) ->
   Result<()>` method, default-impl'd to `Err(Unsupported)`.
9. Port `tts-runtime-openai-realtime` and
   `tts-runtime-gemini-live` behind their respective features.
10. Decide (informed by the implementations) whether `mpsc` is the
    right transport or whether a dedicated `Session` trait reads
    better — see §8.

Exit criteria: the downstream avatar harness routes every TTS call
— batch and realtime — through `ModelRunner`.

---

## 6. Trade-offs

### Keep a thin downstream shim?

The downstream `atomr-agents-tts-core` crate defines a `TextToSpeech`
trait that the avatar harness and several other consumers depend on
directly. We do **not** want to fork that trait or force every
downstream caller to rewrite against `ModelRunner`.

**Recommendation: keep `atomr-agents-tts-core`, but rewrite its
default impls to wrap a `ModelRunner`.** Concretely:

```rust
// atomr-agents-tts-core
impl<R: ModelRunner> TextToSpeech for ModelRunnerTts<R> {
    async fn synthesize(&self, req: SynthesisRequest) -> AudioOutput {
        let batch = SpeechBatch::from(req);
        let stream = self.runner.speak(batch).await?;
        // collect / forward
    }
}
```

This gives us:
- Downstream callers keep their existing trait.
- One implementation of TTS per provider, living in
  `atomr-infer-runtime`.
- The shim is small enough to maintain without it becoming a fork.

### Cost of the split between `TextToSpeech` and `RealtimeSpeech`

The two variants are different enough that forcing them into one
shape (e.g. modeling batch as a degenerate realtime session) would
make the common case awkward. The cost of the split is one extra
enum variant and one extra trait method — small.

### Feature explosion

Eight new cargo features in `atomr-infer-runtime` is a lot. We
accept this as the cost of letting LLM-only consumers avoid
pulling in ONNX Runtime, WebSocket dependencies, and provider SDKs
they don't use. A meta-feature `tts-all` for convenience is cheap.

---

## 7. Downstream shim — concrete shape

Sketched here so reviewers can see the full picture; not part of
the `atomr-infer` work itself.

```rust
// atomr-agents-tts-core (downstream, unchanged trait)
pub trait TextToSpeech {
    async fn synthesize(&self, req: SynthesisRequest)
        -> Result<AudioOutput>;
    async fn synthesize_stream(&self, req: SynthesisRequest)
        -> Result<BoxStream<'static, AudioChunk>>;
}

// atomr-agents-tts-core (new adapter)
pub struct ModelRunnerTts<R: ModelRunner> { runner: R, model: String }

impl<R: ModelRunner> TextToSpeech for ModelRunnerTts<R> { /* … */ }
```

The avatar harness, downstream tests, and any other current
`TextToSpeech` consumer continues to compile unmodified.

---

## 8. Open questions

1. **Voice cloning lifecycle.** `VoiceRef::ClonedFrom(AudioInput)` —
   does the runtime preload the reference clip into a session that
   subsequent calls reference, or does each `SpeechBatch` ship the
   clip inline? Preloading is cheaper at call time but introduces
   session state that `ModelRunner` doesn't otherwise have.
   ElevenLabs and XTTS handle this very differently.

2. **Viseme normalization.** Providers disagree on viseme schemes:
   ElevenLabs / Azure use a 22-id scheme, Oculus / Meta uses a
   15-id scheme, some local models emit raw phoneme strings. Should
   `Viseme` normalize to one canonical scheme (and lose information
   for the consumers who want the native scheme), or should it
   preserve the provider's native form and leave normalization to a
   downstream utility?

3. **Realtime transport.** Is the `mpsc<RealtimeIn, RealtimeOut>`
   pair the right abstraction, or should realtime go through a
   dedicated `Session` trait (something like
   `trait RealtimeSession { fn send(...); fn recv(...); fn close(); }`)?
   `mpsc` composes well with existing async code but leaks
   channel-shaped error semantics; a `Session` trait is harder to
   implement against multiple providers but gives the runner more
   control over backpressure and shutdown.

4. **Alignment latency contract.** Should `AlignmentDelta` always
   arrive in the same `SpeechChunk` as its corresponding audio, or
   may it arrive in a later chunk? ElevenLabs sometimes emits
   alignment ahead of the audio; modeling that explicitly (e.g.
   `AlignmentDelta { covers_audio_offset_ms: Range<u32> }`) might be
   worth the extra field.

5. **Cost / token accounting.** TTS providers bill by character,
   not by token. Does `estimated_tokens` get repurposed, do we add
   `estimated_characters`, or do we generalize to `estimated_units`
   with a kind tag? This affects shared rate-limit middleware.

---

## 9. References

Source crates in the downstream consumer workspace
(`crates/...` paths):

- `crates/tts-core/src/trait_.rs` — the `TextToSpeech` trait,
  `SynthesisRequest`, `AudioOutput`, `RealtimeSession` types that
  the proposed shim wraps.
- `crates/tts-runtime-openai/` — OpenAI `gpt-4o-mini-tts`,
  `tts-1`, `tts-1-hd`. Reference for Phase B step 5.
- `crates/tts-runtime-elevenlabs/` — voice library, voice cloning,
  WebSocket character-level alignment. Reference for Phase B step
  6 and the alignment schema.
- `crates/tts-runtime-openai-realtime/` — OpenAI Realtime API,
  bidirectional. Reference for Phase C.
- `crates/tts-runtime-gemini-live/` — Gemini Live, bidirectional.
  Reference for Phase C.
- `crates/tts-runtime-piper/` — local ONNX, no auth, no streaming.
  Reference for Phase A.
- `crates/tts-runtime-kokoro/` — local ONNX.
- `crates/tts-runtime-moss/` — MOSS-TTS local.
- `crates/tts-runtime-xtts/` — Coqui XTTS v2 local, voice cloning.

---

## 10. Estimating effort

The expectation, based on the existing LLM-runtime ports in
`atomr-infer-runtime`, is roughly:

- Phase A: 1–2 engineer-weeks (trait design + one local runtime).
- Phase B: 2–3 engineer-weeks (three hosted providers, ElevenLabs
  alignment is the bulk of the work) + 1 week for the three
  remaining local runtimes.
- Phase C: 2–3 engineer-weeks (two realtime providers, plus the
  trait/transport decision in open question 3).

Total: ~8–10 engineer-weeks across three phases, each independently
mergeable.

This FR is **non-blocking** for the downstream workspace — the
current direct-client setup works today. The ask is to track this
as a coherence and hygiene goal rather than as urgent work.
