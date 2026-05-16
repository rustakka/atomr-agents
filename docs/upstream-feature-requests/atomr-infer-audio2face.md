# FR-A2F-001 — Audio→ARKit-blendshape modality (Audio2Face)

**Status:** proposed
**Target crate:** `atomr-infer-core` (and `atomr-infer-runtime`)
**Target version:** the next minor after current `0.8.x`
**Filed by:** atomr-agents avatar working group
**Tracking id:** `FR-A2F-001`

---

## 1. Summary

We want `atomr-infer` to grow a first-class **audio-in → ARKit-blendshape-out** modality, on equal footing with today's text/vision LLM modality. Concretely: a new `RuntimeKind::Audio2Face` variant, a new `AudioBatch` payload type, a streaming response of `BlendshapeChunk { timestamp_ms, weights: [f32; 52] }`, and a `RuntimeConfig::Audio2Face { … }` carrying the gRPC endpoint of an NVIDIA Audio2Face-3D microservice (or any compatible backend).

Without this, downstream consumers that need full-face animation from speech audio have to reach around `atomr-infer` entirely and talk to the A2F microservice directly, which breaks the workspace's "every model call goes through `atomr-infer`" invariant (observability, retry, fan-out, cancellation, runtime selection).

The change is **purely additive** and gated behind a new `audio2face` cargo feature so callers who don't need it pay nothing.

---

## 2. Motivation

### 2.1 What downstream needs

Embodied agents — avatars, MetaHumans, virtual presenters — need *full* facial animation driven by their own TTS output. Today the only path to lipsync that `atomr-infer` consumers can take is:

1. Generate speech audio via a TTS provider (out of band).
2. Take per-phoneme alignment from the TTS provider (e.g. ElevenLabs character timestamps).
3. Map phonemes → visemes → a hand-tuned ARKit-blendshape table.

This works for **mouth shapes** but loses everything else the face does while talking: brow raises, cheek squint, eye narrowing, jaw side-to-side, nostril flare, lip purse. The result is uncanny — the mouth animates, the rest of the face is dead. Production-quality avatar work needs the upper face too.

### 2.2 Why the model layer

NVIDIA Audio2Face-3D (and other audio-driven facial-animation services) already solves this: feed 16-kHz PCM audio, get back streaming 52-element Apple-ARKit-compatible blendshape weight vectors at 30 fps. It is, architecturally, a **model** — audio in, structured tensor out — and belongs behind `ModelRunner` for the same reasons LLMs do:

- A single observability surface (the inference layer already records latency, token/byte counts, errors).
- A single retry / circuit-breaker / cancellation story (`RunHandle`).
- A single runtime-selection knob (so callers can swap NVIDIA A2F for a local model, a fake for tests, or a future open-weights audio-to-face model without touching call sites).
- A single config surface (`RuntimeConfig`) so deployment knows how to wire it up.

### 2.3 What downstream looks like today

In this repo we ship `atomr-agents-avatar-harness`, which wraps an `AvatarInferenceClient` for the cognition (LLM) calls plus a TTS layer plus a viseme→ARKit-blendshape sink that drives a UE5 MetaHuman over LiveLink. That harness can already route LLM calls through `atomr-infer` cleanly. The audio→face path can't be: we currently ship a placeholder provider crate (`crates/avatar-provider-audio2face`) whose `Audio2FaceSink::new` returns `Audio2FaceError::Blocked` and references this FR. Once `RuntimeKind::Audio2Face` exists, that stub becomes a thin adapter over `ModelRunner` and the architectural invariant is preserved.

---

## 3. Proposed API

### 3.1 New `RuntimeKind` variant

```rust
// atomr-infer-core/src/runtime.rs
#[non_exhaustive]
pub enum RuntimeKind {
    Local,
    Remote,
    Fake,
    // … existing variants …

    /// Audio-in → ARKit-blendshape-out (e.g. NVIDIA Audio2Face-3D).
    /// Only available with the `audio2face` cargo feature.
    #[cfg(feature = "audio2face")]
    Audio2Face,
}
```

### 3.2 New batch shape

The existing `ExecuteBatch` is fundamentally **LLM-shaped**: `messages: Vec<Message>`, `sampling: SamplingParams`, `estimated_tokens`. None of those fields are meaningful for an audio→face call, and the response type (`tokens` / text) is also wrong. Trying to shoe-horn audio into `Message::content` as a new variant would force every existing LLM caller, runtime, and serializer to grow an `Audio` arm that they will never use, *and* would still leave `sampling` / `estimated_tokens` orphaned.

