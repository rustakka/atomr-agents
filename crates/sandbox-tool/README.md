# atomr-agents-sandbox-tool

The `execute_in_sandbox` **Tool**: exposes the atomr-agents microVM sandbox to
agents. It wraps a `SandboxHarness` and implements `atomr_agents_tool::Tool`.

Given `{ language, code, dependencies?, profile?, stdin?, timeout_secs? }` it:

1. parses and validates the arguments,
2. resolves the toolchain profile (explicit, or inferred from `language`),
3. runs the code in a fresh ephemeral sandbox via the harness — which applies
   the **Rust-floor** budget (Rust profiles get ≥ 2 GB RAM / 2 vCPUs),
4. emits `Event::ToolInvoked` on the harness `EventBus` for tracer telemetry,
5. returns `{ exec_id, exit_code, success, stdout, stderr, timed_out }`.

Backed by the deterministic `MockBackend` via `SandboxHarness::local_default()`,
it runs cross-platform with no Docker or KVM — swap in the Docker or Firecracker
backend for real execution.

Mirrors `web-search-tool`. Licensed under Apache-2.0.
