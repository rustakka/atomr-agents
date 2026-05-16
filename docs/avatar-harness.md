# Avatar harness

The avatar harness gives an agent a real-time, visually rendered
embodiment — typically an Unreal Engine 5 MetaHuman driven from outside
UE5 over a length-prefixed CBOR-over-UDP Live Link stream. It composes
**perception** (STT or pre-transcribed text) → **cognition** (a
JSON-enveloped LLM turn through atomr-infer) → **synthesis** (any
`TextToSpeech` backend) → **sync-manager** (PCM + visemes → ARKit
blendshapes + SMPTE timecode) → **`AvatarSink`** (the wire), supervised
as a Tokio actor pipeline.

This guide covers operator setup on Ubuntu, the architecture rules for
where the harness can run (x86_64 vs aarch64), the current 2026
MetaHuman authoring workflow (in-engine — the web Creator has been
discontinued), a full skeleton for the UE5-side `ILiveLinkSource`
receiver plugin, end-to-end execution, tuning knobs, and the extension
points (custom sinks, custom visemes, custom TTS/STT, custom cognition
envelopes).

## 1. Overview

```text
   ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌──────────────┐    ┌────────────┐
   │  Perception │───►│  Cognition  │───►│  Synthesis  │───►│ SyncManager  │───►│ AvatarSink │
   │ (STT/text)  │    │ atomr-infer │    │  TTS+visemes│    │ 60Hz frames  │    │ UDP→UE5 MH │
   └─────────────┘    └─────────────┘    └─────────────┘    └──────────────┘    └────────────┘
                            │
                            ▼
                      ┌──────────────┐
                      │ EmotionState │ ◄── apply(delta, decay) per turn
                      └──────────────┘
```

Per turn: an `Utterance` flows in; cognition returns a structured
`AgentIntentPacket { response_text, emotion_delta, gesture }`; emotion
state updates; synthesis produces PCM + a viseme track; the sync
manager slices that into `AvatarFrame`s at the configured frame rate,
overlays the running emotion vector, and pushes each frame into the
sink's mpsc channel. Each `AvatarFrame` is encoded as
`[u32 length LE][CBOR(WireFrame)]` and shipped to the UE5 receiver.

### Crate map

| Crate | Role | Arch |
| --- | --- | --- |
| `atomr-agents-avatar-core` | Domain types — `BlendshapeWeights`, `Viseme`, `AvatarFrame`, `AvatarSink` trait, CBOR wire format. | any |
| `atomr-agents-avatar-harness` | Orchestrator — `AvatarHarnessBuilder`, perception / cognition / synthesis / sync actors. | any (body); providers cfg-gated to x86_64 |
| `atomr-agents-avatar-provider-livelink` | UDP Live Link sink (`LiveLinkSink`) — the production transport to UE5. | x86_64 only |
| `atomr-agents-avatar-provider-audio2face` | NVIDIA Audio2Face-3D sink — **stub**, blocked on FR-A2F-001 upstream. | x86_64 only |

The harness body builds on every arch atomr-agents builds on; **only
the optional provider crates are cfg-gated to x86_64**, because the
underlying transports they wrap (UE5 Live Link plugin tooling, NVIDIA
Audio2Face) are x86_64-only at the vendor level.

## 2. Architecture & arch requirements (x86 vs ARM)

### Host running the avatar harness

- **Linux x86_64 — recommended** for the full bundled experience. All
  feature combinations (`providers-livelink`, `providers-a2f`,
  `providers-all`) compile and run.
- **Linux aarch64 / macOS arm64 — body-only.** The harness compiles
  but the shipped sinks do not. See the cfg-gate in
  `crates/avatar-harness/Cargo.toml:46-48`:

  ```toml
  [target.'cfg(target_arch = "x86_64")'.dependencies]
  atomr-agents-avatar-provider-livelink   = { workspace = true, optional = true }
  atomr-agents-avatar-provider-audio2face = { workspace = true, optional = true }
  ```

  On aarch64 the operator must supply a custom `AvatarSink` (a
  WebRTC sink, a file-recording sink, etc. — see §8.1). The cargo
  feature flags accept silently on aarch64 because the dep itself is
  arch-gated; nothing is pulled in but the build still succeeds.

### Host running UE5 + MetaHuman

- **x86_64 only** for officially supported Epic Linux/Windows binaries.
- Linux ARM64 / Apple Silicon ports exist as community forks (e.g.
  `raskolnikoff/UnrealEngine-arm64`) but are **out of scope** for this
  guide — Epic does not ship ARM64 binaries and MetaHuman tooling is
  not validated against them.

### Same machine vs split

Most operators co-locate: a single Ubuntu x86_64 box runs both the
harness and the UE5 editor; the UDP default `127.0.0.1:6666` is just a
loopback. Split-host setups (Linux harness → Windows UE5, or vice
versa) work transparently — the wire is pure UDP+CBOR. To target a
remote UE5 instance, override `LiveLinkConfig::addr`
(`crates/avatar-provider-livelink/src/config.rs:16-21`); open UDP 6666
in the firewall between the two boxes.

### GPU

- The **harness host does not need a GPU**. Cognition runs through
  atomr-infer (any backend — hosted Anthropic/OpenAI, local llama.cpp,
  …); TTS is whatever you wire. CPU-only deployments are supported.
- The **UE5 host needs a Vulkan-capable GPU** (NVIDIA recommended for
  MetaHuman). On Linux this means the proprietary NVIDIA driver in
  practice — Nouveau lacks Vulkan parity.
