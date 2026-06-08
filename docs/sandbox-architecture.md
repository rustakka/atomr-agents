# MicroVM sandbox

Secure, instant-boot compute environments that let an agent execute
**untrusted code** — Python, Bash, JavaScript, Rust — and feed the result
back into a turn. The sandbox is the substrate behind the
`execute_in_sandbox` tool, the PyO3 `SandboxClient`, and agent *branch
states* (fork-from-snapshot).

The design separates a **backend-agnostic contract** (one trait the
hypervisor/container layer implements, one trait a live sandbox exposes)
from the **orchestration** that schedules, pools, quotas, and observes
sandboxes — and from the **backends** that actually run code. That seam is
the whole point: the tool, the harness, the web companion, and the Python
bindings are written once against the contract and run unchanged on a
deterministic mock (CI), a Docker container (dev laptop), a Firecracker
microVM (the real isolation boundary), or a remote gRPC cluster.

This mirrors the `coding-cli-isolator` split (`Isolator` / `ProcessHandle`)
one layer up the stack — see [Coding CLI harness](coding-cli-harness.md) —
and slots into the framework like any other [tool](agent-pipeline.md) and
[harness](architecture.md).

## Where it sits

```
   agent turn ── execute_in_sandbox (Tool) ─┐
   workflow step ── SandboxHarness (Callable)├─▶ SandboxHarness
   Python ── SandboxClient ──────────────────┘        │
   HTTP ── sandbox-harness-web (REST + SSE) ──────────┤
                                                      │  scheduler · snapshot pool
                                                      │  registry · quota · events
                                                      ▼
                                          SandboxBackend  (trait)
                              ┌───────────────┬───────────────┬───────────────┐
                              ▼               ▼               ▼               ▼
                           Mock           Docker         Firecracker        Remote
                        (in-mem,      ("insecure dev    (local microVM,    (Tier-3 gRPC
                         CI/tests)      mode")            KVM boundary)      cluster)
                              ▲               ▲               ▲               │
                              └───────────────┴───────────────┴── SandboxHandle (trait)
                                                              │
                                       host ↔ guest wire protocol (vsock + postcard frames)
                                                              ▼
                                          sandbox-guest-agent (PID 1, in-VM)
```

## Crate layout

```
crates/sandbox-core/            # contract: traits, domain types, events, MockBackend
crates/sandbox-harness/         # orchestration: scheduler, snapshot pool, registry, quota, Callable
crates/sandbox-tool/            # the execute_in_sandbox Tool
crates/sandbox-backend-docker/  # Docker "insecure dev mode" backend (bollard)
crates/sandbox-proto/           # host ↔ guest wire protocol (postcard frames over vsock)
crates/sandbox-guest-agent/     # in-VM PID-1 daemon that serves the protocol
crates/sandbox-harness-web/     # Axum REST + SSE companion
```

Python parity lives in `crates/py-bindings/src/sandbox.rs` → the
`atomr_agents.sandbox` facade. The umbrella exposes the surface behind the
`sandbox` feature:

```rust
// crates/umbrella/Cargo.toml
sandbox = ["dep:atomr-agents-sandbox-core",
           "dep:atomr-agents-sandbox-harness",
           "dep:atomr-agents-sandbox-tool", "tool"]
```

```rust
use atomr_agents::sandbox;            // glob re-export of sandbox-core
use atomr_agents::sandbox::harness;   // SandboxHarness
use atomr_agents::sandbox::tool;      // execute_in_sandbox
```

