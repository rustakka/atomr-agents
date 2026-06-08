# atomr-agents-sandbox-proto

The host ↔ guest **wire protocol** for the atomr-agents microVM sandbox.

Communication is **length-prefixed `postcard` frames** (`u32` little-endian
length + postcard body) over `AF_VSOCK`:

- `GuestRequest` — `Ping`, `Exec`, `WriteFile`, `ReadFile`, `Shutdown`.
- `GuestResponse` — `Pong`, streamed `ExecStdout`/`ExecStderr`, `ExecDone`,
  `FileWritten`, `FileRead`, `Error`.
- `read_frame` / `write_frame` work over any `tokio::io::AsyncRead`/`AsyncWrite`
  (in-memory pipes in tests, a real vsock socket in the guest), with a 64 MiB
  frame cap that rejects a corrupt length prefix before allocating.

A hand-rolled framed protocol (not gRPC-over-vsock) keeps the in-VM guest agent
a tiny static binary — no tonic / h2 / tower stack inside the VM. gRPC is used
only host-to-host in the Tier-3 cluster, where vsock frames are tunneled inside
the node ↔ control-plane stream.

Consumed by `sandbox-guest-agent` (the in-VM PID 1 daemon) and, on the host
side, by the Firecracker backend. Licensed under Apache-2.0.