- The **Audio2Face sink will require an NVIDIA RTX-class GPU on the
  Audio2Face microservice host** once unblocked. Currently a stub:
  `Audio2FaceSink::new()` returns `Audio2FaceError::Blocked` at
  `crates/avatar-provider-audio2face/src/lib.rs:79-84`.

## 3. Ubuntu environment setup

Tested on Ubuntu 24.04 LTS (Noble) and 22.04 LTS (Jammy). The host
preference for batched privilege escalation is honoured throughout —
each privileged step is a single `pkexec bash -c '…'` (or `sudo bash
-c '…'`) so the password / polkit prompt fires once.

### 3.1 Base toolchain

```bash
# Rust (pinned by rust-toolchain.toml in the workspace root)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# System packages — one elevated batch
pkexec bash -c '
  apt-get update &&
  apt-get install -y \
    pkg-config libssl-dev cmake protobuf-compiler \
    libasound2-dev libopus-dev \
    python3 python3-pip python3-venv
'

# Python (≥3.10) for the bindings + smoke tests
python3 -m venv .venv
source .venv/bin/activate
pip install -U pip maturin pytest
```

What each piece is for: `libasound2-dev` for any STT/TTS runtime that
links ALSA; `libopus-dev` for Opus-aware audio codecs; `protobuf-compiler`
for atomr-infer's gRPC modalities; `cmake` for ciborium / tokenizer
build deps.

### 3.2 Workspace build

The avatar crates build standalone — you don't need the rest of the
workspace, although `cargo build --workspace` works too.

```bash
# Bare harness (no transport)
cargo build -p atomr-agents-avatar-harness

# With UDP Live Link sink to UE5 (recommended on x86_64)
cargo build -p atomr-agents-avatar-harness --features providers-livelink

# Everything that compiles on this arch
cargo build -p atomr-agents-avatar-harness --features providers-all

# Run the in-tree end-to-end tests
cargo test -p atomr-agents-avatar-harness
cargo test -p atomr-agents-avatar-core
cargo test -p atomr-agents-avatar-provider-livelink   # x86_64 only
```

On aarch64 the `providers-livelink` and `providers-a2f` features
resolve to no extra deps; the build succeeds and the sink types just
aren't available — bring your own `AvatarSink`.

### 3.3 Python bindings

```bash
# From the workspace root, with the venv active
maturin develop \
  --manifest-path crates/py-bindings/Cargo.toml \
  --features avatar

# Smoke test
pytest python/atomr_agents/tests/test_avatar.py -v
```

`test_avatar.py` (`python/atomr_agents/tests/test_avatar.py:19-23`)
auto-skips on arm64. Note that **CI matrices that publish maturin
wheels must include `aarch64-unknown-linux-gnu`** alongside x86_64 —
the avatar facade itself imports cleanly on arm64 (it returns
`is_available() == False`), but downstream packaging breaks if you
only ship x86_64 wheels.

### 3.4 UE5 install on Ubuntu

Epic Games Launcher is Windows/Mac only; on Linux you download the UE5
Linux binary tarball directly:

1. Create an Epic Games account; link it to your GitHub account at
   <https://www.unrealengine.com/en-US/ue4-on-github>. This grants
   access to the `EpicGames` GitHub org.
2. Either:
   - Download the **prebuilt Linux binary** tarball from Epic's "Linux
     downloads" page (UE 5.6 LTS minimum; UE 5.7 recommended for
     Linux MetaHuman Creator + Python batch API), or
   - Build from source: `git clone https://github.com/EpicGames/UnrealEngine`,
     run `Setup.sh`, then `GenerateProjectFiles.sh`, then `make`.
     Source builds take 1–2 hours on a modern workstation and ~150 GB
     of disk.
3. Required runtime packages — one elevated batch:

   ```bash
   pkexec bash -c '
     apt-get update &&
     apt-get install -y \
       libvulkan1 mesa-vulkan-drivers vulkan-tools \
       libsdl2-2.0-0 libxcb-xkb1 libxkbcommon-x11-0 \
       libfreetype6 libfontconfig1 libglu1-mesa \
       libgtk-3-0
   '
   ```

4. NVIDIA proprietary driver recommended. Verify with `vulkaninfo
   --summary | head` — you should see your RTX/GTX GPU listed as a
   Vulkan-capable device.
5. **Prefer X11 over Wayland** for the UE editor session — the content
   browser drag-drops can be flaky on Wayland in 5.6/5.7. Toggle at
   login from your display manager.

### 3.5 Firewall / port

The Live Link UDP transport defaults to **UDP 6666** between the
harness host and the UE5 host. Default from
`crates/avatar-provider-livelink/src/config.rs:32-34`. Open it both
ways if your harness and UE5 are on different boxes:

```bash
pkexec bash -c 'ufw allow 6666/udp comment "atomr-agents avatar Live Link"'
```

For a same-machine setup (default `127.0.0.1:6666`) no firewall change
is needed.

## 4. MetaHuman creation guide (current 2026 workflow)

> **Important context.** As of MetaHuman 5.6 (June 2025) the **standalone
> web-based MetaHuman Creator is sunset** and Quixel Bridge is no
> longer the delivery mechanism. All new character authoring runs
> **inside Unreal Editor** via the MetaHuman plugin shipped with UE 5.6+.
> The legacy web Creator hard-shuts-down on **2026-11-05**; existing
> characters get a 90-day download window after that.
> MetaHuman 5.7 (Nov 2025) added Linux support for the in-engine Creator
> plus a Python/Blueprint batch API and FBX round-tripping, so the
> workflow below works on either Windows or Linux.

