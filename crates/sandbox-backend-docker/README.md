# atomr-agents-sandbox-backend-docker

The **Docker** backend for the atomr-agents microVM sandbox — the cross-platform
"Insecure Dev Mode" fallback.

Containers share the host kernel, so this is **not** a security boundary for
untrusted code (that is the Firecracker backend's job). It exists so the whole
sandbox surface — `execute_in_sandbox`, the harness, the PyO3 client — runs
end-to-end on a developer laptop with only Docker Desktop: real Python / Bash /
JavaScript / Rust execution, file I/O, and commit-based snapshot/fork.

Model: one long-lived container per sandbox (`sleep infinity`); each `exec` runs
via the Docker exec API inside it. Files are moved with tar
(`upload_to_container` / `download_from_container`), confined to the sandbox
working directory. `snapshot` commits the container to an image; `fork` boots a
fresh container from that image.

Wire it into a harness directly:

```rust
use std::sync::Arc;
use atomr_agents_sandbox_backend_docker::DockerBackend;
use atomr_agents_sandbox_harness::{SandboxHarness, SandboxHarnessConfig, BestFitScheduler};

let backend = Arc::new(DockerBackend::local()?);
let harness = SandboxHarness::new(backend, Arc::new(BestFitScheduler), SandboxHarnessConfig::default());
```

Integration tests skip automatically when no Docker daemon is reachable.

Mirrors the bollard machinery in `coding-cli-isolator`. Licensed under Apache-2.0.
