# Claude Agent SDK harness

The **Claude Agent SDK harness** wraps Anthropic's
[`claude-agent-sdk`](https://pypi.org/project/claude-agent-sdk/) — the
programmable form of Claude Code — as a first-class atomr-agents harness. It
brings Claude Code's full agent harness (slash commands, subagents, hooks,
MCP, permission modes, sessions, custom system prompts, and the built-in
`Read`/`Write`/`Edit`/`Bash`/`Glob`/`Grep`/`WebSearch` tools) into the atomr
framework, and bills against **your Anthropic API credits** by driving the
bundled `claude` CLI under the hood.

> **Not Managed Agents.** This is the client-side Agent SDK, distinct from the
> server-side "Managed Agents" API (`client.beta.agents`). The SDK shells out
> to the `claude` CLI over a line-delimited-JSON stdio protocol; it does not
> call `/v1/messages` directly.

## Where it sits

atomr already has a *coding-cli* harness whose `claude` vendor drives the
Claude Code **CLI** as a fire-and-forget terminal process. The Agent SDK
harness is the *programmatic* sibling: a structured agent with typed options
(hooks, subagents, MCP, permission modes), a typed bidirectional message
protocol, and in-process custom tools. The two share the `.claude/`
projection idea but are deliberately separate harnesses.

```
            atomr config (YAML) + registry (ArtifactKind::Harness)
                              │
        AgentSdkHarnessSpec ──┤  (mirrors ClaudeAgentOptions; round-trips to JSON)
                              ▼
   AgentSdkHarness  ── impl Callable ──► composes into Pipeline / workflow / team
        │  EventBus + broadcast<AgentSdkEvent> + SessionRegistry + SpendLedger
        │
        ├── run() (headless)         ──► AgentSdkBackend::query()
        ├── start_session()/Actor    ──► AgentSdkBackend::create_session()
        ▼
  AgentSdkBackend (trait)  ── MockBackend (tests)   PythonAgentSdkBackend (py-bindings)
                                                         │ pump_async_iter (Rust drives
                                                         │ the Python async iterator)
                                                         ▼
                                       python/atomr_agents/agent_sdk.py
                                       ClaudeAgentSDKBackend → claude_agent_sdk
                                       query() / ClaudeSDKClient → `claude` CLI → API credits
```

## Crate layout

| Crate | Role |
|---|---|
| `atomr-agents-agent-sdk-core` | Contract: `AgentSdkConfig` (mirrors `ClaudeAgentOptions`), normalized `AgentSdkMessage` / `ResultSummary` / `AgentSdkEvent`, the `AgentSdkBackend` / `AgentSdkSession` trait seam, and `MockBackend`. |
| `atomr-agents-agent-sdk-harness` | Orchestrator: `AgentSdkHarness` (`impl Callable`), session registry, event/budget projection, `.claude/` materialization, and (feature `actor`) `AgentSdkActor`. |
| `atomr-agents-agent-sdk-harness-web` | Axum REST + SSE companion. |
| `py-bindings` (`agent_sdk` module) | `PythonAgentSdkBackend` + the reverse async-iterator bridge; `AgentSdkHarness` / `AgentSdkSession` pyclasses; `invoke_tool`. |
| `python/atomr_agents/agent_sdk.py` | `ClaudeAgentSDKBackend` wrapper, message normalization, options construction, tool composition, the `harness(...)` builder. |
| `host` (`agent_sdk` module) | On-disk loader: `<root>/agent-sdk/<id>/` → spec + `.claude` projection. |

## The contract

The pluggable seam is `AgentSdkBackend`:

```rust
pub type MessageStream =
    Pin<Box<dyn Stream<Item = Result<AgentSdkMessage, AgentSdkError>> + Send>>;

#[async_trait] pub trait AgentSdkBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn available(&self) -> bool;
    async fn query(&self, req: QueryRequest) -> Result<MessageStream, AgentSdkError>;
    async fn create_session(&self, spec: SessionSpec)
        -> Result<Box<dyn AgentSdkSession>, AgentSdkError>;
}
```