### 4.1 What you need

- **Unreal Engine 5.6 LTS minimum** — 5.7 recommended for Linux Creator
  parity and the batch API.
- **Epic Games account** linked to GitHub if you're building UE from
  source.
- **Disk**: ~150 GB for the engine; ~10 GB per MetaHuman.
- **GPU**: Vulkan-capable (Linux) or DirectX 12 (Windows). NVIDIA
  RTX-class is strongly recommended for MetaHuman material complexity.

### 4.2 Install the MetaHuman plugin

From inside Unreal Editor:

1. `Edit` → `Plugins` → search for **MetaHuman**.
2. Enable it; click **Restart Now**.
3. From 5.6 onward, no external service handshake is required — the
   Creator UI is a dockable editor tab (`Window` → `MetaHuman`).

### 4.3 Windows authoring path (recommended for first run)

1. Create a new UE5 project (Games → Blank → C++ or Blueprint).
2. Enable the MetaHuman plugin per §4.2.
3. Open **Window → MetaHuman Creator** → pick a preset → sculpt face
   and body in the dockable tab.
4. **Assemble character to project** (replaces the legacy "export
   from web Creator → import via Quixel Bridge" step). The character
   appears as a Blueprint actor under `Content/MetaHumans/<Name>/`.
5. **Validate Face_AnimBP and the Face Control Board curves.** Open
   `Content/MetaHumans/Common/Face/Face_AnimBP`; confirm the curve
   list includes ARKit-named curves (`eyeBlinkLeft`, `jawOpen`,
   `mouthSmileLeft`, …). MetaHumans ship with the full 52-curve set,
   ordered to match Apple's canonical enum.

### 4.4 Linux authoring path (UE 5.7+)

Identical UX inside the editor. Linux-specific gotchas:

- Use an **X11 session**, not Wayland — content browser drag-drops are
  flaky on Wayland; the Creator viewport works either way but the
  asset operations matter.
- Expect occasional Vulkan validation warnings in the log on launch;
  they don't affect the Creator.
- If font rendering looks off, install `fonts-noto` and restart the
  editor.
- The Linux Creator hits parity with Windows in 5.7 — same panels,
  same blendshape coverage, same FBX round-trip.

### 4.5 Mesh-to-MetaHuman / Body Conform (alternative)

If you have a face scan or sculpt, you can drive a MetaHuman *from
your geometry* rather than the preset library: `MetaHuman` →
`Mesh to MetaHuman`. This conforms a fitted MetaHuman to your mesh
topology. Out of scope for runtime pipeline configuration here, but
it's the recommended path for branded / custom likenesses.

### 4.6 Runtime validation

1. Drag the MetaHuman Blueprint actor into a level.
2. Open the assembled character's Face Blueprint.
3. Wire a `LiveLinkPose` node into the Face_AnimBP graph, bound to
   a Live Link subject name you'll have your receiver plugin publish
   (recommend `AtomrAvatar`).
4. In **Live Link** (`Window → Virtual Production → Live Link`),
   confirm the subject shows up once the harness starts streaming —
   green-bullet status, frame rate matching `SyncConfig::frame_rate`
   (default 60 Hz).

### 4.7 Migrating legacy web-Creator characters

If you have characters authored on the pre-5.6 web Creator:

- Download them from your MetaHuman library **before 2026-11-05** +
  the 90-day grace window. Epic's migration docs cover the import
  workflow into the in-engine MetaHuman plugin.
- The 52-blendshape rig itself is unchanged across versions — once
  migrated, the wire format and `Face_AnimBP` curve names are
  identical.

### 4.8 Note on Linux ARM64 hosts for authoring

Not supported by Epic binaries. If your Linux dev box is ARM64, do
MetaHuman authoring on a separate **Windows or Linux x86_64** machine
and only deploy the packaged project (or the receiver plugin) to
Linux x86_64 for runtime.

## 5. UE5 ILiveLinkSource receiver plugin

The plugin is **not** in this repository — the operator authors it
inside their UE5 project. Below is a complete skeleton that decodes the
exact wire format the harness emits.

### 5.1 What this plugin does

1. Listens on UDP `6666` for length-prefixed CBOR datagrams.
2. For each datagram: parse the 4-byte LE length prefix, CBOR-decode
   the `WireFrame`, version-check, then split into:
   - 52 ARKit blendshape weights → pushed into Live Link as a
     `FLiveLinkAnimationFrameData`, named subject `AtomrAvatar`.
   - PCM `s16le` audio → queued into a `USoundWaveProcedural` driving
     the MetaHuman's `UAudioComponent`.
   - SMPTE timecode → embedded on the frame as a `FQualifiedFrameTime`
     so Sequencer / Take Recorder pick it up.
3. Live Link's binding to `Face_AnimBP` drives all 52 face curves
   automatically; the audio component plays in sync.

### 5.2 Wire format (restated from `avatar-core/src/wire.rs`)

```text
   ┌────┬───────────────────────────┐
   │ N  │     CBOR(WireFrame)       │
   └────┴───────────────────────────┘
     4B            N bytes      (N = u32 LE)
```

`WireFrame` (CBOR struct, field order stable —
`crates/avatar-core/src/wire.rs:48-63`):

| Field | Type | Notes |
| --- | --- | --- |
| `version` | `u16` | `1` today (`WIRE_FORMAT_VERSION`). Reject mismatches. |
| `timecode` | `SmpteTimecode { hours: u8, minutes: u8, seconds: u8, frames: u8, frame_rate: u8 }` | Non-drop, integer rate. |
| `audio` | `AudioChunk { samples_s16le: bytes, sample_rate_hz: u32, channels: u8 }` | Interleaved s16 LE. |
| `weights` | `[f32; 52]` (CBOR tuple) | ARKit order — see §5.5. |
| `emotion` (opt) | `EmotionVector { valence, arousal, anger, surprise, tension : f32 }` | Skipped if absent. |
| `body` (opt) | `BodyRigHint { curves: map<string,f32> }` | Optional named curves. |

The CBOR encoding is via **ciborium**; any conformant CBOR decoder
(TinyCBOR, QCBOR, cbor11) will read it. We recommend TinyCBOR for the
UE plugin — small, MIT-licensed, single-file vendoring.

### 5.3 `.uplugin` manifest

`AtomrAvatarLiveLink.uplugin`:

```json
{
  "FileVersion": 3,
  "Version": 1,
  "VersionName": "0.1.0",
  "FriendlyName": "Atomr Avatar Live Link",
  "Description": "Receives CBOR-framed AvatarFrames over UDP and drives a MetaHuman Face_AnimBP + audio component.",
  "Category": "Animation",
  "CreatedBy": "your-org",
  "EnabledByDefault": true,
  "CanContainContent": false,
  "IsBetaVersion": true,
  "Installed": false,
  "Modules": [
    {
      "Name": "AtomrAvatarLiveLink",
      "Type": "Runtime",
      "LoadingPhase": "PostEngineInit",
      "WhitelistPlatforms": [ "Win64", "Linux" ]
    }
  ],
  "Plugins": [
    { "Name": "LiveLink",          "Enabled": true },
    { "Name": "LiveLinkInterface", "Enabled": true }
  ]
}
```

Module dependencies in `AtomrAvatarLiveLink.Build.cs`:

```cs
using UnrealBuildTool;

public class AtomrAvatarLiveLink : ModuleRules
{
    public AtomrAvatarLiveLink(ReadOnlyTargetRules Target) : base(Target)
    {
        PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
        PublicDependencyModuleNames.AddRange(new string[] {
            "Core", "CoreUObject", "Engine",
            "LiveLink", "LiveLinkInterface", "LiveLinkAnimationCore",
            "Networking", "Sockets",
            "AudioMixer", "AudioPlatformConfiguration",
            "TimeManagement"
        });
        // Vendor TinyCBOR under ThirdParty/tinycbor; expose its include
        // directory here.
        PublicIncludePaths.Add(System.IO.Path.Combine(ModuleDirectory, "../ThirdParty/tinycbor"));
    }
}
```

### 5.4 `FAtomrAvatarLiveLinkSource` skeleton

Header (`Public/AtomrAvatarLiveLinkSource.h`):

```cpp
#pragma once

#include "CoreMinimal.h"
#include "ILiveLinkSource.h"
#include "Sockets.h"
#include "Common/UdpSocketReceiver.h"
#include "Misc/QualifiedFrameTime.h"

class FAtomrAvatarLiveLinkSource : public ILiveLinkSource
{
public:
    FAtomrAvatarLiveLinkSource(const FString& InEndpoint, int32 InPort);
    virtual ~FAtomrAvatarLiveLinkSource() override;

    // ILiveLinkSource
    virtual void ReceiveClient(ILiveLinkClient* InClient, FGuid InSourceGuid) override;
    virtual bool IsSourceStillValid() const override { return Socket != nullptr; }
    virtual bool RequestSourceShutdown() override;
    virtual FText GetSourceType()    const override { return NSLOCTEXT("AtomrAvatar","Type","Atomr Avatar"); }
    virtual FText GetSourceMachineName() const override { return FText::FromString(Endpoint); }
    virtual FText GetSourceStatus()  const override { return Status; }

private:
    void OnDatagram(const FArrayReaderPtr& Data, const FIPv4Endpoint& From);
    void PushPose(const TArray<float>& Weights52, const FQualifiedFrameTime& Tc);
    void EnqueueAudio(const uint8* Pcm, int32 Bytes, int32 SampleRateHz, int32 Channels);

    ILiveLinkClient* Client = nullptr;
    FGuid SourceGuid;
    FSocket* Socket = nullptr;
    TUniquePtr<FUdpSocketReceiver> Receiver;
    FString Endpoint;
    int32 Port = 6666;
    FName SubjectName = TEXT("AtomrAvatar");
    FText Status;
};
```

Implementation outline (`Private/AtomrAvatarLiveLinkSource.cpp`):

```cpp
#include "AtomrAvatarLiveLinkSource.h"

#include "ILiveLinkClient.h"
#include "Roles/LiveLinkAnimationRole.h"
#include "Roles/LiveLinkAnimationTypes.h"
#include "Networking.h"
#include "Common/UdpSocketBuilder.h"
#include "tinycbor.h"   // vendored under ThirdParty/tinycbor

static constexpr uint16 kWireVersion = 1;

// ARKit-52 names — Apple's canonical order, matching avatar-core::ArkitBlendshape.
static const TArray<FName>& ArkitNames()
{
    static const TArray<FName> Names = {
        TEXT("eyeBlinkLeft"),  TEXT("eyeLookDownLeft"), TEXT("eyeLookInLeft"),
        TEXT("eyeLookOutLeft"),TEXT("eyeLookUpLeft"),   TEXT("eyeSquintLeft"),
        TEXT("eyeWideLeft"),   TEXT("eyeBlinkRight"),   TEXT("eyeLookDownRight"),
        TEXT("eyeLookInRight"),TEXT("eyeLookOutRight"), TEXT("eyeLookUpRight"),
        TEXT("eyeSquintRight"),TEXT("eyeWideRight"),
        TEXT("jawForward"),    TEXT("jawLeft"),         TEXT("jawRight"),
        TEXT("jawOpen"),
        TEXT("mouthClose"),    TEXT("mouthFunnel"),     TEXT("mouthPucker"),
        TEXT("mouthLeft"),     TEXT("mouthRight"),
        TEXT("mouthSmileLeft"),TEXT("mouthSmileRight"),
        TEXT("mouthFrownLeft"),TEXT("mouthFrownRight"),
        TEXT("mouthDimpleLeft"),TEXT("mouthDimpleRight"),
        TEXT("mouthStretchLeft"),TEXT("mouthStretchRight"),
        TEXT("mouthRollLower"),TEXT("mouthRollUpper"),
        TEXT("mouthShrugLower"),TEXT("mouthShrugUpper"),
        TEXT("mouthPressLeft"),TEXT("mouthPressRight"),
        TEXT("mouthLowerDownLeft"),TEXT("mouthLowerDownRight"),
        TEXT("mouthUpperUpLeft"), TEXT("mouthUpperUpRight"),
        TEXT("browDownLeft"),  TEXT("browDownRight"),   TEXT("browInnerUp"),
        TEXT("browOuterUpLeft"),TEXT("browOuterUpRight"),
        TEXT("cheekPuff"),     TEXT("cheekSquintLeft"), TEXT("cheekSquintRight"),
        TEXT("noseSneerLeft"), TEXT("noseSneerRight"),
        TEXT("tongueOut")
    };
    check(Names.Num() == 52);
    return Names;
}

FAtomrAvatarLiveLinkSource::FAtomrAvatarLiveLinkSource(const FString& InEndpoint, int32 InPort)
    : Endpoint(InEndpoint), Port(InPort)
{
    FIPv4Endpoint Bind;
    FIPv4Endpoint::Parse(FString::Printf(TEXT("%s:%d"), *Endpoint, Port), Bind);

    Socket = FUdpSocketBuilder(TEXT("AtomrAvatarRecv"))
        .AsReusable()
        .BoundToEndpoint(Bind)
        .WithReceiveBufferSize(1 << 20);

    if (Socket)
    {
        Receiver = MakeUnique<FUdpSocketReceiver>(Socket, FTimespan::FromMilliseconds(5), TEXT("AtomrAvatarRecvThread"));
        Receiver->OnDataReceived().BindRaw(this, &FAtomrAvatarLiveLinkSource::OnDatagram);
        Receiver->Start();
        Status = NSLOCTEXT("AtomrAvatar","Status","Listening");
    }
    else
    {
        Status = NSLOCTEXT("AtomrAvatar","Status","Bind failed");
    }
}

void FAtomrAvatarLiveLinkSource::OnDatagram(const FArrayReaderPtr& Data, const FIPv4Endpoint&)
{
    const TArray<uint8>& Bytes = *Data;
    if (Bytes.Num() < 4) return;

    const uint32 Len = (uint32(Bytes[0])      ) | (uint32(Bytes[1]) <<  8)
                     | (uint32(Bytes[2]) << 16) | (uint32(Bytes[3]) << 24);
    if (uint32(Bytes.Num() - 4) < Len) return; // partial — UDP shouldn't fragment for our sizes

    // CBOR decode the next `Len` bytes into version, timecode, audio, weights, emotion?, body?
    CborParser P; CborValue Root;
    if (cbor_parser_init(Bytes.GetData() + 4, Len, 0, &P, &Root) != CborNoError) return;
    if (!cbor_value_is_map(&Root) && !cbor_value_is_array(&Root)) return;

    // The Rust struct serializes as a CBOR map by default with serde —
    // walk fields by name. Helpers (omitted for brevity) parse each:
    //   uint16  version
    //   struct  timecode {hours,minutes,seconds,frames,frame_rate}
    //   struct  audio    {samples_s16le: bytes, sample_rate_hz: u32, channels: u8}
    //   array   weights  [f32; 52]
    //   struct? emotion  {valence,arousal,anger,surprise,tension}
    //   struct? body     {curves: map<string,f32>}
    //
    // After decode: validate version == kWireVersion, then:
    FQualifiedFrameTime Tc(/*hh:mm:ss:ff at frame_rate*/);
    TArray<float> W; W.SetNumZeroed(52);
    // ... copy weights ...

    PushPose(W, Tc);
    // EnqueueAudio(audio_ptr, audio_len, sample_rate_hz, channels);
}

void FAtomrAvatarLiveLinkSource::PushPose(const TArray<float>& W, const FQualifiedFrameTime& Tc)
{
    if (!Client) return;

    // First time only: declare the subject + static blendshape names.
    static bool bStaticPushed = false;
    if (!bStaticPushed)
    {
        FLiveLinkStaticDataStruct Static(FLiveLinkBaseStaticData::StaticStruct());
        FLiveLinkBaseStaticData& Data = *Static.Cast<FLiveLinkBaseStaticData>();
        Data.PropertyNames = ArkitNames();
        Client->PushSubjectStaticData_AnyThread(
            { SourceGuid, SubjectName }, ULiveLinkBasicRole::StaticClass(), MoveTemp(Static));
        bStaticPushed = true;
    }

    FLiveLinkFrameDataStruct Frame(FLiveLinkBaseFrameData::StaticStruct());
    FLiveLinkBaseFrameData& F = *Frame.Cast<FLiveLinkBaseFrameData>();
    F.PropertyValues = W;
    F.WorldTime = FPlatformTime::Seconds();
    F.MetaData.SceneTime = Tc;

    Client->PushSubjectFrameData_AnyThread({ SourceGuid, SubjectName }, MoveTemp(Frame));
}
```

Notes on the C++:

- `LiveLinkAnimationCore` / `LiveLinkBasicRole` is enough — we don't
  need the full `AnimationRole` skeletal hierarchy because we're
  driving named curves, not bones.
- `PushSubjectStaticData_AnyThread` runs once to publish the 52 curve
  names; thereafter every `PushSubjectFrameData_AnyThread` is just a
  52-float vector aligned to those names.
- For the audio path, allocate a `USoundWaveProcedural` once on the
  game thread and call `QueueAudio(Pcm, Bytes)` from the receiver
  thread. Bind it to the MetaHuman pawn's `UAudioComponent` in
  Blueprint or in `BeginPlay`.
- The SMPTE timecode is non-drop and integer-rate (always exactly
  matches `SyncConfig::frame_rate`), so the mapping to
  `FQualifiedFrameTime` is just `FFrameTime(frames)` at
  `FFrameRate(frame_rate, 1)`.

### 5.5 Face_AnimBP wiring

Inside `Face_AnimBP`:

1. Add a `Live Link Pose` node, set **Live Link Subject** to
   `AtomrAvatar`, role = `Basic`.
2. The 52 property names produced by the static push (§5.4) match the
   MetaHuman face control board curve names 1:1 — no remap node is
   needed.
3. Plug `Live Link Pose` into `Output Pose`. The Face Control Board
   converts the curves into rig-space deltas automatically.

### 5.6 Build & package

```bash
# Linux UE5
RunUAT.sh BuildPlugin \
  -Plugin=/path/to/AtomrAvatarLiveLink/AtomrAvatarLiveLink.uplugin \
  -Package=/path/to/output -TargetPlatforms=Linux

# Windows UE5
RunUAT.bat BuildPlugin ^
  -Plugin=C:\path\AtomrAvatarLiveLink\AtomrAvatarLiveLink.uplugin ^
  -Package=C:\path\output -TargetPlatforms=Win64
```

Drop the resulting plugin tree under your project's `Plugins/` folder
and restart the editor.

### 5.7 Testing the receive loop

The Rust E2E pattern from `crates/avatar-harness/tests/end_to_end.rs`
exercises the full pipeline against a `CapturingSink`. For the UE side,
the simplest end-to-end test is:

1. Start UE5 with the plugin enabled; place a MetaHuman in a level
   with `Face_AnimBP` bound to subject `AtomrAvatar`.
2. Open `Window → Virtual Production → Live Link`; you should see
   subject `AtomrAvatar` light up green once the harness streams.
3. Run `cargo run -p atomr-agents-avatar-harness --features
   providers-livelink --example <your_demo>` (operator-provided);
   confirm:
   - `stat livelink` in the UE console shows a frame rate matching
     `SyncConfig::frame_rate`.
   - The MetaHuman's mouth tracks the synthesized audio.
   - `Take Recorder` captures the run if armed (SMPTE timecode is
     present on every frame).

## 6. Running end-to-end

### Rust (library use)

```rust
use std::sync::Arc;
use atomr_agents_avatar_core::AvatarSink;
use atomr_agents_avatar_harness::{
    AvatarHarnessBuilder, AvatarHarnessConfig, CognitionConfig,
    cognition::AvatarInferenceClient,
};
use atomr_agents_avatar_provider_livelink::{LiveLinkConfig, LiveLinkSink};
use atomr_agents_tts_core::{DynTextToSpeech, VoiceRef};

# async fn demo(
#   inference: Arc<dyn AvatarInferenceClient>,
#   tts: DynTextToSpeech,
# ) -> atomr_agents_avatar_core::Result<()> {
let harness = AvatarHarnessBuilder::new()
    .with_inference(inference)
    .with_cognition_config(CognitionConfig::default())
    .with_tts(tts, VoiceRef::named("alloy"))
    .with_config(AvatarHarnessConfig::default())
    .build()?;

let sink: Arc<dyn AvatarSink> = Arc::new(LiveLinkSink::new(LiveLinkConfig::loopback()));
harness.attach_sink(sink).await?;

// Drive a turn — cognition → TTS → sync → UDP frames to UE5.
harness.user_said("Hi! Tell me a joke about Vulkan drivers.").await?;

// ... eventually ...
harness.shutdown().await?;
# Ok(())
# }
```

### Python

```python
import asyncio
from atomr_agents.avatar import AvatarHarness, CapturingSink
from atomr_agents.tts import TextToSpeech

async def my_inference(batch):
    return '{"response_text": "Hello!", "emotion_delta": {"valence": 0.4}}'

async def main():
    tts = TextToSpeech.mock()
    harness = AvatarHarness(my_inference, tts, "alloy", frame_rate=60)
    sink = CapturingSink()
    await harness.attach_sink(sink.as_sink())
    await harness.user_said("hello there")
    # frames will appear on the sink — drain at your cadence
    print(await harness.last_intent())
    await harness.shutdown()

asyncio.run(main())
```

(For a Live Link UDP sink in Python, build the wheel with
`maturin develop --features avatar-livelink`; the
`atomr_agents.avatar.LiveLinkSink` factory then becomes available —
see `python/atomr_agents/avatar.py:34-35`.)

### Logs to grep

The harness uses the `atomr_agents_avatar_harness` tracing target.
Useful patterns:

- `"avatar turn failed"` — non-fatal per-turn error
  (`harness.rs:126`). Inspect the attached error.
- `"avatar harness pipeline task exiting"` — graceful shutdown
  observed (`harness.rs:129`).
- `"livelink udp sink started"` /
  `"livelink udp sink stopped"` — provider lifecycle
  (`avatar-provider-livelink/src/sink.rs:93-98, 143`).
- `"livelink udp send failed"` — wire-level send error
  (`avatar-provider-livelink/src/sink.rs:137`). Check firewall / port.

## 7. Tuning & operational knobs

| Field | Default | Where | Notes |
| --- | --- | --- | --- |
| `SyncConfig::frame_rate` | `60` Hz | `sync_manager.rs:33-39` | Set to `30` for Audio2Face-style 30 Hz delivery; must match what the UE receiver expects. |
| `SyncConfig::apply_emotion` | `true` | `sync_manager.rs:33-39` | Disable to omit emotion overlay on each frame (lipsync-only). |
| `AvatarHarnessConfig::perception_buffer` | `32` | `harness.rs:30-39` | Bound on the perception mpsc queue. |
| `AvatarHarnessConfig::frame_buffer` | `512` | `harness.rs:30-39` | Bound on the sink frame mpsc queue. ~8 s at 60 Hz; raise for very long utterances if you see drop warnings. |
| `AvatarHarnessConfig::emotion_decay` | `0.5` | `harness.rs:30-39` | Per-turn decay factor on the running affect vector. |
| `LiveLinkConfig::addr` | `127.0.0.1:6666` | `avatar-provider-livelink/src/config.rs:32-49` | UDP target; remote UE5 → set to the remote `host:port`. |
| `LiveLinkConfig::bind` | `None` (`0.0.0.0:0`) | same | Override if a specific local interface / port is required. |
| `LiveLinkConfig::max_fps` | `60` | same | Defensive throttle. `0` disables pacing in the sink. |
| `CognitionConfig::persona_prompt` | warm-avatar default | `cognition.rs:30-44` | Replace with your system prompt. The JSON-envelope instruction is appended automatically. |
| `CognitionConfig::model` | `"claude-haiku-4-5-20251001"` | same | atomr-infer model id. |
| `CognitionConfig::sampling.temperature` | `0.7` | same | Decoding temperature. |
| `CognitionConfig::sampling.max_tokens` | `160` | same | Per-reply token cap; keep small for low latency on screen. |
| `CognitionConfig::estimated_tokens` | `256` | same | Hint for atomr-infer's rate limiter. |

## 8. Extending the system

The harness is built on four trait points: `AvatarSink`,
`AvatarInferenceClient`, `TextToSpeech` (from `tts-core`), and
`SpeechToText` (from `stt-core` if you wire one). Each is a swap-in.

### 8.1 Custom `AvatarSink`

`AvatarSink` is defined at `crates/avatar-core/src/sink.rs:86-97`:

```rust
#[async_trait]
pub trait AvatarSink: Send + Sync + 'static {
    fn kind(&self) -> SinkKind;
    fn capabilities(&self) -> SinkCapabilities;
    async fn start(&self, frame_rx: mpsc::Receiver<AvatarFrame>) -> Result<SinkHandle>;
}
```

Lifecycle:

1. `start` spawns a long-running task that drains `frame_rx`.
2. The task **must** check `SinkHandle::stop` (`AtomicBool`) on every
   loop iteration so cooperative shutdown works.
3. Return a `SinkHandle::new(stop, join)` whose `join` completes
   when the task exits (channel closed OR stop flag set).

Worked example — a recording sink that buffers `AvatarFrame`s to a
`Vec` for offline replay:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use async_trait::async_trait;
use atomr_agents_avatar_core::{
    AvatarFrame, AvatarSink, Result, SinkCapabilities, SinkHandle, SinkKind,
};
use tokio::sync::{mpsc, Mutex};

pub struct VecSink {
    pub recorded: Arc<Mutex<Vec<AvatarFrame>>>,
}

#[async_trait]
impl AvatarSink for VecSink {
    fn kind(&self) -> SinkKind { SinkKind::MockCapture }
    fn capabilities(&self) -> SinkCapabilities { SinkCapabilities::default() }

    async fn start(&self, mut frame_rx: mpsc::Receiver<AvatarFrame>) -> Result<SinkHandle> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = stop.clone();
        let buf = self.recorded.clone();
        let join = tokio::spawn(async move {
            while !stop_t.load(Ordering::Relaxed) {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    frame_rx.recv(),
                ).await {
                    Ok(Some(f)) => buf.lock().await.push(f),
                    Ok(None) => break,
                    Err(_) => continue,
                }
            }
        });
        Ok(SinkHandle::new(stop, join))
    }
}
```

A WebRTC / browser preview sink, an MP4 writer, a Discord voice
bridge — all the same shape: drain `frame_rx`, respect the stop flag,
encode `AvatarFrame` however the downstream wants it.

### 8.2 Viseme → ARKit remap

The default 15-viseme table (`Sil / Ae / Aa / Ao / Eh / Er / Ih / W /
Oh / S / Sh / Th / F / D / Kk`) is hand-tuned at
`crates/avatar-core/src/viseme.rs:120-210` against the Azure / Oculus
viseme IDs. To use a different viseme set (Disney/Preston-Blair,
JALI, custom-per-rig), don't fork `viseme_to_arkit` — write a sibling
mapper for your viseme type that returns a `BlendshapeWeights`
overlay, and feed `SyncBundle { audio, visemes: your_visemes }` into
the sync manager. The sync manager only cares that each `VisemeFrame`
has `start_secs`, `end_secs`, `weight`, and a `Viseme` whose mapping
is known.

For projects building custom MetaHuman faces (Mesh-to-MetaHuman with
different topology), you can also tune individual viseme overlays:
`Viseme::Aa` defaults to `JawOpen=0.75 + MouthLowerDown{Left,Right}=0.45`
— if your face needs a smaller jaw drop, scale these down in your
own helper.

### 8.3 Custom TTS

Implement `atomr_agents_tts_core::TextToSpeech` and pass an
`Arc<dyn TextToSpeech>` (= `DynTextToSpeech`) via
`AvatarHarnessBuilder::with_tts(tts, voice)`. The synthesis actor
(`crates/avatar-harness/src/synthesis.rs:43-58`) currently falls back
to a **synthetic jaw track** (alternating `Aa`/`Sil` at 10 Hz) when
the backend doesn't surface viseme alignment — your backend can
either return that alignment via a sidechannel today (then plug it
into `SynthesisActor` by re-implementing `speak`) or wait for
FR-TTS-001 (atomr-infer TTS consolidation), which adds
character-level `AlignmentDelta` as a first-class
`AudioBatch`/`RealtimeBatch` output.

### 8.4 Custom STT front-end

`PerceptionActor` (`crates/avatar-harness/src/perception.rs`) is just a
thin `mpsc::Sender<Utterance>`. Bring your own STT — any
`atomr_agents_stt_core::SpeechToText` works — and push
`Utterance { text, speaker }` onto the channel as utterances commit.
The harness doesn't care about the STT backend, only about strings
arriving on the channel.

### 8.5 Cognition JSON envelope

The default envelope is at `crates/avatar-harness/src/cognition.rs:59-79`:

```jsonc
{
  "response_text": "…",
  "emotion_delta": {
    "valence":  -1..1,
    "arousal":   0..1,
    "anger":     0..1,
    "surprise":  0..1,
    "tension":   0..1
  },
  "gesture": "nod" | "shake" | "shrug" | "wave" | "point" | "idle" | null
}
```

`parse_intent` (`cognition.rs:146-153`) strips ```json fences and
falls back to plain text when the model emits prose. To extend the
envelope (e.g. add `target_subject` for multi-character scenes),
subclass `AgentIntentPacket` by defining your own struct and your own
`AvatarInferenceClient` impl that returns it serialized — then plug
that client into the builder. The pipeline core will keep working as
long as `response_text` and `emotion_delta` are present.

