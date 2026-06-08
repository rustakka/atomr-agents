"""Smoke tests for the microVM sandbox Python bindings.

Only runs after `maturin develop`. Exercises the FFI shape against the default
local (mock-backed) client — no Docker or KVM required.

The `_native` extension is loaded directly off disk so this test does not
depend on the package facade.
"""

import asyncio
import importlib.machinery
import importlib.util
import pathlib

import pytest


def _load_native():
    pkg_dir = pathlib.Path(__file__).resolve().parents[1]
    candidates = sorted(pkg_dir.glob("_native*.so")) + sorted(pkg_dir.glob("_native*.pyd"))
    if not candidates:
        pytest.skip("native extension not built; run `maturin develop`", allow_module_level=True)
    loader = importlib.machinery.ExtensionFileLoader("_native", str(candidates[0]))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


native = _load_native()


def test_sandbox_module_layout() -> None:
    assert hasattr(native, "sandbox")
    sb = native.sandbox
    for name in (
        "SandboxClient",
        "Sandbox",
        "SandboxConfig",
        "SandboxProfile",
        "SandboxEventStream",
    ):
        assert hasattr(sb, name), f"missing {name}"


def test_profile_enum_values() -> None:
    profile = native.sandbox.SandboxProfile
    assert profile.FullStack.value == "full_stack"
    assert profile.PythonOnly.value == "python_only"
    assert profile.RustOnly.value == "rust_only"


def test_config_constructors_do_not_crash() -> None:
    config = native.sandbox.SandboxConfig
    config.local()
    config.mock()
    config.docker()
    config.firecracker()
    assert "endpoint" in repr(config.cluster("grpc://sandbox.internal:50051")).lower()


def test_client_local_default() -> None:
    client = native.sandbox.SandboxClient.local_default()
    assert client.backend == "mock"
    assert client.live_count() == 0
    assert "SandboxClient" in repr(client)


def test_run_ephemeral() -> None:
    async def go():
        client = native.sandbox.SandboxClient.local_default()
        res = await client.run({"language": "python", "code": "print(2 + 2)"})
        assert res["success"] is True
        assert "[mock:Python]" in res["stdout"]

    asyncio.run(go())


def test_create_exec_files_fork_destroy() -> None:
    async def go():
        sb = native.sandbox
        client = sb.SandboxClient.local_default()
        box = await client.create(sb.SandboxProfile.FullStack)
        await box.write_file("src/main.rs", b"fn main(){}")
        assert (await box.read_file("src/main.rs")) == b"fn main(){}"

        res = await box.exec({"language": "rust", "code": "fn main(){}"})
        assert res["success"] is True

        child = await box.fork()
        assert (await child.read_file("src/main.rs")) == b"fn main(){}"

        snap = await box.snapshot()
        assert isinstance(snap, str)

        await box.destroy()
        await child.destroy()

    asyncio.run(go())