`MockBackend` (in `-core`) emits a scripted `system → assistant → result`
turn so the whole stack — harness, web, PyO3 — is testable without the SDK,
the `claude` CLI, or credits. `PythonAgentSdkBackend` (in `py-bindings`)
drives the real SDK. The door is open for a future pure-Rust backend that
spawns `claude` directly.

`AgentSdkHarness` implements `Callable`, so it composes as a `Pipeline` step,
a workflow node, or a team routing target. `run()` returns the final
`ResultSummary`; `start_session()` returns a stateful `InteractiveAgentSession`.

## Concept mapping (REUSE > EXTEND > INTRODUCE)

| SDK concept | atomr bridge |
|---|---|
| Skills | atomr `Skill` → `.claude/skills/<id>/SKILL.md` via the projection (`setting_sources = ["project"]`). |
| Slash commands | `<root>/agent-sdk/<id>/commands/*.md` → `.claude/commands/`; invoked by sending `/<name>` as the prompt (the SDK resolves it). |
| Subagents | atomr `SubagentDef` → SDK `AgentDefinition` in the `agents` option. `HandoffTool` still routes *between* harnesses at the workflow layer. |
| Hooks | Python hook callbacks (optionally driven by atomr host hooks) passed through `ClaudeAgentOptions.hooks`. |
| MCP | External servers from `mcp/*.yaml` → `mcp_servers`; atomr Python `@tool` guests + Rust tools → in-process SDK MCP via `create_sdk_mcp_server` (`mcp__atomr__<tool>`). |
| Permission modes | Spec `permission_mode` (default `bypassPermissions`) + `permission_policy` → a `can_use_tool` callback. |
| Sessions | Pass-through `resume` / `continue_conversation` / `fork_session`; the SDK owns the `.jsonl`. |
| System-prompt presets | `{type:"preset",preset:"claude_code",append:...}` via `SystemPromptConfig`. |

## Configuration

On-disk layout (loaded by `atomr_agents_host::load_agent_sdk`):

```text
<root>/agent-sdk/<id>/
  harness.yaml          # AgentSdkHarnessSpec (optional; defaults if absent)
  commands/<name>.md    # slash commands (frontmatter: description/model/allowed-tools)
  skills/<id>/SKILL.md  # skills (frontmatter: name/description/allowed-tools)
  mcp/<name>.yaml       # external MCP servers (McpServerConfig)
```

`harness.yaml`:

```yaml
id: code-reviewer
default_model: claude-opus-4-8       # applied when a request omits one
fallback_model: claude-sonnet-4-6
default_permission_mode: bypassPermissions   # DEFAULT — override per run/spec
default_setting_sources: [project]           # only harness-materialized .claude/
max_concurrent_sessions: 16
default_max_turns: 32
default_max_cost_usd: 1.0
auth: { provider: anthropic }                # bedrock | vertex set CLAUDE_CODE_USE_* env
```

The projection materializes `.claude/` into the run's `cwd` before each run,
so the agent sees deterministic, atomr-controlled config.

## Auth & credits

> **This harness bills your Anthropic API credits.** Every run drives the
> `claude` CLI, which calls the Anthropic API.