## 9. Roadmap & known gaps

- **Audio2Face provider is a stub.** Returns
  `Audio2FaceError::Blocked` at
  `crates/avatar-provider-audio2face/src/lib.rs:79-84`. Tracked by
  **FR-A2F-001** in
  `docs/upstream-feature-requests/atomr-infer-audio2face.md`. Once
  the upstream `RuntimeKind::Audio2Face` modality lands, swap-in is
  a one-line change.
- **TTS consolidation** under `ModelRunner` tracked by **FR-TTS-001**
  in `docs/upstream-feature-requests/atomr-infer-tts-consolidation.md`.
  Avatar pipeline gets character-level alignment (and a real
  phonemizer-based viseme track instead of the synthetic jaw fallback)
  once that lands.
- **No xtask / CLI subcommand** wraps the avatar harness today —
  it's library-use only. Sibling harnesses (`coding-cli-harness`,
  `stt-harness`) have CLIs; an `atomr-avatar` xtask is a clean
  next step.
- **UE5 receiver plugin is not in this repo** — operators author it
  using the skeleton in §5. Vendoring a reference plugin under
  `crates/avatar-ue5-receiver/` (or a sibling repo) is on the
  roadmap.
- **MetaHuman Creator web app retirement: 2026-11-05** — migrate
  pre-5.6 web-Creator characters before then per §4.7.