We strongly recommend a sibling type instead:

```rust
// atomr-infer-core/src/batch.rs
#[cfg(feature = "audio2face")]
#[derive(Debug, Clone)]
pub struct AudioBatch {
    /// Same request-id discipline as ExecuteBatch — flows into traces.
    pub request_id: String,

    /// Logical model name, e.g. "audio2face-3d/claire-v2.3". Runtime
    /// resolves this against its configured backend.
    pub model: String,

    /// Raw PCM audio. Streaming push is also fine (see §3.4).
    pub audio_pcm: Vec<u8>,

    /// Typically 16_000 for A2F-3D today; we expose it so future
    /// backends can negotiate.
    pub sample_rate_hz: u32,

    /// 1 = mono. A2F expects mono; reject != 1 at the runtime layer.
    pub channels: u8,

    /// Optional named emotion preset the backend supports
    /// ("neutral", "happy", "angry", "sad", …). String for now;
    /// see §8 for the enum-vs-string discussion.
    pub emotion_preset: Option<String>,

    /// Maps to A2F's `AnimationHeader.multiplier` — a global gain on
    /// all 52 weights. None = use RuntimeConfig default.
    pub multiplier: Option<f32>,

    /// When true, response is an async stream of BlendshapeChunks
    /// (§3.4). When false, the runtime collects the full sequence
    /// and returns it as one Vec<BlendshapeChunk>.
    pub stream: bool,
}
```

And the response:

```rust
#[cfg(feature = "audio2face")]
#[derive(Debug, Clone, Copy)]
pub struct BlendshapeChunk {
    /// Milliseconds since the start of this audio batch.
    pub timestamp_ms: u64,

    /// 52 Apple-ARKit-canonical-order weights, each in [0.0, 1.0].
    /// Order must match Apple's `ARFaceAnchor.BlendShapeLocation`
    /// enum (see references). Document the exact ordering in
    /// rustdoc — downstream consumes by index, not by name.
    pub weights: [f32; 52],
}
```

The fixed-size `[f32; 52]` is deliberate: ARKit's blendshape set is canonical, well-known, and unlikely to grow. It keeps the struct `Copy`, zero-alloc, and trivially `bytemuck`-able for downstream serialization (LiveLink, OSC, gRPC). If we ever need to support a non-ARKit rig, that is a new modality, not a vector-length change.

### 3.3 `ModelRunner` extension

Two options here, in preference order.

**Option A — separate trait method (preferred).** Keep `ModelRunner::execute(...)` untouched for LLM/vision callers, add a sibling method:

```rust
#[async_trait]
pub trait ModelRunner: Send + Sync {
    async fn execute(&self, batch: ExecuteBatch) -> Result<RunHandle>;

    #[cfg(feature = "audio2face")]
    async fn execute_audio(&self, batch: AudioBatch) -> Result<AudioRunHandle> {
        Err(Error::Unsupported("audio2face not implemented by this runtime"))
    }
}
```

A default impl returning `Unsupported` means **no existing `ModelRunner` impl needs to change** — they automatically advertise "I don't do audio" and the compiler is happy. Runtimes that *do* support audio override it. This is the same pattern Tokio uses for `AsyncRead`/`AsyncWrite` extension traits and is the lowest-friction additive change.

**Option B — make `execute` generic over a `Batch` trait.** More elegant long-term, much higher blast radius, would need to wait for a 1.0. Don't do this now.

### 3.4 Streaming handle

Mirror today's token-stream `RunHandle`:

```rust
#[cfg(feature = "audio2face")]
pub struct AudioRunHandle { /* … */ }

impl AudioRunHandle {
    /// Stream BlendshapeChunks as the backend emits them.
    /// Recommended cadence: one chunk per ~33 ms (30 fps), which
    /// matches A2F-3D's native output rate.
    pub fn into_stream(self)
        -> impl Stream<Item = Result<BlendshapeChunk>> + Send;

    /// Cancels the upstream gRPC call (same semantics as
    /// RunHandle::cancel today).
    pub async fn cancel(&self);
}
```

