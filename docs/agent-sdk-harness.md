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
[sandbox harness](sandbox-architecture.md).**

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

## Status & roadmap

- **Now:** contract + Rust orchestration + web companion + Python bridge +
  host loader. Driven via the official Python SDK.
- **Next:** a pure-Rust backend that spawns the `claude` CLI directly
  (bypassing Python), and surfacing the SDK `plugins` option.

## Related

- [coding-cli-harness](coding-cli-harness.md) — drives the Claude Code **CLI**
  as a terminal vendor (the non-programmatic sibling).
- [sandbox-architecture](sandbox-architecture.md) — run untrusted agent work
  in an isolated microVM.
- The Anthropic Agent SDK docs and the `claude-api` skill for SDK specifics.
