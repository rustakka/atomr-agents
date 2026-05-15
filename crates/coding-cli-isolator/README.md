# atomr-agents-coding-cli-isolator

Spawns CLI processes for the coding-cli harness, either on the host or
inside a Docker container.

| Backend           | Headless    | Interactive (PTY) |
| ----------------- | ----------- | ----------------- |
| `LocalIsolator`   | tokio       | portable-pty      |
| `DockerIsolator`  | bollard     | bollard TTY exec  |

Disable the `docker` feature to drop the bollard dependency.

The `Isolator` trait normalizes both backends behind one async surface
so the harness doesn't branch on `IsolationSpec` at runtime — it
constructs the right isolator at startup and treats it uniformly.

## Default per-vendor images

The `images/` directory holds the Dockerfiles for the default per-vendor
base images (`atomr-agents/coding-cli-claude`, `...-codex`,
`...-gemini`). Build with:

```bash
docker build -t atomr-agents/coding-cli-claude:latest \
  -f crates/coding-cli-isolator/images/claude.Dockerfile .
```