Downstream UE5 LiveLink consumers need a frame every 33 ms or they stall the avatar; chunking finer than that wastes wakeups, coarser than that stutters. 30 fps is the right default. If a future backend can do 60 fps, the stream just emits twice as often — the type doesn't change.

---

## 4. Runtime config

```rust
// atomr-infer-runtime/src/config.rs
#[cfg(feature = "audio2face")]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Audio2FaceRuntimeConfig {
    /// gRPC endpoint, e.g. "https://a2f.internal:50051".
    pub grpc_endpoint: String,

    /// Default emotion preset if AudioBatch::emotion_preset is None.
    #[serde(default)]
    pub emotion_preset_default: Option<String>,

    /// Default for AudioBatch::multiplier. A2F-3D ships with 1.0;
    /// most rigs want ~1.0–1.4 depending on the MetaHuman.
    #[serde(default = "default_multiplier")]
    pub multiplier_default: f32,

    /// Optional TLS / auth knobs — same shape as the existing
    /// Remote runtime config.
    #[serde(flatten)]
    pub tls: TlsConfig,
}

fn default_multiplier() -> f32 { 1.0 }
```

And in the `RuntimeConfig` enum:

```rust
pub enum RuntimeConfig {
    Local(LocalRuntimeConfig),
    Remote(RemoteRuntimeConfig),
    Fake(FakeRuntimeConfig),
    // …
    #[cfg(feature = "audio2face")]
    Audio2Face(Audio2FaceRuntimeConfig),
}
```

This keeps deployment YAML uniform — one `runtimes:` block, one entry per modality, same selection logic.

---

## 5. Streaming semantics

- `stream = true` returns `AudioRunHandle` whose stream yields one `BlendshapeChunk` per **~33 ms** of audio (30 fps), in audio-timeline order. The runtime is responsible for buffering and emitting on that cadence — the backend may chunk differently and the adapter normalises.
- `stream = false` collects the full timeline and returns `Vec<BlendshapeChunk>` (still 30 fps, just buffered). Useful for batch / offline workflows.
- Backpressure: same model as today's token stream — if the consumer doesn't pull, the runtime pauses gRPC reads. No silent drops.
- Cancellation: same as `RunHandle::cancel` — drops the gRPC stream upstream and `AudioRunHandle::into_stream()` terminates.
- Errors: `Result<BlendshapeChunk>` per item, so transient backend errors mid-stream surface to the consumer without tearing down the whole call. Terminal errors close the stream.

---

## 6. Backwards compatibility

- **Additive only.** No existing type changes shape. `ExecuteBatch`, `Message`, `RunHandle`, current `RuntimeKind` variants — all untouched.
- New `RuntimeKind::Audio2Face` is `#[non_exhaustive]`-gated and `cfg(feature = "audio2face")`-gated. Callers who don't enable the feature don't see it.
- New `ModelRunner::execute_audio` has a default `Err(Unsupported)` impl, so every existing `impl ModelRunner` continues to compile unchanged.
- The `audio2face` feature pulls in `tonic` + the A2F-3D `.proto`-generated client. None of that ships in the default build. Workspaces that already use `tonic` for the `Remote` runtime can share the dep; workspaces that don't (Fake / Local only) stay slim.
- MSRV unchanged.
- Public-API SemVer impact: **minor** (additive variants on `#[non_exhaustive]` enums, additive trait method with default body).

---

## 7. Migration plan

Three steps, each landable independently:

**Step 1 — types and feature gate (no behaviour).**
- Add the `audio2face` feature to `atomr-infer-core` and `atomr-infer-runtime`.
- Land `AudioBatch`, `BlendshapeChunk`, `AudioRunHandle`, `Audio2FaceRuntimeConfig`, and the `RuntimeKind::Audio2Face` / `RuntimeConfig::Audio2Face` variants. All behind `cfg(feature = "audio2face")`.
- Add `ModelRunner::execute_audio` with the `Unsupported` default.
- Ship a `FakeRuntime` impl that emits a deterministic 30-fps sine-wave-shaped blendshape stream so downstream can wire integration tests without a GPU.
- Cut a minor release. Downstream can adopt the types and start writing adapter code against the fake.