- **Streaming TTS path** — `SynthesisActor::speak` is currently a
  batch call. Streaming TTS + incremental viseme emission would
  drop per-turn latency from `audio_duration` to first-frame; this
  is gated on FR-TTS-001's `RealtimeBatch`.

## 10. References

Internal:

- [`docs/agentic-framework-architecture.md`](agentic-framework-architecture.md) — harness / actor pattern context.
- [`docs/upstream-feature-requests/atomr-infer-audio2face.md`](upstream-feature-requests/atomr-infer-audio2face.md) — FR-A2F-001.
- [`docs/upstream-feature-requests/atomr-infer-tts-consolidation.md`](upstream-feature-requests/atomr-infer-tts-consolidation.md) — FR-TTS-001.
- [`docs/stt-harness.md`](stt-harness.md) — the upstream STT pipeline that feeds perception.

External (current as of 2026-05-16):

- MetaHuman Creator web app discontinuation notice: <https://forums.unrealengine.com/t/metahuman-creator-web-application-is-being-discontinued/2695297>
- MetaHuman 5.6 workflow changes (in-engine Creator): <https://dev.epicgames.com/documentation/en-us/metahuman/metahuman-5-6-workflow-changes>
- UE 5.6 release notes: <https://www.unrealengine.com/news/unreal-engine-5-6-is-now-available>
- MetaHuman 5.7 release (Linux Creator + batch API): <https://www.metahuman.com/releases/metahuman-5-7-is-now-available>
- Apple ARKit blendshape locations: <https://developer.apple.com/documentation/arkit/arfaceanchor/blendshapelocation>
- Live Link plugin docs: <https://dev.epicgames.com/documentation/en-us/unreal-engine/live-link-in-unreal-engine>
