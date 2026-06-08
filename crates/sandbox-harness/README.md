# atomr-agents-sandbox-harness

Orchestration for the atomr-agents **microVM sandbox**. The harness owns a
pluggable `SandboxBackend`, a registry of live sandboxes, a bin-packing
`Scheduler` and a warm `SnapshotPool`, an `EventBus`, and a broadcast channel
of `SandboxEvent`s.

Two execution paths:

- **Ephemeral one-shot** — `run_once` / `run_exec`: create a sandbox, run one
  exec, destroy it, emitting the full lifecycle. This is what the
  `execute_in_sandbox` tool (`sandbox-tool`) drives.
- **Persistent registry** — `create` → `exec` / `snapshot` / `fork` /
  `destroy` by id, with a concurrency quota. This backs the PyO3
  `SandboxClient` and agent branch states (fork).

`SandboxHarness::local_default()` wires the deterministic `MockBackend` + a
`BestFitScheduler`, so the whole surface runs cross-platform with no Docker or
KVM. The harness is itself a `Callable`, so it can stand in as a workflow step.

Mirrors `coding-cli-harness`. Licensed under Apache-2.0.