- `auth.provider: anthropic` → requires `ANTHROPIC_API_KEY` (resolved from the
  host `providers:` config's `api_key_env`).
- `bedrock` → `CLAUDE_CODE_USE_BEDROCK=1` + AWS credentials.
- `vertex` → `CLAUDE_CODE_USE_VERTEX=1` + GCP credentials.
- **claude.ai OAuth / subscription tokens are prohibited** by the Agent SDK —
  if `CLAUDE_CODE_OAUTH_TOKEN` is present it is ignored, not forwarded.

`ResultSummary.cost_usd` is the SDK's **client-side estimate**, not an
invoice. Cost/usage is charged to the shared `SpendLedger`; `max_cost_usd`
caps spend at turn boundaries (`max_turns` is the hard structural cap).

## Permission modes & safety

The default `permission_mode` is **`bypassPermissions`**: the agent
auto-approves every tool, including `Bash`, `Write`, and network access. This
is the configured default for autonomy and is overridable per request/spec
(`default` / `acceptEdits` / `plan`). The harness validates `cwd` / `add_dirs`
as real directories and defaults `setting_sources` to `["project"]` so the
agent only sees atomr-materialized config. **Run untrusted work inside the
[sandbox harness](sandbox-architecture.md)** — which Pattern C wires in
directly (below).

## Pattern C: per-session sandbox workspaces

`bypassPermissions` is unsafe for untrusted prompts on the bare host. **Pattern
C** contains it: each interactive session — and each headless run — gets its
own isolated [microVM sandbox](sandbox-architecture.md) as a disposable
workspace. It is **feature-gated** (`agent-sdk-harness/sandbox`) and off unless
you enable `spec.workspace`.

**The key constraint.** A `SandboxHandle` exposes **no host-visible mount**
(file I/O is async `write_file`/`read_file` over tar/vsock; `/workspace` is
container-local), so the host-resident `claude` CLI cannot `cd` into a session
sandbox. Pattern C therefore does two things together:

1. **Stages** the `.claude/` projection *into* the sandbox (via `write_file`)
   instead of onto the host filesystem.
2. **Routes** the agent's work into the sandbox: it injects an in-process
   `run_in_sandbox` tool bound to the session's sandbox and **disables the host
   `Bash`/`Write`/`Edit`** (appended to `disallowed_tools`,
   `mcp__atomr__run_in_sandbox` added to `allowed_tools`). That tool is the
   agent's only file/shell surface, so all model-authored work stays contained.

On session close the workspace is **discarded** (destroyed) by default, or
**snapshotted** to the warm pool (`on_close: snapshot`) for faster future
starts. Warm starts fork from the [`SnapshotPool`](sandbox-architecture.md) when
one is available, else cold-create. Two independent quotas apply: the harness
session cap *and* the `SandboxHarness` concurrency cap.

### The shared-registry seam

No handle is serialized across the PyO3 boundary. You pass **one**
`SandboxClient` to the harness; the Rust side holds the same
`Arc<SandboxHarness>` it wraps, and the Python `run_in_sandbox` tool closes over
the same client — so the per-session sandbox the harness creates is the exact
one the tool execs into. The session's `SandboxId` rides the normal config
round-trip (`config.sandbox_workspace_id`), which the wrapper reads to bind the
tool.

### Spec

```yaml
# harness.yaml
default_permission_mode: bypassPermissions
workspace:
  enabled: true
  profile: python_and_npm     # python_only | npm_only | rust_only | python_and_npm | full_stack
  backend: { kind: auto }     # auto | mock | docker | firecracker | { kind: remote, endpoint: … }
  on_close: discard           # discard (default) | snapshot
  reuse_warm: true            # prefer a warm fork from the snapshot pool
```

### Python

```python
import atomr_agents.agent_sdk as asdk
from atomr_agents.sandbox import SandboxClient

# One client, shared by the harness and the run_in_sandbox tool.
sbx = SandboxClient.local_default()          # or .create over docker / firecracker

h = asdk.harness(
    spec={"workspace": {"enabled": True, "profile": "full_stack", "on_close": "discard"}},
    sandbox=sbx,
)

# The agent's Bash/Write/Edit are disabled; it works only inside its sandbox.
sess = await h.session({})
await sess.query("Write a script that computes the 5000th prime and run it.")
async for ev in sess.events():
    if ev["kind"] == "run_finished":
        break
await sess.close()                            # workspace discarded here
```

The real isolation strength tracks the sandbox backend tier — `mock` (tests),
`docker` ("insecure dev mode", shared kernel), `firecracker` (the true
boundary). Code containment is not network containment: the SDK still calls
Anthropic, so set the sandbox networking policy accordingly. Running the whole
`claude` CLI *inside* the VM (a `SandboxAgentSdkBackend`) and a host-side
credential proxy are future work — see *Status & roadmap*.

## Python parity

```python
import atomr_agents.agent_sdk as asdk

# Expose an atomr tool to the agent as mcp__atomr__add.
async def add(args):
    return {"content": [{"type": "text", "text": str(args["a"] + args["b"])}]}

h = asdk.harness(tools=[add], spec={"default_permission_mode": "bypassPermissions"})

# Headless one-shot.
result = await h.run({
    "prompt": "Use the add tool on 2 and 3, then summarize.",
    "cwd": "/repo",
    "allowed_tools": ["mcp__atomr__add"],
})
print(result["result"], result["cost_usd"])

# Interactive bidirectional session — the actor surface.
sess = await h.session({"cwd": "/repo"})
await sess.query("/review the auth module")
async for ev in sess.events():
    if ev["kind"] == "assistant_text_delta":
        print(ev["text"], end="")
    elif ev["kind"] == "run_finished":
        break
await sess.close()
```

`AgentSdkHarness.mock()` builds the same surface over `MockBackend` for tests
that must not touch the SDK or credits.

## Generalizing to other providers

This harness is named `agent-sdk`, not `claude-*`, on purpose. The Anthropic
Agent SDK established a shape — *a programmable coding agent you configure with
options, drive with prompts, and consume as a stream of typed messages
(system → assistant → result), with built-in tools, subagents, hooks, MCP, and
sessions*. As other vendors ship Agent-SDK-style products, they tend to
**mirror that structure with small differences** (renamed options, slightly
different message classes, different auth env vars, a different tool-naming
convention). The harness is built so those differences are absorbed by a thin
adapter — **the Rust orchestration, events, budget, web companion, and actor
layers never change.**

### What's provider-neutral vs. provider-specific

Everything above the `AgentSdkBackend` trait only ever sees the **normalized
schema** — `AgentSdkConfig`, `AgentSdkMessage`, `ResultSummary`,
`AgentSdkEvent`. It has no Anthropic-specific knowledge. All provider specifics
are concentrated in exactly two places:

| Layer | Provider-neutral? | Where a new provider's deltas live |
|---|---|---|
| `AgentSdkHarness`, session registry, budget ledger, `.claude/` projection, events, web companion, `AgentSdkActor` | ✅ Neutral | — |
| `AgentSdkBackend` / `AgentSdkSession` traits + `MessageStream` | ✅ Neutral | A new impl, but the *shape* is fixed |
| `AgentSdkConfig` (the JSON dict) | Mostly neutral | Unknown fields are simply not emitted; provider-only knobs ride in `env` / opaque values |
| **`_build_options(config)`** (Python) | ❌ Provider-specific | Map the neutral config → the provider's options object |
| **`_normalize(msg)`** (Python) | ❌ Provider-specific | Map the provider's message classes → the normalized dict |

### Two ways to add a provider

**(A) A new Python wrapper** — for any provider whose SDK is *shaped like*
`claude-agent-sdk` (an async-iterating agent with an options object and
typed messages). Subclass `ClaudeAgentSDKBackend` and override only the two
adapter methods. Everything else — the PyO3 bridge, the reverse
async-iterator pump, the harness — is reused unchanged:

```python
import acme_agent_sdk as acme  # a hypothetical Anthropic-shaped SDK
from atomr_agents.agent_sdk import ClaudeAgentSDKBackend, _filter_kwargs

class AcmeAgentSDKBackend(ClaudeAgentSDKBackend):
    # 1. Map the neutral config dict → Acme's options object.
    def _build_options(self, config):
        kw = {k: config[k] for k in (
            "system_prompt", "allowed_tools", "model", "max_turns", "cwd",
        ) if config.get(k) is not None}
        # Small deltas: Acme calls it `tool_allowlist`, not `allowed_tools`.
        if "allowed_tools" in kw:
            kw["tool_allowlist"] = kw.pop("allowed_tools")
        # `_filter_kwargs` already drops anything Acme's options don't accept,
        # so version skew never 400s at construction.
        return acme.AgentOptions(**_filter_kwargs(acme.AgentOptions, kw))

    # 2. Map Acme's message objects → the normalized dict schema.
    def _normalize(self, msg):
        name = type(msg).__name__
        if name == "AcmeInit":
            return {"type": "system", "session_id": msg.conversation_id}
        if name == "AcmeText":
            return {"type": "assistant", "blocks": [{"kind": "text", "text": msg.text}]}
        if name == "AcmeDone":
            return {"type": "result", "subtype": "success", "result": msg.final,
                    "session_id": msg.conversation_id, "cost_usd": msg.cost}
        return {"type": "unknown", "repr": repr(msg)}

    # 3. The async-generator entry points keep the same names the Rust
    #    backend calls: `run(config, prompt)`, `open_session(config)`,
    #    `session_send/stream/interrupt/...`. Override `run`/`session_stream`
    #    only if Acme's iteration API differs from `query()` / a client.

# Register it under its own key and point a harness at it — same plumbing as
# the built-in `harness(...)` builder, just a different backend instance.
from atomr_agents import _native

_native.guest.register_agent_sdk_factory("acme", AcmeAgentSDKBackend())
harness = _native.agent_sdk.AgentSdkHarness.from_python_backend("acme")
```

The two helpers that make this robust already exist: `_normalize` dispatches on
`type(msg).__name__` with `getattr` defaults (so a renamed field degrades
instead of crashing), and `_filter_kwargs` strips any option the target SDK
doesn't accept (so an extra knob is ignored rather than fatal).

**(B) A native Rust backend** — implement `AgentSdkBackend` /
`AgentSdkSession` directly in Rust (e.g. spawn the provider's CLI and parse its
stdio protocol into `AgentSdkMessage`). This is the same door left open for a
pure-Rust `claude` driver. No Python required; the harness consumes it
identically.

### When an adapter is *not* enough

The contract assumes the Agent-SDK shape: a configurable agent that streams
`system → assistant(blocks) → result` and accepts prompts. A provider that
diverges structurally — no streaming, no session concept, a fundamentally
different tool/permission model — needs more than the two adapter methods.
Extend the neutral schema (add variants to `AgentSdkMessage` /
`AgentSdkConfig`) rather than bending an ill-fitting provider through it; the
`#[serde(other)] Unknown` fallback keeps such additions backward-compatible.

### Auth & tooling deltas

- **Credentials** are env-var driven (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_USE_*`);
  a new provider sets its own vars at spawn via the spec's `auth` block / the
  config `env` map — no code change in the neutral layers.
- **In-process tools** are bridged through `create_sdk_mcp_server` /
  `invoke_tool`; a provider with a different custom-tool API overrides the tool
  composition helper (`tools_to_sdk_server`) in its wrapper subclass.

## Status & roadmap

- **Now:** contract + Rust orchestration + web companion + Python bridge +
  host loader. Driven via the official Python SDK. **Pattern C** (per-session
  sandbox workspaces) ships behind the `sandbox` feature — tool-routed
  containment with discard/snapshot lifecycle.
- **Next:** a pure-Rust backend that spawns the `claude` CLI directly
  (bypassing Python); surfacing the SDK `plugins` option; **Pattern B** — a
  `SandboxAgentSdkBackend` that runs the whole `claude` CLI *inside* the VM
  (full process containment) with a host-side credential proxy so the API key
  never enters the guest.

## Related

- [coding-cli-harness](coding-cli-harness.md) — drives the Claude Code **CLI**
  as a terminal vendor (the non-programmatic sibling).
- [sandbox-architecture](sandbox-architecture.md) — run untrusted agent work
  in an isolated microVM.
- The Anthropic Agent SDK docs and the `claude-api` skill for SDK specifics.