**Step 2 — NVIDIA Audio2Face-3D backend.**
- Add a new `atomr-infer-runtime-audio2face` crate (or a module inside `atomr-infer-runtime` gated on the feature). Vendor or fetch the A2F-3D `.proto`. Implement `ModelRunner::execute_audio` against the gRPC streaming RPC.
- Map `AudioBatch.emotion_preset` → A2F's emotion field, `AudioBatch.multiplier` → `AnimationHeader.multiplier`.
- Translate A2F's blendshape ordering into ARKit canonical order at the boundary, so downstream `[f32; 52]` is always ARKit-ordered regardless of backend.
- x86_64 + NVIDIA-GPU is the deployment target; `cfg(target_arch = "x86_64")` the crate so workspace builds on aarch64 dev machines don't fail (downstream is already doing this for its stub provider — see references).

**Step 3 — docs and example.**
- Add a small `examples/audio2face_stream.rs` that loads a `.wav`, runs it through the runtime, and prints the first second of blendshape frames as JSON.
- Document the canonical ARKit ordering in rustdoc on `BlendshapeChunk::weights` — link Apple's reference. Downstream consumes by index, so this is load-bearing.

---

## 8. Open questions

1. **Emotion preset typing.** `Option<String>` is the lowest-friction choice and matches what A2F-3D actually accepts on the wire, but it punts validation to runtime and makes IDE autocomplete useless. Alternative: an `EmotionPreset` enum with a `Custom(String)` escape hatch. The enum is nicer but risks lagging behind whatever presets NVIDIA ships next. Recommend starting with `String` and revisiting once we have a second backend to compare against.

2. **Fused audio-out + face-out batches.** A natural next step is "give me text, return synchronized speech audio *and* blendshape frames in one streamed response" — i.e. fuse TTS and Audio2Face into a single inference call so the consumer never has to align them client-side. Should that be a third modality (`Text2AvatarBatch`), or should `AudioBatch` grow a `text: Option<String>` field that, when present, makes `audio_pcm` an output rather than input? The former is cleaner; the latter is fewer types. Downstream avatar harnesses would benefit either way — this is a real pain point today.

3. **Generic multi-modal batches.** Longer-term: should `RuntimeKind` accept a generic `Batch` trait so adding modality N+1 doesn't require adding `execute_X` to `ModelRunner`? That's the "Option B" from §3.3. Pros: one trait, infinite modalities. Cons: object-safety pain, type-erasure overhead, harder for runtimes to advertise capabilities statically. Probably worth a separate design doc once we have 3+ modalities in flight (text, vision, audio2face, TTS, ASR).

4. **Backend negotiation of frame rate.** We've hard-coded 30 fps in the streaming contract. If a future backend emits 60 fps natively, do we downsample at the runtime (lossy, simple) or expose `target_fps` on `AudioBatch` (correct, more API surface)? Not urgent; flagging.

5. **Sample-rate handling.** A2F-3D wants 16 kHz mono. Should the runtime resample inputs that arrive at 24/44.1/48 kHz, or reject them and force the caller to resample? Resampling at the runtime is more user-friendly but pulls in a resampler dep. Lean toward "reject with a clear error in v1, add opt-in resampling later."

---

## 9. References

- **NVIDIA Audio2Face-3D microservice gRPC API** — the reference backend this FR is shaped around. Streaming RPC, 16 kHz PCM in, 52-element ARKit-compatible blendshape weights at 30 fps out, plus `AnimationHeader.multiplier` and emotion preset string.
- **Apple ARKit blendshape canonical ordering and semantics** — <https://developer.apple.com/documentation/arkit/arfaceanchor/blendshapelocation>. The 52-element order in `BlendshapeChunk::weights` must match this enum's declaration order; downstream consumers index by integer, not by name.
- **Downstream stub provider crate** — `crates/avatar-provider-audio2face/src/lib.rs` in the `atomr-agents` workspace. `Audio2FaceSink::new` currently returns `Audio2FaceError::Blocked` and points at this FR. Once `RuntimeKind::Audio2Face` lands, that crate becomes a ~100-line adapter that constructs an `AudioBatch`, drives `ModelRunner::execute_audio`, and forwards `BlendshapeChunk`s into the existing `AvatarSink` channel.
- **Downstream consumer pattern** — `crates/agent/src/inference.rs` in `atomr-agents` shows how a real consumer wraps a `ModelRunner` today; the audio path will follow the same shape.

---

*If the upstream maintainers want to discuss the API shape before implementation, the avatar working group is happy to prototype Step 1 as a PR against `atomr-infer-core` for review.*
