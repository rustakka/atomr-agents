# atomr-agents-sandbox-core

Backend-agnostic core for the atomr-agents **microVM sandbox** subsystem —
secure, instant-boot compute environments that let agents execute untrusted
code (Python / Bash / JS / Rust).

This crate carries no hypervisor or container code. It defines the contract
that every backend implements and every consumer depends on:

- **Traits** — [`SandboxBackend`] (provisions sandboxes; mirrors the
  `Isolator` trait) and [`SandboxHandle`] (one live sandbox: `exec`,
  `write_file`/`read_file`, `snapshot`, `fork`, `destroy`; mirrors
  `ProcessHandle`).
- **Domain types** — `SandboxProfile` (the 5 toolchain profiles), `Language`,
  `ResourceBudget` (with the security-relevant 2 GB / 2 vCPU **Rust floor**),
  `CreateSandbox`, `SandboxInfo`, `ExecRequest`, `ExecResult`, `ExitStatus`,
  `SandboxBackendSel`.
- **Events** — `SandboxEvent` + `SandboxEventStream` (broadcast).
- **`MockBackend`** — a deterministic, in-memory backend so the tool /
  harness / PyO3 layers are unit-testable cross-platform, with no Docker or
  KVM.

Real backends live in sibling crates: `sandbox-backend-docker`,
`sandbox-backend-firecracker`, `sandbox-backend-remote`. The
`execute_in_sandbox` tool lives in `sandbox-tool`; orchestration in
`sandbox-harness`.

Licensed under Apache-2.0.
