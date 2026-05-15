"""Smoke tests for the coding-cli harness Python bindings.

Only runs after `maturin develop`. Exercises the FFI shape against the
default local harness — no actual CLI binaries required for the
constructor / layout assertions.

The `_native` extension is loaded directly off disk so this test does
not depend on the package facade.
"""

import asyncio
import importlib.machinery
import importlib.util
import pathlib
import tempfile

import pytest


def _load_native():
    pkg_dir = pathlib.Path(__file__).resolve().parents[1]
    candidates = sorted(pkg_dir.glob("_native*.so"))
    if not candidates:
        pytest.skip("native extension not built; run `maturin develop`")
    loader = importlib.machinery.ExtensionFileLoader("_native", str(candidates[0]))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


native = _load_native()


def test_coding_cli_module_layout() -> None:
    assert hasattr(native, "coding_cli")
    cc = native.coding_cli
    for name in ("CodingCliHarness", "CodingCliEventStream", "InteractiveSession"):
        assert hasattr(cc, name), f"missing {name}"


def test_local_default_lists_vendors() -> None:
    h = native.coding_cli.CodingCliHarness.local_default()
    vendors = h.vendors()
    # In the default build we wire claude/codex/gemini in.
    assert isinstance(vendors, list)
    for v in vendors:
        assert isinstance(v, str)
    # repr should not crash.
    assert "CodingCliHarness" in repr(h)


def test_run_headless_rejects_bad_workdir() -> None:
    async def go():
        h = native.coding_cli.CodingCliHarness.local_default()
        req = {
            "vendor": "claude",
            "mode": "headless",
            "prompt": "noop",
            "workdir": "/this/does/not/exist",
        }
        with pytest.raises(RuntimeError):
            await h.run_headless(req)

    asyncio.run(go())


def test_run_headless_rejects_unknown_vendor() -> None:
    async def go():
        h = native.coding_cli.CodingCliHarness.local_default()
        with tempfile.TemporaryDirectory() as tmp:
            req = {
                "vendor": {"other": "definitely-not-real"},
                "mode": "headless",
                "prompt": "noop",
                "workdir": tmp,
            }
            with pytest.raises(RuntimeError):
                await h.run_headless(req)

    asyncio.run(go())


def test_sessions_starts_empty() -> None:
    h = native.coding_cli.CodingCliHarness.local_default()
    assert h.sessions() == []
