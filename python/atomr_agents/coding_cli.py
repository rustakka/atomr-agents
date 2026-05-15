"""Facade over :mod:`atomr_agents._native.coding_cli`.

The coding-cli harness wraps local AI coding CLIs (Claude Code, Codex
CLI, Gemini CLI) as atomr-agents callables. Two modes:

* **Headless** — non-interactive, structured event stream:

    .. code-block:: python

        from atomr_agents.coding_cli import CodingCliHarness

        harness = CodingCliHarness.local_default()
        req = {
            "vendor": "claude",
            "mode": "headless",
            "prompt": "list files in src/",
            "workdir": "/path/to/repo",
        }
        result = await harness.run_headless(req)
        print(result["final_text"])

* **Interactive** — tmux-wrapped TUI bridged over a PTY:

    .. code-block:: python

        session = await harness.start_interactive({
            "vendor": "claude",
            "mode": "interactive",
            "workdir": "/path/to/repo",
            "prompt": "",
        })
        await session.send_keys(b"ls\\n")
        chunk = await session.read()   # raw PTY bytes
        await session.stop()

Both modes broadcast the normalized event stream via :meth:`events`::

    stream = harness.events()
    while (ev := await stream.recv()) is not None:
        print(ev["kind"], ev)
"""

from ._native import coding_cli as _sub

globals().update({k: getattr(_sub, k) for k in dir(_sub) if not k.startswith("_")})
__all__ = [k for k in dir(_sub) if not k.startswith("_")]