**Built today:** `sandbox-core` (incl. `MockBackend`), `sandbox-harness`,
`sandbox-tool`, `sandbox-backend-docker`, `sandbox-proto`,
`sandbox-guest-agent`, `sandbox-harness-web`. The **Firecracker** and
**remote-cluster** backends are *designed into the surface* (the
`SandboxBackendSel` variants and Python `SandboxConfig` constructors exist
and are forward-compatible) but their backend crates are not yet
implemented. See [Status & roadmap](#status--roadmap).

## The contract (`sandbox-core`)

Two traits, deliberately small. Everything else in the subsystem is built on
them, and every backend implements them.

### `SandboxBackend` — provisions sandboxes

Mirrors the `Isolator` trait.

```rust
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Stable identifier used in logs and `SandboxInfo::backend`.
    fn name(&self) -> &str;
    /// Whether this backend is usable on the current host
    /// (KVM present, Docker reachable, endpoint configured…). `Auto` uses this.
    async fn available(&self) -> bool;
    /// Cold-boot a fresh sandbox, or warm-fork when `req.from_snapshot` is set.
    async fn create(&self, req: CreateSandbox) -> Result<Box<dyn SandboxHandle>>;
}
```

**Provisioning contract:** implementations MUST provision using
`req.effective_budget()` and MUST NOT read `req.budget` directly —
`effective_budget` re-applies the [Rust floor](#the-rust-floor), which a raw
`req.budget` can undercut. The harness also normalizes `req.budget` to the
effective value before dispatch, so the floor holds even if a backend
forgets.

### `SandboxHandle` — one live sandbox

Mirrors `ProcessHandle`, extended with `snapshot` / `fork`. Every method
takes `&self` (not `Box<Self>`) so handles stay `Arc`-shareable for the
harness registry and the PyO3 wrappers.

```rust
#[async_trait]
pub trait SandboxHandle: Send + Sync {
    fn info(&self) -> &SandboxInfo;

    /// Run code to completion, returning captured output.
    async fn exec(&self, req: ExecRequest) -> Result<ExecResult>;
    /// Streaming variant: stdout/stderr arrive as byte chunks while the exec
    /// runs, with the final ExecResult on a oneshot. The default impl buffers
    /// `exec` then emits once — backends that can truly stream (Firecracker
    /// via vsock) override this.
    async fn exec_streaming(&self, req: ExecRequest) -> Result<ExecStream>;

    /// File ops are confined to the sandbox root — absolute paths and `..`
    /// traversal are rejected so untrusted code cannot escape.
    async fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()>;
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;

    /// Snapshot live memory + disk. Unsupported backends return Unsupported.
    async fn snapshot(&self) -> Result<SnapshotId>;
    /// Fork a NEW sandbox from this one's state (the <50 ms warm-fork path on
    /// Firecracker). Backs agent branch states.
    async fn fork(&self) -> Result<Box<dyn SandboxHandle>>;
    /// Idempotent teardown.
    async fn destroy(&self) -> Result<()>;
}

pub struct ExecStream {
    pub stdout_rx: mpsc::Receiver<Vec<u8>>,
    pub stderr_rx: mpsc::Receiver<Vec<u8>>,
    pub result:    oneshot::Receiver<ExecResult>,
}
```

### Domain types

| Type | Shape |
|---|---|
| `Language` | `Python` · `Bash` · `Js` · `Rust` (serialized snake_case) |
| `SandboxProfile` | `PythonOnly` · `NpmOnly` · `RustOnly` · `PythonAndNpm` · `FullStack` |
| `ResourceBudget` | `vcpus: u8` · `mem_mib: u32` · `disk_mib: u32` · `wall_clock_secs: u64` |
| `CreateSandbox` | `profile` · `budget: Option` · `backend: SandboxBackendSel` · `env` · `from_snapshot: Option` · `metadata` |
| `SandboxInfo` | `id` · `profile` · `budget` · `backend: String` · `boot_ms: u64` · `forked_from: Option` · `created_at` |
| `ExecRequest` | `language` · `code` · `dependencies: Vec<String>` · `stdin: Option` · `env` · `timeout_secs: Option` |
| `ExecResult` | `exec_id` · `exit: ExitStatus` · `stdout` · `stderr` · `started_at` · `ended_at` · `timed_out` |
| `ExitStatus` | `code: Option<i32>` · `success: bool` |

`ExecResult::to_tool_json()` flattens to the **canonical consumer shape** —
the same object the tool, the harness `Callable`, the web API, and the
Python bindings all return:

```json
{ "exec_id": "…", "exit_code": 0, "success": true,
  "stdout": "…", "stderr": "", "timed_out": false }
```

### Toolchain profiles

A `SandboxProfile` names the toolchain image a sandbox boots with. Five ship:

| Profile | Languages | Toolchain | Logical image tag |
|---|---|---|---|
| `PythonOnly` | Python, Bash | CPython + pip | `sandbox/python-only` |
| `NpmOnly` | Js, Bash | Node.js + npm | `sandbox/npm-only` |
| `RustOnly` | Rust, Bash | rustc + LLVM | `sandbox/rust-only` |
| `PythonAndNpm` | Python, Js, Bash | CPython + Node | `sandbox/python-and-npm` |
| `FullStack` | Python, Js, Rust, Bash | all of the above | `sandbox/full-stack` |

When the caller doesn't pin a profile, `SandboxProfile::for_language` infers
one: `Python → PythonOnly`, `Js → NpmOnly`, `Rust → RustOnly`, and
`Bash → FullStack` (Bash needs no toolchain, so it gets the safe superset).
`profile.supports(lang)` gates execution fail-fast; `image_tag()` is
resolved to a concrete image by the backend.

### The Rust floor

Rust compilation is memory- and CPU-hungry, so any profile that can build
Rust (`RustOnly`, `FullStack`) is held to a **hard minimum of 2 vCPUs and
2 GB (2048 MiB) RAM**, regardless of what the caller asked for. The floor
only ever *raises* a budget — it is idempotent and never lowers.

```rust
// ResourceBudget defaults: 1 vCPU / 512 MiB / 2048 MiB disk / 60 s wall clock.
// Rust floor constants: RUST_MIN_VCPUS = 2, RUST_MIN_MEM_MIB = 2048.

pub fn enforce_rust_floor(mut self) -> Self {
    self.vcpus  = self.vcpus.max(2);
    self.mem_mib = self.mem_mib.max(2048);
    self
}
```

It is enforced in three layers (belt-and-suspenders): `ResourceBudget::
for_profile`, `CreateSandbox::effective_budget`, and the harness's
`normalize` step before every backend dispatch.

## Backends & the isolation tiers

`available()` + the `Auto` selector resolve to the **most secure backend the
host can run**: Firecracker → Docker → Mock. The intended escalation:

| Tier | Backend | Isolation | When | Status |
|---|---|---|---|---|
| — | **Mock** | none (in-memory) | CI, unit tests, cross-platform dev | **built** |
| 1 | **Docker** | shares host kernel — *not* a security boundary | dev laptop, real execution with only Docker Desktop | **built** |
| 2 | **Firecracker** | per-sandbox microVM (KVM); the real boundary; <50 ms warm-fork | running genuinely untrusted code | designed |
| 3 | **Remote** | Firecracker on a fleet, reached over a gRPC control plane | scale-out / multi-tenant | designed |

### `SandboxBackendSel` — backend selection

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxBackendSel {
    #[default] Auto,                       // most secure available
    Mock,
    Docker { image: Option<String> },
    Firecracker,
    Remote { endpoint: String },
}
```

### `MockBackend` (built — `sandbox-core`)

A deterministic, in-memory backend that lives in `-core` so every downstream
crate is unit-testable cross-platform with **no Docker and no KVM**. It
records file writes in a `BTreeMap`, returns synthetic exec output, and
implements `snapshot`/`fork` by cloning its in-memory filesystem (a
low-fidelity stand-in for a Firecracker copy-on-write fork, lineage
recorded). `available()` is always `true`. This is what
`SandboxHarness::local_default()` wires, and it is the reason the entire
sandbox surface — tool, harness, web, PyO3 — has portable tests.

### Docker backend ("insecure dev mode", built — `sandbox-backend-docker`)

> Containers share the host kernel, so this is **not** a security boundary
> for untrusted code (that is the Firecracker backend's job). It exists so
> the whole sandbox surface runs end-to-end on a developer laptop with only
> Docker Desktop.

Model:

- **One long-lived container per sandbox** (`sleep infinity`); each `exec`
  runs through the Docker exec API inside it (`create_exec` / `start_exec` /
  `inspect_exec` for the exit code).
- **Files move as tar streams** confined to `/workspace`:
  `upload_to_container` for `write_file`, `download_from_container` for
  `read_file`. A `confine()` guard rejects absolute paths and `..`.
- **`snapshot` = `commit_container`** to an `atomr-sandbox:<uuid>` image;
  **`fork` = snapshot + boot a fresh container** from that image.
- Built on `bollard` (0.17), reusing the machinery proven in
  `coding-cli-isolator`. Integration tests skip automatically when no Docker
  daemon is reachable.

### Firecracker & remote (designed)

Firecracker is the tier that makes "execute untrusted code" true: a
single-tenant microVM per sandbox, KVM-backed, with the **vsock host↔guest
protocol** below and a sub-50 ms copy-on-write warm-fork. The remote tier
places Firecracker microVMs across a fleet behind a gRPC control plane (the
"Tier-3 cluster"), where the per-VM vsock frames are tunneled inside the
node ↔ control-plane stream. The contract, the protocol, the guest agent,
and the scheduler are already shaped for these tiers; the backend crates
that drive `firecracker`/`jailer` and the gRPC client remain to be built.

## Orchestration (`sandbox-harness`)

`SandboxHarness` owns a pluggable backend, a registry of live sandboxes, a
scheduler, a warm snapshot pool, an event bus, and a broadcast channel of
lifecycle events.

```rust
pub struct SandboxHarness {
    backend:   Arc<dyn SandboxBackend>,
    pub bus:   EventBus,                                  // Event::ToolInvoked telemetry
    event_tx:  broadcast::Sender<SandboxEvent>,           // lifecycle stream
    sandboxes: DashMap<SandboxId, Arc<dyn SandboxHandle>>,// persistent registry
    live:      AtomicUsize,                               // concurrency admission counter
    pool:      Arc<SnapshotPool>,                         // warm snapshots per profile
    scheduler: Arc<dyn Scheduler>,                        // placement
    config:    SandboxHarnessConfig,
}
```

`SandboxHarnessConfig` defaults: `event_channel_capacity = 256`,
`max_concurrent_sandboxes = 64`, `snapshot_pool_capacity = 4` (per profile).

`SandboxHarness::local_default()` = `MockBackend` + `BestFitScheduler` +
defaults — the cross-platform default used by tests and the Python bindings.

### Two execution paths

**Ephemeral one-shot** — `run_exec` / `run_once`. Create a sandbox, run one
exec, destroy it, emitting the full lifecycle. This is what
`execute_in_sandbox` drives. It resolves the profile from the language when
none is given, fails fast if the profile doesn't support the language,
applies the Rust floor, and bypasses the registry quota (it never lingers).

**Persistent registry** — `create` → `get` / `list` / `exec` / `snapshot` /
`fork` / `destroy`, keyed by `SandboxId`. This backs the PyO3 `SandboxClient`
and agent branch states (`fork`). `create` and `fork` are subject to the
concurrency quota.

### Concurrency quota (TOCTOU-safe)

A naïve `if registry.len() < max` check races across the provisioning
`await`. Instead the harness reserves a slot **atomically before** the await
on an `AtomicUsize`:

```rust
fn reserve_slot(&self) -> Result<(), SandboxError> {
    if self.live.fetch_add(1, SeqCst) >= self.config.max_concurrent_sandboxes {
        self.live.fetch_sub(1, SeqCst);
        return Err(SandboxError::BudgetViolation(/* … */));
    }
    Ok(())
}
```

A failed provision calls `release_slot`; a successful one holds the slot
until `destroy`. Over-quota callers get a `BudgetViolation` (no queueing).

### Scheduler — best-fit bin packing

```rust
pub trait Scheduler: Send + Sync {
    fn place(&self, need: &ResourceBudget,
             prefer_snapshot: Option<&SnapshotId>,
             nodes: &[NodeStatus]) -> Option<NodeId>;
}
```

`BestFitScheduler` filters to nodes that fit (`free_vcpus`, `free_mem_mib`),
then picks by a sort key of **(snapshot-locality, tightest remaining
memory, tightest remaining vCPUs)** — so a fork lands on a node that already
holds its parent snapshot, and otherwise sandboxes pack densely. On a single
host this degenerates to "is there room?"; it earns its keep in the Tier-3
fleet. `NodeStatus` carries `{ id, free_vcpus, free_mem_mib, warm_snapshots }`.

### Snapshot pool — warm forks

```rust
pub struct SnapshotPool {
    inner: Mutex<HashMap<SandboxProfile, VecDeque<SnapshotId>>>,
    capacity_per_profile: usize,
}
```

A bounded FIFO of warm snapshots **per profile**. `offer(profile, snap)`
returns `false` and drops the snapshot when the profile's queue is full;
`take(profile)` pops the oldest. Provisioning with `from_snapshot` can pull
a warm snapshot instead of cold-booting — the substrate for instant
fork-resume.

### Harness as a `Callable`

`SandboxHarness` implements `Callable` (label `"sandbox-harness"`), so it
drops into a [workflow](workflows-and-hitl.md) as a step. It accepts a
flattened exec request plus optional `profile` / `backend` overrides and
returns the canonical exec JSON:

```jsonc
// input
{ "language": "python", "code": "print(2+2)",
  "profile": "python_only", "backend": { "kind": "auto" } }
// output → ExecResult::to_tool_json()
```

## Events & observability

Two distinct streams:

- **`Event::ToolInvoked`** on the harness `EventBus` — generic framework
  telemetry (`tool_id`, `args_hash`, `elapsed_ms`, `ok`) consumed by
  [tracers](observability.md).
- **`SandboxEvent`** on a `tokio::broadcast` channel — sandbox lifecycle,
  consumed by the SSE endpoint and the Python `SandboxEventStream`. A tagged
  enum (`{"kind": "…", …}`) so web clients switch on `kind`:

  | Variant | Fields |
  |---|---|
  | `Created` | `id`, `profile`, `boot_ms` |
  | `ExecStarted` | `id`, `language` |
  | `ExecEnded` | `id`, `exec_id`, `exit_code`, `elapsed_ms` |
  | `ExecError` | `id`, `error` |
  | `Forked` | `parent`, `child`, `snapshot` |
  | `Destroyed` | `id` |

`SandboxEventStream` wraps a `broadcast::Receiver`; `recv()` drops missed
events silently on lag and ends on close — same semantics as
`CodingCliEventStream`.

## The `execute_in_sandbox` tool (`sandbox-tool`)

The agent-facing surface. `SandboxTool` wraps an `Arc<SandboxHarness>` and
implements `Tool` (id `sandbox.execute_in_sandbox`).

**Arguments** (`required: language, code`):

| Field | Type | Notes |
|---|---|---|
| `language` | `"python" \| "bash" \| "js" \| "rust"` | required |
| `code` | string (minLength 1) | required |
| `dependencies` | string[] | `pip` / `npm` installs run before the code |
| `profile` | `"python_only" \| "npm_only" \| "rust_only" \| "python_and_npm" \| "full_stack"` | inferred from `language` if omitted |
| `stdin` | string | piped to the process |
| `timeout_secs` | integer, 1–600 | wall-clock cap |

It parses and validates args, runs the code via `run_exec(..,
SandboxBackendSel::Auto)` — applying the Rust floor — emits
`Event::ToolInvoked`, and returns the canonical exec JSON. Backed by the mock
via `SandboxHarness::local_default()` it runs cross-platform; swap in Docker
or Firecracker for real execution. Mirrors `web-search-tool`.

## Host ↔ guest wire protocol (`sandbox-proto`)

How the host talks to code running *inside* a Firecracker microVM.
Communication is **length-prefixed `postcard` frames** — `u32`
little-endian length + postcard body — over `AF_VSOCK`.

```rust
pub enum GuestRequest {                       pub enum GuestResponse {
    Ping,                                         Pong,
    Exec { exec_id, language, code,               ExecStdout { exec_id, chunk },   // streamed
           dependencies, stdin, env,              ExecStderr { exec_id, chunk },   // streamed
           timeout_ms },                          ExecDone   { exec_id, exit_code, // terminal
    WriteFile { path, bytes },                                 timed_out },
    ReadFile  { path },                           FileWritten,
    Shutdown,                                     FileRead   { bytes },
}                                                 Error      { message },
                                              }
```

An `Exec` produces a stream of `ExecStdout`/`ExecStderr` frames terminated by
exactly one `ExecDone`; every other response is a single frame.

```rust
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), ProtoError>;
pub async fn read_frame<R, T>(r: &mut R) -> Result<Option<T>, ProtoError>;
```

`read_frame` reads the `u32` length first and rejects anything over the
**64 MiB cap (`MAX_FRAME_BYTES`) before allocating the body buffer**, so a
corrupt length prefix can't trigger a huge allocation; a clean EOF at a frame
boundary returns `Ok(None)`. The functions are transport-agnostic — any
`tokio::io::AsyncRead`/`AsyncWrite` — so they're exercised over in-memory
pipes in tests and over a real vsock socket in the guest.

**Why hand-rolled and not gRPC-over-vsock:** it keeps the in-VM guest agent
a tiny static binary — no tonic / h2 / tower stack inside the VM, which
matters for cold-start size and the musl static link. gRPC is used only
*host-to-host* in the Tier-3 cluster, where vsock frames are tunneled inside
the node ↔ control-plane stream.

## The guest agent (`sandbox-guest-agent`)

A small Rust daemon that runs as **PID 1** inside a Firecracker guest and
serves the protocol command set.

- **Core (`lib`)** — `serve` / `serve_rw` drive the request/response loop
  over any async reader+writer (transport-agnostic, unit-tested over
  in-memory pipes on every platform). `exec` builds per-language commands:

  | Language | Dependencies | Run |
  |---|---|---|
  | Python | `pip install --quiet <deps>` | `python -c <code>` |
  | Js | `npm install --silent <deps>` | `node -e <code>` |
  | Bash | — | `bash -c <code>` |
  | Rust | — | base64-decode code → `rustc main.rs` → run the binary |

  Interpreted languages pass code as a literal argv element (no shell, so no
  quoting hazard). File I/O is **confined to the sandbox root** (`/workspace`
  by default) — `resolve()` rejects absolute paths and any `..` component.

- **Binary** — serves the protocol over **stdin/stdout**; the host (the
  Linux Firecracker backend) bridges the guest's stdio onto the vsock
  device, so this binary carries no platform socket code and stays a lean
  static **musl** build. On Linux it also does best-effort PID-1 init —
  mounting `/proc`, `/sys`, `/tmp` (failures non-fatal; the kernel cmdline
  may already have mounted them).

```sh
cargo build -p atomr-agents-sandbox-guest-agent \
  --release --target x86_64-unknown-linux-musl
```

## Web companion (`sandbox-harness-web`)

An Axum REST + SSE server over a `SandboxHarness`. `WebServer::new(config,
harness)`; `WebConfig` defaults to `0.0.0.0:8080`. The bundled binary serves
a mock-backed harness; wire a Docker- or Firecracker-backed harness into
`WebServer::new` for real execution.

| Method | Path | Body → Response |
|---|---|---|
| POST | `/run` | `{language, code, profile?, …}` → exec JSON |
| POST | `/sandboxes` | `CreateSandbox` → `SandboxInfo` |
| GET | `/sandboxes` | → `[SandboxInfo]` |
| POST | `/sandboxes/:id/exec` | `ExecRequest` → exec JSON |
| POST | `/sandboxes/:id/fork` | → `SandboxInfo` |
| POST | `/sandboxes/:id/snapshot` | → `{ snapshot_id }` |
| DELETE | `/sandboxes/:id` | → `204` |
| GET | `/events` | → SSE stream of `SandboxEvent` (JSON per event) |
| GET | `/healthz` | → `{ status, backend, live }` |

The SSE endpoint subscribes to the harness broadcast channel and serializes
each `SandboxEvent` as a JSON SSE frame with keep-alive. It is a pure
JSON/SSE API — no embedded SPA. Mirrors `channel-harness-web`.

## Python parity

`atomr_agents.sandbox` re-exports the native `sandbox` submodule (mirrors
`coding_cli`). Backed by the mock via `SandboxClient.local_default()` it runs
cross-platform; Docker / Firecracker / remote slot in behind the same
surface.

```python
from atomr_agents.sandbox import SandboxClient, SandboxConfig, SandboxProfile

# Ephemeral one-shot
client = SandboxClient.local_default()
result = await client.run({"language": "python", "code": "print(2 + 2)"})
print(result["stdout"])

# Persistent sandbox + branching
config  = SandboxConfig.cluster(endpoint="grpc://sandbox.internal:50051")
sandbox = await client.create(SandboxProfile.FullStack, config)
await sandbox.write_file("src/main.rs", b'fn main() { println!("hi"); }')
out = await sandbox.run("cargo run --release")
child = await sandbox.fork()           # backs agent branch states

# Lifecycle event stream
stream = client.events()
while (ev := await stream.recv()) is not None:
    print(ev["kind"], ev)
```

| Class | Surface |
|---|---|
| `SandboxClient` | `local_default()`, `run()` (one-shot), `create()` (persistent), `events()` |
| `Sandbox` | `exec`, `run`, `write_file`, `read_file`, `fork`, `snapshot`, `destroy` |
| `SandboxConfig` | `local()` (Auto) · `mock()` · `docker(image?)` · `firecracker()` · `cluster(endpoint)` |
| `SandboxProfile` | the 5 toolchain profiles (enum, `.value` → snake_case) |
| `SandboxEventStream` | async `recv()` → dict, or `None` once closed |

Async methods bridge Rust futures to Python awaitables via
`pyo3-async-runtimes`. The `SandboxConfig` constructors for `firecracker()`
and `cluster()` already exist (forward-compatible); they resolve to the
not-yet-built backends today.

## Security posture

- **The Docker backend is explicitly *not* a security boundary.** Containers
  share the host kernel; it is "insecure dev mode" for running *your own*
  code conveniently, not for untrusted code.
- **Firecracker is the boundary.** A per-sandbox KVM microVM is the tier
  intended for genuinely untrusted agent-authored code; the protocol, guest
  agent, and contract are built for it.
- **Path confinement is enforced at every layer** that touches a real
  filesystem — the `SandboxHandle` contract, the Docker backend's
  `confine()`, and the guest agent's `resolve()` all reject absolute paths
  and `..` traversal.
- **The Rust floor** guarantees Rust builds get enough resources to avoid
  OOM-thrash, applied independent of caller input in three layers.
- **Wall-clock timeouts** (`timeout_secs`, the budget's `wall_clock_secs`,
  the protocol's `timeout_ms`) bound runaway execs.
- **Frame size is capped** (64 MiB) before allocation to blunt a malformed
  length prefix from the guest.
- **Concurrency is quota'd** to bound resource exhaustion from many
  concurrent sandboxes.

## Testing & determinism

The `MockBackend` makes the entire surface — core types, harness paths,
tool, web routes, PyO3 bindings — testable on any platform with no Docker and
no KVM: deterministic synthetic exec output, in-memory files, and
clone-based snapshot/fork. CI exercises the contract everywhere; the Docker
integration tests self-skip when no daemon is present. This is the same
pattern `MockWebSearch` follows in `web-search-core`.

## Status & roadmap

| Component | Status |
|---|---|
| `sandbox-core` contract + domain types + events + `MockBackend` | ✅ built |
| `sandbox-harness` (registry, quota, scheduler, snapshot pool, `Callable`) | ✅ built |
| `execute_in_sandbox` tool | ✅ built |
| Docker "insecure dev mode" backend | ✅ built |
| Host↔guest protocol (`sandbox-proto`) | ✅ built |
| In-VM guest agent (`sandbox-guest-agent`) | ✅ built |
| Web companion (REST + SSE) | ✅ built |
| Python bindings (`atomr_agents.sandbox`) | ✅ built |
| Firecracker backend (`sandbox-backend-firecracker`) | ◻ designed; surface reserved |
| Remote / Tier-3 gRPC cluster backend (`sandbox-backend-remote`) | ◻ designed; surface reserved |

The `SandboxBackendSel::{Firecracker, Remote}` variants and the Python
`SandboxConfig::{firecracker, cluster}` constructors exist so callers can
target those tiers today; selecting them resolves to a backend that is not
yet implemented.

## Related

- [Architecture](architecture.md) — the framework's crate stack and where
  tools/harnesses slot in.
- [Coding CLI harness](coding-cli-harness.md) — the `Isolator` /
  `ProcessHandle` split this subsystem mirrors one layer up.
- [Agent pipeline](agent-pipeline.md) — how the `execute_in_sandbox` tool is
  dispatched in a turn.
- [Workflows and HITL](workflows-and-hitl.md) — the harness as a workflow
  step.
- [Observability](observability.md) — `Event::ToolInvoked` and the tracer
  exporters.
- [Feature matrix](feature-matrix.md) — the `sandbox` umbrella feature.
