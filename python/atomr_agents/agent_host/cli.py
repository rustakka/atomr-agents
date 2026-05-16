"""atomr-host CLI — entry point for the on-disk agent runtime.

The CLI is intentionally argparse-based (no extra dependency) and
delegates all real work to :mod:`atomr_agents.agent_host` modules so
the same surface is available programmatically.

Subcommands shipped in M1:

* ``atomr-host init [--root PATH] [--force]`` — scaffold a host root.
* ``atomr-host agent new <id> [--model NAME] [--force]`` — scaffold an agent.
* ``atomr-host agent list`` — list agent ids under the host root.
* ``atomr-host agent show <id>`` — print the parsed AgentDefinition.
* ``atomr-host agent rm <id> [--force]`` — remove an agent directory.

Later milestones extend this CLI in-place; the subparser scaffolding is
deliberately written so adding ``chat``, ``cron``, ``channel``, etc.
slots in next to ``agent``.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import shutil
import sys
from pathlib import Path
from typing import Any, Sequence

from .config import HostConfig
from .errors import AgentHostError, AgentNotFoundError
from .layout import default_root
from .loader import AgentLoader
from .scaffold import scaffold_agent, scaffold_host

__all__ = ["build_parser", "main"]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="atomr-host",
        description="Long-lived on-disk runtime for atomr-agents.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help=(
            "Host root directory (default: $ATOMR_HOST_ROOT or "
            "~/.atomr/host)."
        ),
    )
    sub = parser.add_subparsers(dest="command", required=True)

    # ---- init ---------------------------------------------------------
    p_init = sub.add_parser("init", help="scaffold a fresh host root + default agent")
    p_init.add_argument(
        "--force",
        action="store_true",
        help="overwrite existing config.yaml / AGENTS.md",
    )
    p_init.add_argument(
        "--no-default-agent",
        action="store_true",
        help="skip creating the seed `default` agent",
    )
    p_init.set_defaults(func=_cmd_init)

    # ---- agent --------------------------------------------------------
    p_agent = sub.add_parser("agent", help="manage agents on disk")
    agent_sub = p_agent.add_subparsers(dest="agent_command", required=True)

    p_agent_new = agent_sub.add_parser("new", help="scaffold a new agent")
    p_agent_new.add_argument("agent_id")
    p_agent_new.add_argument("--model", default=None, help="override default model")
    p_agent_new.add_argument("--force", action="store_true", help="overwrite files in-place")
    p_agent_new.set_defaults(func=_cmd_agent_new)

    p_agent_list = agent_sub.add_parser("list", help="list agent ids")
    p_agent_list.add_argument(
        "--format",
        choices=("plain", "json"),
        default="plain",
        help="output format",
    )
    p_agent_list.set_defaults(func=_cmd_agent_list)

    p_agent_show = agent_sub.add_parser("show", help="print parsed agent definition")
    p_agent_show.add_argument("agent_id")
    p_agent_show.add_argument(
        "--format",
        choices=("pretty", "json"),
        default="pretty",
        help="output format",
    )
    p_agent_show.set_defaults(func=_cmd_agent_show)

    p_agent_rm = agent_sub.add_parser("rm", help="remove an agent directory")
    p_agent_rm.add_argument("agent_id")
    p_agent_rm.add_argument(
        "--force",
        action="store_true",
        help="proceed without confirmation prompt",
    )
    p_agent_rm.set_defaults(func=_cmd_agent_rm)

    # ---- branch -------------------------------------------------------
    p_branch = sub.add_parser("branch", help="branches + checkpoints (M10)")
    branch_sub = p_branch.add_subparsers(dest="branch_command", required=True)

    p_branch_ls = branch_sub.add_parser("ls", help="list branches for an agent")
    p_branch_ls.add_argument("agent_id")
    p_branch_ls.set_defaults(func=_cmd_branch_ls)

    p_branch_new = branch_sub.add_parser("new", help="fork a new branch from another branch's tip")
    p_branch_new.add_argument("agent_id")
    p_branch_new.add_argument("new_branch")
    p_branch_new.add_argument("--from", dest="source_branch", default="main")
    p_branch_new.set_defaults(func=_cmd_branch_new)

    p_branch_switch = branch_sub.add_parser("switch", help="switch CURRENT to a branch")
    p_branch_switch.add_argument("agent_id")
    p_branch_switch.add_argument("branch")
    p_branch_switch.set_defaults(func=_cmd_branch_switch)

    p_branch_diff = branch_sub.add_parser("diff", help="diff two branches' latest checkpoints")
    p_branch_diff.add_argument("agent_id")
    p_branch_diff.add_argument("branch_a")
    p_branch_diff.add_argument("branch_b")
    p_branch_diff.set_defaults(func=_cmd_branch_diff)

    p_branch_rm = branch_sub.add_parser("rm", help="delete a branch (refuses `main` without --force)")
    p_branch_rm.add_argument("agent_id")
    p_branch_rm.add_argument("branch")
    p_branch_rm.add_argument("--force", action="store_true")
    p_branch_rm.set_defaults(func=_cmd_branch_rm)

    # ---- registry -----------------------------------------------------
    p_reg = sub.add_parser("registry", help="cache + resolve published artifacts (M11)")
    reg_sub = p_reg.add_subparsers(dest="registry_command", required=True)

    p_reg_ls = reg_sub.add_parser("ls", help="list cached artifacts")
    p_reg_ls.add_argument("--kind", default=None, help="filter by artifact kind")
    p_reg_ls.set_defaults(func=_cmd_registry_ls)

    p_reg_resolve = reg_sub.add_parser("resolve", help="resolve <kind>:<id>@<version>")
    p_reg_resolve.add_argument("slug")
    p_reg_resolve.set_defaults(func=_cmd_registry_resolve)

    # ---- eval ---------------------------------------------------------
    p_eval = sub.add_parser("eval", help="run on-disk eval suites against an agent (M12)")
    eval_sub = p_eval.add_subparsers(dest="eval_command", required=True)

    p_eval_ls = eval_sub.add_parser("ls", help="list eval suites under <root>/evals/")
    p_eval_ls.set_defaults(func=_cmd_eval_ls)

    p_eval_new = eval_sub.add_parser("new", help="scaffold a minimal eval suite")
    p_eval_new.add_argument("suite_id")
    p_eval_new.add_argument("--force", action="store_true")
    p_eval_new.set_defaults(func=_cmd_eval_new)

    p_eval_run = eval_sub.add_parser("run", help="run a suite against an agent")
    p_eval_run.add_argument("agent_id")
    p_eval_run.add_argument("suite_id")
    p_eval_run.set_defaults(func=_cmd_eval_run)

    # ---- events -------------------------------------------------------
    p_events = sub.add_parser("events", help="JSONL event log (M9)")
    events_sub = p_events.add_subparsers(dest="events_command", required=True)

    p_events_tail = events_sub.add_parser("tail", help="follow events.jsonl")
    p_events_tail.add_argument(
        "--no-follow", action="store_true", help="print existing lines then exit"
    )
    p_events_tail.add_argument(
        "--poll", type=float, default=0.5, help="poll interval seconds (default: 0.5)"
    )
    p_events_tail.set_defaults(func=_cmd_events_tail)

    p_events_emit = events_sub.add_parser(
        "emit", help="append a record (for testing the log + downstream curator)"
    )
    p_events_emit.add_argument("kind")
    p_events_emit.add_argument("--agent-id", default=None)
    p_events_emit.add_argument(
        "--payload", default="{}", help="JSON payload (default: {})"
    )
    p_events_emit.set_defaults(func=_cmd_events_emit)

    # extend `skill` with history / revert / review --------------------
    # (subparser was created earlier; add the three commands now)
    # Cron / routes / MCP / hooks subparsers already created earlier.

    # ---- cron ---------------------------------------------------------
    p_cron = sub.add_parser("cron", help="manage scheduled jobs (M6)")
    cron_sub = p_cron.add_subparsers(dest="cron_command", required=True)

    p_cron_add = cron_sub.add_parser("add", help="scaffold a new cron entry")
    p_cron_add.add_argument("cron_id")
    p_cron_add.add_argument(
        "--when", default="every:1h", help="schedule expression (default: every:1h)"
    )
    p_cron_add.add_argument(
        "--call",
        default='{"kind":"builtin","id":"noop"}',
        help='JSON for the `call` block (default: {"kind":"builtin","id":"noop"})',
    )
    p_cron_add.add_argument("--input", default="{}", help="JSON payload (default: {})")
    p_cron_add.add_argument("--force", action="store_true", help="overwrite if file exists")
    p_cron_add.set_defaults(func=_cmd_cron_add)

    p_cron_ls = cron_sub.add_parser("ls", help="list cron entries under the host root")
    p_cron_ls.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_cron_ls.set_defaults(func=_cmd_cron_ls)

    p_cron_rm = cron_sub.add_parser("rm", help="remove a cron entry")
    p_cron_rm.add_argument("cron_id")
    p_cron_rm.add_argument("--force", action="store_true", help="no confirmation prompt")
    p_cron_rm.set_defaults(func=_cmd_cron_rm)

    # ---- gateway / routes --------------------------------------------
    p_gw = sub.add_parser("routes", help="inspect AGENTS.md routing rules (M7)")
    p_gw.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_gw.set_defaults(func=_cmd_routes)

    # ---- mcp ---------------------------------------------------------
    p_mcp = sub.add_parser("mcp", help="manage MCP tool servers (M8)")
    mcp_sub = p_mcp.add_subparsers(dest="mcp_command", required=True)

    p_mcp_add = mcp_sub.add_parser("add", help="scaffold an MCP server config")
    p_mcp_add.add_argument("tool_id")
    p_mcp_add.add_argument(
        "--command",
        required=True,
        help="shell-quoted command to launch the MCP server (e.g. 'npx @mcp/fs-server .')",
    )
    p_mcp_add.add_argument("--description", default=None)
    p_mcp_add.add_argument("--force", action="store_true")
    p_mcp_add.set_defaults(func=_cmd_mcp_add)

    p_mcp_ls = mcp_sub.add_parser("ls", help="list configured MCP servers")
    p_mcp_ls.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_mcp_ls.set_defaults(func=_cmd_mcp_ls)

    # ---- hooks --------------------------------------------------------
    p_hooks = sub.add_parser("hooks", help="manage / inspect agent hooks (M5)")
    hooks_sub = p_hooks.add_subparsers(dest="hooks_command", required=True)

    p_hooks_ls = hooks_sub.add_parser("ls", help="list hooks for an agent")
    p_hooks_ls.add_argument("agent_id")
    p_hooks_ls.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_hooks_ls.set_defaults(func=_cmd_hooks_ls)

    p_hooks_test = hooks_sub.add_parser(
        "test", help="dispatch a synthetic event through an agent's hooks"
    )
    p_hooks_test.add_argument("agent_id")
    p_hooks_test.add_argument("event", help="event name to dispatch")
    p_hooks_test.add_argument(
        "--payload",
        default="{}",
        help='JSON payload (default: {}). Example: \'{"tool":"shell.exec","text":"api_key=sk-..."}\'',
    )
    p_hooks_test.add_argument(
        "--when",
        choices=("pre", "post", "both"),
        default=None,
        help="only fire hooks with this `when` (default: any)",
    )
    p_hooks_test.set_defaults(func=_cmd_hooks_test)

    # ---- skill --------------------------------------------------------
    p_skill = sub.add_parser("skill", help="manage agent skills (M4)")
    skill_sub = p_skill.add_subparsers(dest="skill_command", required=True)

    p_skill_new = skill_sub.add_parser("new", help="scaffold a new SKILL.md under an agent")
    p_skill_new.add_argument("agent_id")
    p_skill_new.add_argument("skill_id")
    p_skill_new.add_argument("--name", default=None, help="display name (default: skill_id)")
    p_skill_new.add_argument("--priority", type=int, default=5, help="0..10 (default: 5)")
    p_skill_new.add_argument(
        "--keywords",
        default=None,
        help="comma-separated trigger keywords (default: skill_id)",
    )
    p_skill_new.add_argument("--force", action="store_true", help="overwrite if file exists")
    p_skill_new.set_defaults(func=_cmd_skill_new)

    p_skill_ls = skill_sub.add_parser("ls", help="list parsed skills for an agent")
    p_skill_ls.add_argument("agent_id")
    p_skill_ls.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_skill_ls.set_defaults(func=_cmd_skill_ls)

    p_skill_val = skill_sub.add_parser("validate", help="validate every SKILL.md under an agent")
    p_skill_val.add_argument("agent_id")
    p_skill_val.add_argument(
        "--format", choices=("plain", "json"), default="plain", help="output format"
    )
    p_skill_val.set_defaults(func=_cmd_skill_validate)

    p_skill_hist = skill_sub.add_parser(
        "history", help="list .history snapshots of a curated skill (M9)"
    )
    p_skill_hist.add_argument("agent_id")
    p_skill_hist.add_argument("skill_id")
    p_skill_hist.set_defaults(func=_cmd_skill_history)

    p_skill_revert = skill_sub.add_parser(
        "revert", help="restore the most recent .history snapshot (M9)"
    )
    p_skill_revert.add_argument("agent_id")
    p_skill_revert.add_argument("skill_id")
    p_skill_revert.add_argument(
        "--snapshot",
        default=None,
        help="path to a specific snapshot (default: most recent)",
    )
    p_skill_revert.set_defaults(func=_cmd_skill_revert)

    p_skill_rev = skill_sub.add_parser(
        "review", help="approve/reject pending curator proposals (M9)"
    )
    p_skill_rev.add_argument("agent_id")
    p_skill_rev.add_argument("skill_id", nargs="?", default=None)
    p_skill_rev.add_argument(
        "--approve", action="store_true", help="promote the proposal to live"
    )
    p_skill_rev.add_argument(
        "--reject", action="store_true", help="discard the proposal"
    )
    p_skill_rev.set_defaults(func=_cmd_skill_review)

    # ---- memory -------------------------------------------------------
    p_memory = sub.add_parser("memory", help="inspect / sync MEMORY.md and USER.md (M3)")
    memory_sub = p_memory.add_subparsers(dest="memory_command", required=True)

    p_mem_sync = memory_sub.add_parser(
        "sync", help="push MEMORY.md + USER.md bullets into an in-memory MemoryStore"
    )
    p_mem_sync.add_argument("agent_id")
    p_mem_sync.set_defaults(func=_cmd_memory_sync)

    p_mem_show = memory_sub.add_parser(
        "show", help="print parsed MEMORY.md / USER.md bullets for an agent"
    )
    p_mem_show.add_argument("agent_id")
    p_mem_show.set_defaults(func=_cmd_memory_show)

    # ---- rules --------------------------------------------------------
    p_rules = sub.add_parser("rules", help="render persona + rules + memory (M3)")
    rules_sub = p_rules.add_subparsers(dest="rules_command", required=True)

    p_rules_show = rules_sub.add_parser(
        "show", help="print the assembled system prompt for an agent"
    )
    p_rules_show.add_argument("agent_id")
    p_rules_show.set_defaults(func=_cmd_rules_show)

    # ---- chat ---------------------------------------------------------
    p_chat = sub.add_parser(
        "chat",
        help="open an interactive local chat with an agent (M2)",
    )
    p_chat.add_argument("agent_id", help="agent to chat with")
    p_chat.add_argument(
        "--channel-id",
        default="cli:local",
        help="logical channel id for this session (default: cli:local)",
    )
    p_chat.add_argument(
        "--peer",
        default="user",
        help="peer name for the thread (default: user)",
    )
    p_chat.add_argument(
        "--no-persist",
        action="store_true",
        help="skip writing the thread JSONL log to disk",
    )
    p_chat.set_defaults(func=_cmd_chat)

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return int(args.func(args) or 0)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:  # pragma: no cover
        print("interrupted", file=sys.stderr)
        return 130


# ---------- subcommand implementations --------------------------------


def _resolve_config(args: argparse.Namespace) -> HostConfig:
    root = args.root if args.root is not None else default_root()
    return HostConfig.load(root)


def _cmd_init(args: argparse.Namespace) -> int:
    root = args.root if args.root is not None else default_root()
    cfg = scaffold_host(root, force=args.force)
    print(f"initialized host root at {cfg.paths.root}")
    if not args.no_default_agent:
        paths = scaffold_agent(cfg, "default", force=args.force)
        print(f"  + seeded agent at {paths.dir}")
    return 0


def _cmd_agent_new(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    paths = scaffold_agent(cfg, args.agent_id, model=args.model, force=args.force)
    print(f"created agent `{args.agent_id}` at {paths.dir}")
    return 0


def _cmd_agent_list(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    ids = AgentLoader(cfg).agent_ids()
    if args.format == "json":
        print(json.dumps(ids))
    else:
        for aid in ids:
            print(aid)
    return 0


def _cmd_agent_show(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    loader = AgentLoader(cfg)
    try:
        defn = loader.parse(args.agent_id)
    except AgentNotFoundError:
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2

    if args.format == "json":
        print(json.dumps(_definition_to_json(defn), indent=2, default=str))
    else:
        _print_definition(defn)
    return 0


def _cmd_branch_ls(args: argparse.Namespace) -> int:
    from .branching import current_branch, list_branches, latest_checkpoint

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    if not paths.dir.is_dir():
        print(f"error: no agent `{args.agent_id}`", file=sys.stderr)
        return 2
    current = current_branch(paths)
    branches = list_branches(paths)
    if not branches:
        print(f"(no branches under {paths.checkpoints_dir})")
        return 0
    for b in branches:
        marker = "*" if b == current else " "
        latest = latest_checkpoint(paths, b)
        ts = latest.ts_ms if latest else "—"
        print(f"{marker} {b}  latest_ts_ms={ts}")
    return 0


def _cmd_branch_new(args: argparse.Namespace) -> int:
    from .branching import fork_branch

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    try:
        cp = fork_branch(paths, source_branch=args.source_branch, new_branch=args.new_branch)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(f"forked {args.new_branch} from {args.source_branch} → {cp.path}")
    return 0


def _cmd_branch_switch(args: argparse.Namespace) -> int:
    from .branching import switch_branch

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    try:
        cp = switch_branch(paths, args.branch)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(f"current branch is now `{args.branch}` (ts={cp.ts_ms})")
    return 0


def _cmd_branch_diff(args: argparse.Namespace) -> int:
    from .branching import diff_branches

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    try:
        diff = diff_branches(paths, args.branch_a, args.branch_b)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(diff, indent=2, default=str))
    return 0


def _cmd_branch_rm(args: argparse.Namespace) -> int:
    from .branching import delete_branch

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    try:
        delete_branch(paths, args.branch, force=args.force)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(f"deleted branch `{args.branch}`")
    return 0


def _cmd_registry_ls(args: argparse.Namespace) -> int:
    from .registry import list_artifacts

    cfg = _resolve_config(args)
    arts = list_artifacts(cfg.paths, kind=args.kind)
    if not arts:
        print(f"(no cached artifacts under {cfg.paths.registry_dir})")
        return 0
    for a in arts:
        print(f"{a.slug}  ({a.path})")
    return 0


def _cmd_registry_resolve(args: argparse.Namespace) -> int:
    from .registry import parse_slug, resolve_artifact

    cfg = _resolve_config(args)
    try:
        kind, id_, version = parse_slug(args.slug)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    art = resolve_artifact(cfg.paths, kind=kind, id=id_, version=version)
    if art is None:
        print(f"not cached: {args.slug}")
        return 2
    print(json.dumps({"slug": art.slug, "path": str(art.path), "payload": art.payload}, indent=2))
    return 0


def _cmd_eval_ls(args: argparse.Namespace) -> int:
    from .evals import list_suites

    cfg = _resolve_config(args)
    ids = list_suites(cfg)
    if not ids:
        print(f"(no suites under {cfg.paths.root / 'evals'})")
        return 0
    for sid in ids:
        print(sid)
    return 0


def _cmd_eval_new(args: argparse.Namespace) -> int:
    from .evals import scaffold_suite

    cfg = _resolve_config(args)
    target = scaffold_suite(cfg, args.suite_id, force=args.force)
    print(f"wrote {target}")
    return 0


def _cmd_eval_run(args: argparse.Namespace) -> int:
    from .evals import load_suite, run_suite_sync

    cfg = _resolve_config(args)
    try:
        suite = load_suite(cfg, args.suite_id)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    run = run_suite_sync(cfg, args.agent_id, suite)
    for r in run.results:
        status = "PASS" if r.passed else "FAIL"
        reason = f" — {r.reason}" if r.reason else ""
        print(f"[{status}] {r.case_id}  score={r.score:.2f}{reason}")
    print(
        f"suite={run.suite_id} agent={run.agent_id}  passed={run.passed}/"
        f"{len(run.results)}  pass_rate={run.pass_rate:.2%}"
    )
    return 0 if run.pass_rate == 1.0 else 1


def _cmd_events_tail(args: argparse.Namespace) -> int:
    import asyncio as _asyncio

    from .events import EventLog

    cfg = _resolve_config(args)
    log = EventLog(cfg.paths.events_jsonl)

    async def _run() -> None:
        async for rec in log.tail(follow=not args.no_follow, poll_seconds=args.poll):
            line = {"ts_ms": rec.ts_ms, "kind": rec.kind}
            if rec.agent_id:
                line["agent_id"] = rec.agent_id
            if rec.payload:
                line["payload"] = rec.payload
            print(json.dumps(line))

    try:
        _asyncio.run(_run())
    except KeyboardInterrupt:
        pass
    return 0


def _cmd_events_emit(args: argparse.Namespace) -> int:
    from .events import EventLog

    cfg = _resolve_config(args)
    try:
        payload = json.loads(args.payload)
    except json.JSONDecodeError as exc:
        print(f"error: invalid --payload JSON: {exc}", file=sys.stderr)
        return 2
    if not isinstance(payload, dict):
        print("error: --payload must be a JSON object", file=sys.stderr)
        return 2
    log = EventLog(cfg.paths.events_jsonl)
    rec = log.emit(args.kind, agent_id=args.agent_id, **payload)
    print(json.dumps({"ts_ms": rec.ts_ms, "kind": rec.kind}))
    return 0


def _cmd_skill_history(args: argparse.Namespace) -> int:
    from .curator import list_history

    cfg = _resolve_config(args)
    paths = list_history(cfg, args.agent_id, args.skill_id)
    if not paths:
        print(f"(no history for {args.agent_id}/{args.skill_id})")
        return 0
    for p in paths:
        print(p)
    return 0


def _cmd_skill_revert(args: argparse.Namespace) -> int:
    from .curator import revert_skill

    cfg = _resolve_config(args)
    target = Path(args.snapshot) if args.snapshot else None
    try:
        outcome = revert_skill(cfg, args.agent_id, args.skill_id, target=target)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    status = "OK" if outcome.accepted else "FAIL"
    print(f"[{status}] {outcome.reason or '(no reason)'} → {outcome.target_path}")
    return 0 if outcome.accepted else 1


def _cmd_skill_review(args: argparse.Namespace) -> int:
    from .curator import list_proposals, promote_proposal, reject_proposal

    cfg = _resolve_config(args)
    if not args.skill_id:
        proposals = list_proposals(cfg, args.agent_id)
        if not proposals:
            print(f"(no pending proposals for agent `{args.agent_id}`)")
            return 0
        for p in proposals:
            print(p)
        return 0
    if args.approve == args.reject:
        print("error: pass exactly one of --approve or --reject", file=sys.stderr)
        return 2
    fn = promote_proposal if args.approve else reject_proposal
    try:
        outcome = fn(cfg, args.agent_id, args.skill_id)
    except AgentHostError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    status = "OK" if outcome.accepted or not args.approve else "FAIL"
    print(f"[{status}] {outcome.reason or '(no reason)'} → {outcome.target_path}")
    return 0


def _cmd_cron_add(args: argparse.Namespace) -> int:
    from .scheduler import scaffold_cron

    cfg = _resolve_config(args)
    try:
        call = json.loads(args.call)
        input_payload = json.loads(args.input)
    except json.JSONDecodeError as exc:
        print(f"error: invalid JSON: {exc}", file=sys.stderr)
        return 2
    target = scaffold_cron(
        cfg, args.cron_id,
        when=args.when, call=call, input=input_payload, force=args.force,
    )
    print(f"wrote {target}")
    return 0


def _cmd_cron_ls(args: argparse.Namespace) -> int:
    from .scheduler import load_crons

    cfg = _resolve_config(args)
    entries = load_crons(cfg)
    if args.format == "json":
        print(json.dumps([
            {
                "id": e.id, "expression": e.expression,
                "call": dict(e.call), "input": dict(e.input), "enabled": e.enabled,
            } for e in entries
        ], indent=2))
        return 0
    if not entries:
        print(f"(no crons under {cfg.paths.crons_dir})")
        return 0
    for e in entries:
        call_id = e.call.get("id") or e.call.get("kind") or "?"
        print(f"{e.id}  when={e.expression}  call={call_id}  enabled={e.enabled}")
    return 0


def _cmd_cron_rm(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    target = cfg.paths.crons_dir / f"{args.cron_id}.yaml"
    if not target.is_file():
        print(f"no cron `{args.cron_id}` at {target}", file=sys.stderr)
        return 2
    if not args.force:
        resp = input(f"remove {target}? [y/N] ")
        if resp.strip().lower() not in {"y", "yes"}:
            print("aborted")
            return 1
    target.unlink()
    print(f"removed {target}")
    return 0


def _cmd_routes(args: argparse.Namespace) -> int:
    from .gateway import build_router, load_agents_md

    cfg = _resolve_config(args)
    rules = load_agents_md(cfg.paths)
    router = build_router(cfg, agents_md=rules)
    if args.format == "json":
        print(json.dumps({
            "default_agent": router.default_agent,
            "channel_pins": dict(router.channel_pins),
            "peer_pins": [
                {"channel_id": k[0], "peer": k[1], "agent_id": v}
                for k, v in router.peer_pins.items()
            ],
        }, indent=2))
        return 0
    print(f"default_agent: {router.default_agent or '(unset)'}")
    print("channel pins:")
    if not router.channel_pins:
        print("  (none)")
    for ch, ag in router.channel_pins.items():
        print(f"  {ch} → {ag}")
    print("peer pins:")
    if not router.peer_pins:
        print("  (none)")
    for (ch, peer), ag in router.peer_pins.items():
        print(f"  {ch} {peer} → {ag}")
    return 0


def _cmd_mcp_add(args: argparse.Namespace) -> int:
    import shlex
    from .mcp import scaffold_mcp_tool

    cfg = _resolve_config(args)
    cmd_list = shlex.split(args.command)
    if not cmd_list:
        print("error: --command must contain at least one token", file=sys.stderr)
        return 2
    target = scaffold_mcp_tool(
        cfg.paths, args.tool_id,
        command=cmd_list, description=args.description, force=args.force,
    )
    print(f"wrote {target}")
    return 0


def _cmd_mcp_ls(args: argparse.Namespace) -> int:
    from .mcp import load_mcp_servers

    cfg = _resolve_config(args)
    servers = load_mcp_servers(cfg.paths)
    if args.format == "json":
        print(json.dumps([
            {
                "id": s.id, "command": list(s.command),
                "description": s.description, "env": dict(s.env),
            } for s in servers
        ], indent=2))
        return 0
    if not servers:
        print(f"(no MCP servers under {cfg.paths.tools_dir})")
        return 0
    for s in servers:
        desc = f" — {s.description}" if s.description else ""
        print(f"{s.id}  cmd={' '.join(s.command)}{desc}")
    return 0


def _cmd_hooks_ls(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    try:
        defn = AgentLoader(cfg).parse(args.agent_id)
    except AgentNotFoundError:
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    if args.format == "json":
        items = [
            {
                "event": h.event,
                "when": h.when,
                "match": dict(h.match),
                "call": dict(h.call),
                "budget": dict(h.budget),
                "source_path": str(h.source_path) if h.source_path else None,
            }
            for h in defn.hooks
        ]
        print(json.dumps(items, indent=2))
        return 0
    if not defn.hooks:
        print(f"(no hooks under {cfg.paths.agent(args.agent_id).hooks_dir})")
        return 0
    for h in defn.hooks:
        match = ",".join(f"{k}={v}" for k, v in h.match.items()) or "*"
        call = h.call.get("id") or h.call.get("kind") or "?"
        print(f"{h.event}  when={h.when}  match=[{match}]  call={call}")
    return 0


def _cmd_hooks_test(args: argparse.Namespace) -> int:
    import asyncio as _asyncio

    from .hooks import HookDispatcher, HookRegistry, default_hook_resolver

    cfg = _resolve_config(args)
    try:
        defn = AgentLoader(cfg).parse(args.agent_id)
    except AgentNotFoundError:
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    try:
        payload = json.loads(args.payload)
    except json.JSONDecodeError as exc:
        print(f"error: --payload is not valid JSON: {exc}", file=sys.stderr)
        return 2
    if not isinstance(payload, dict):
        print("error: --payload must be a JSON object", file=sys.stderr)
        return 2

    registry = HookRegistry()
    jsonl_path = cfg.paths.events_jsonl
    resolver = default_hook_resolver(jsonl_path=jsonl_path)
    registry.register_definitions(defn.hooks, resolver)
    dispatcher = HookDispatcher(registry)
    results = _asyncio.run(
        dispatcher.dispatch(args.event, payload, when=args.when, ctx={"agent_id": args.agent_id})
    )
    print(f"dispatched {args.event} → {len(results)} hook(s)")
    for r in results:
        status = "OK" if r.ok else "FAIL"
        out_preview = ""
        if isinstance(r.output, dict):
            out_preview = (
                f" output_keys={list(r.output)}" if r.output else " output={}"
            )
        err_preview = f" error={r.error}" if r.error else ""
        print(
            f"  [{status}] {r.hook_id} ({r.duration_ms:.1f}ms when={r.when}){out_preview}{err_preview}"
        )
    return 0 if all(r.ok for r in results) else 1


def _cmd_skill_new(args: argparse.Namespace) -> int:
    from .skills import scaffold_skill

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    if not paths.dir.is_dir():
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    keywords = (
        [k.strip() for k in args.keywords.split(",") if k.strip()]
        if args.keywords
        else None
    )
    target = scaffold_skill(
        paths,
        args.skill_id,
        name=args.name,
        priority=args.priority,
        keywords=keywords,
        force=args.force,
    )
    print(f"wrote {target}")
    return 0


def _cmd_skill_ls(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    try:
        defn = AgentLoader(cfg).parse(args.agent_id)
    except AgentNotFoundError:
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    if args.format == "json":
        items = [
            {
                "id": s.id,
                "name": s.name,
                "priority": s.priority,
                "keywords": list(s.keywords),
                "tool_overlay": list(s.tool_overlay),
            }
            for s in defn.skills
        ]
        print(json.dumps(items, indent=2))
        return 0
    if not defn.skills:
        print(f"(no skills under {cfg.paths.agent(args.agent_id).skills_dir})")
        return 0
    for s in defn.skills:
        kws = ", ".join(s.keywords) or "-"
        print(f"{s.id}  priority={s.priority}  keywords=[{kws}]  name={s.name!r}")
    return 0


def _cmd_skill_validate(args: argparse.Namespace) -> int:
    from .skills import report_to_dict, validate_skills

    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    if not paths.dir.is_dir():
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    reports = validate_skills(paths)
    if args.format == "json":
        print(json.dumps([report_to_dict(r) for r in reports], indent=2))
        return 0 if all(r.ok for r in reports) else 1
    if not reports:
        print(f"(no skills under {paths.skills_dir})")
        return 0
    bad = 0
    for r in reports:
        status = "OK" if r.ok else "FAIL"
        print(f"[{status}] {r.skill_id}  ({r.path})")
        for e in r.errors:
            print(f"    error:   {e}")
        for w in r.warnings:
            print(f"    warning: {w}")
        if not r.ok:
            bad += 1
    if bad:
        print(f"{bad} of {len(reports)} skill(s) failed validation", file=sys.stderr)
        return 1
    return 0


def _cmd_memory_sync(args: argparse.Namespace) -> int:
    import asyncio as _asyncio

    from atomr_agents import _native as _native_pkg  # noqa: F401  - native gate
    from .markdown_sync import sync_all

    cfg = _resolve_config(args)
    loaded = AgentLoader(cfg).load(args.agent_id)
    store = _native_pkg.memory.in_memory_store()
    counts = _asyncio.run(sync_all(loaded, store))
    print(f"synced {args.agent_id}: memory_md={counts['memory_md']}, user_md={counts['user_md']}")
    return 0


def _cmd_memory_show(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    loader = AgentLoader(cfg)
    try:
        defn = loader.parse(args.agent_id)
    except AgentNotFoundError:
        print(f"error: no agent `{args.agent_id}` under {cfg.paths.agents_dir}", file=sys.stderr)
        return 2
    from .loader import _split_rules

    memory_bullets = _split_rules(defn.memory.body)
    user_bullets = _split_rules(defn.user.body)
    print(f"agent: {args.agent_id}")
    print(f"MEMORY.md ({len(memory_bullets)} bullets):")
    for b in memory_bullets:
        print(f"  - {b}")
    print(f"USER.md ({len(user_bullets)} bullets):")
    for b in user_bullets:
        print(f"  - {b}")
    return 0


def _cmd_rules_show(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    loaded = AgentLoader(cfg).load(args.agent_id)
    from .rules import build_system_prompt

    prompt = build_system_prompt(loaded)
    print(prompt)
    return 0


def _cmd_chat(args: argparse.Namespace) -> int:
    from .chat import chat_repl  # imported lazily so `init` works without _native

    cfg = _resolve_config(args)
    loader = AgentLoader(cfg)
    loaded = loader.load(args.agent_id)
    chat_repl(
        loaded,
        channel_id=args.channel_id,
        peer=args.peer,
        persist=not args.no_persist,
    )
    return 0


def _cmd_agent_rm(args: argparse.Namespace) -> int:
    cfg = _resolve_config(args)
    paths = cfg.paths.agent(args.agent_id)
    if not paths.dir.is_dir():
        print(f"no agent `{args.agent_id}` at {paths.dir}", file=sys.stderr)
        return 2
    if not args.force:
        resp = input(f"remove {paths.dir} and everything under it? [y/N] ")
        if resp.strip().lower() not in {"y", "yes"}:
            print("aborted")
            return 1
    shutil.rmtree(paths.dir)
    print(f"removed {paths.dir}")
    return 0


# ---------- formatting helpers ----------------------------------------


def _definition_to_json(defn: Any) -> dict[str, Any]:
    """Convert an AgentDefinition (dataclass tree) into JSON-safe dicts."""
    return {
        "agent_id": defn.agent_id,
        "model": defn.model,
        "paths": {
            "dir": str(defn.paths.dir),
            "agent_yaml": str(defn.paths.agent_yaml),
        },
        "spec": {
            "max_iterations": defn.max_iterations,
            "token_budget": defn.token_budget,
            "time_budget_ms": defn.time_budget_ms,
            "money_budget_usd": defn.money_budget_usd,
            "skillset_id": defn.skillset_id,
            "skillset_version": defn.skillset_version,
        },
        "soul": {
            "frontmatter": defn.soul.frontmatter,
            "body": defn.soul.body,
        },
        "rules_body": defn.rules.body,
        "memory_body": defn.memory.body,
        "user_body": defn.user.body,
        "skills": [_dataclass_to_dict(s) for s in defn.skills],
        "hooks": [_dataclass_to_dict(h) for h in defn.hooks],
    }


def _dataclass_to_dict(obj: Any) -> dict[str, Any]:
    if dataclasses.is_dataclass(obj):
        return {f.name: _coerce(getattr(obj, f.name)) for f in dataclasses.fields(obj)}
    return {}


def _coerce(value: Any) -> Any:
    if isinstance(value, Path):
        return str(value)
    if dataclasses.is_dataclass(value):
        return _dataclass_to_dict(value)
    if isinstance(value, list):
        return [_coerce(v) for v in value]
    if isinstance(value, dict):
        return {k: _coerce(v) for k, v in value.items()}
    return value


def _print_definition(defn: Any) -> None:
    print(f"agent: {defn.agent_id}")
    print(f"  dir:   {defn.paths.dir}")
    print(f"  model: {defn.model or '(unset)'}")
    print("  spec:")
    print(f"    max_iterations:  {defn.max_iterations}")
    print(f"    token_budget:    {defn.token_budget}")
    print(f"    time_budget_ms:  {defn.time_budget_ms}")
    print(f"    money_budget_usd:{defn.money_budget_usd}")
    print(f"    skillset:        {defn.skillset_id}@{defn.skillset_version}")
    if defn.soul.frontmatter:
        identity = defn.soul.frontmatter.get("identity", "(unset)")
        print(f"  soul.identity:  {identity}")
    print(f"  rules:   {sum(1 for line in defn.rules.body.splitlines() if line.strip())} line(s)")
    print(f"  memory:  {sum(1 for line in defn.memory.body.splitlines() if line.strip())} line(s)")
    print(f"  user:    {sum(1 for line in defn.user.body.splitlines() if line.strip())} line(s)")
    print(f"  skills:  {len(defn.skills)}")
    for sd in defn.skills:
        kws = f" keywords={sd.keywords}" if sd.keywords else ""
        print(f"    - {sd.id} (priority={sd.priority}){kws}")
    print(f"  hooks:   {len(defn.hooks)}")
    for h in defn.hooks:
        print(f"    - {h.event} when={h.when}")


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
