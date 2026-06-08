# atomr-agents-sandbox-guest-agent

The in-VM **guest agent** for the atomr-agents microVM sandbox — a small Rust
daemon that runs as **PID 1** inside a Firecracker guest and serves the
[`atomr-agents-sandbox-proto`] command set (`Ping`, `Exec`, `WriteFile`,
`ReadFile`, `Shutdown`).

- **Core (`lib`)** — `serve` / `serve_rw` drive the request/response loop over
  any async reader+writer; `exec` builds and runs the per-language commands;
  file I/O is confined to the sandbox root (`/workspace`). This is
  transport-agnostic and unit-tested over in-memory pipes on every platform.
- **Binary** — serves the protocol over **stdin / stdout**. The production
  transport is `AF_VSOCK`; the host (the Linux Firecracker backend) bridges the
  guest's stdio onto the vsock device, so this binary carries no platform
  socket code and stays a lean static musl build. On Linux it also does
  best-effort PID-1 init (`/proc`, `/sys`, `/tmp`).

Build the static guest binary for the rootfs image:

```sh
cargo build -p atomr-agents-sandbox-guest-agent \
  --release --target x86_64-unknown-linux-musl
```

Licensed under Apache-2.0.
