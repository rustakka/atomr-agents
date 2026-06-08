"""Facade over :mod:`atomr_agents._native.sandbox`.

Secure, instant-boot microVM sandboxes for executing untrusted agent code
(Python / Bash / JavaScript / Rust). Backed by the deterministic mock backend
via :meth:`SandboxClient.local_default`, it runs cross-platform; the Docker,
Firecracker, and remote-cluster backends slot in behind the same surface.

Ephemeral one-shot::

    from atomr_agents.sandbox import SandboxClient

    client = SandboxClient.local_default()
    result = await client.run({"language": "python", "code": "print(2 + 2)"})
    print(result["stdout"])

Persistent sandbox with a toolchain profile (PRD example)::

    from atomr_agents.sandbox import SandboxClient, SandboxConfig, SandboxProfile

    config = SandboxConfig.cluster(endpoint="grpc://sandbox.internal:50051")
    client = SandboxClient.local_default()
    sandbox = await client.create(SandboxProfile.FullStack, config)

    await sandbox.write_file("src/main.rs", b"fn main() { println!(\\"hi\\"); }")
    out = await sandbox.run("cargo run --release")
    print(out["stdout"])
    await sandbox.destroy()

Branching via fork, and the lifecycle event stream::

    child = await sandbox.fork()           # backs agent branch states
    stream = client.events()
    while (ev := await stream.recv()) is not None:
        print(ev["kind"], ev)
"""

from ._native import sandbox as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
