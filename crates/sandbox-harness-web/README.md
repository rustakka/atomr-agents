# atomr-agents-sandbox-harness-web

Axum REST + SSE companion for the atomr-agents sandbox harness.

| Method | Path | Body → Response |
|---|---|---|
| POST | `/run` | `{language, code, profile?, …}` → flat exec result |
| POST | `/sandboxes` | `CreateSandbox` → `SandboxInfo` |
| GET | `/sandboxes` | → `[SandboxInfo]` |
| POST | `/sandboxes/:id/exec` | `ExecRequest` → flat exec result |
| POST | `/sandboxes/:id/fork` | → `SandboxInfo` |
| POST | `/sandboxes/:id/snapshot` | → `{ snapshot_id }` |
| DELETE | `/sandboxes/:id` | → `204` |
| GET | `/events` | → SSE stream of `SandboxEvent`s |
| GET | `/healthz` | → `{ status, backend, live }` |

The bundled binary serves a mock-backed harness:

```sh
cargo run -p atomr-agents-sandbox-harness-web   # listens on 0.0.0.0:8080
```

Wire a Docker- or Firecracker-backed `SandboxHarness` into `WebServer::new`
for real execution. Mirrors `channel-harness-web`. Licensed under Apache-2.0.
