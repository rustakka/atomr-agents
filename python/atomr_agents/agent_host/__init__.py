"""Agent-host runtime — long-lived process + on-disk layout for atomr-agents.

The host gives an atomr-agents agent persistent identity, memory, skills,
rules, tools, hooks, schedules, and inbound channels — what Claude Code
provides for the Claude model.

This package is intentionally pure-Python: the loader reads YAML/Markdown
from disk and assembles instances of the existing native types
(``AgentSpec``, ``Skill``, ``SkillSet``, ``PersonaValue``, etc.) shipped
by ``atomr_agents._native``. A native ``crates/host`` Rust crate may
land later for hot-reload, cron, and MCP work; nothing here depends on
it.

Common entry points::

    from atomr_agents.agent_host import HostConfig, AgentLoader, layout

    cfg = HostConfig.load_default()         # ~/.atomr/host/config.yaml
    loader = AgentLoader(cfg)
    loaded = loader.load("default")          # LoadedAgent
    spec = loaded.spec                       # native AgentSpec
    persona = loaded.persona                 # PersonaValue
    skills = loaded.skill_set                # SkillSet

The on-disk layout is documented in :mod:`atomr_agents.agent_host.layout`
and at ``docs/agent-host/layout.md``.
"""

from .chat import (
    AgentRouter,
    ChatSession,
    build_chat_callable,
    chat_repl,
    render_chat_preview,
    thread_log_path,
)
from .branching import (
    DEFAULT_BRANCH,
    Checkpoint,
    current_branch,
    delete_branch,
    diff_branches,
    fork_branch,
    latest_checkpoint,
    list_branches,
    list_checkpoints,
    prune_branch,
    switch_branch,
    write_checkpoint,
)
from .config import HostConfig, ProviderConfig
from .curator import (
    AutoPromoteCurationStrategy,
    CurationCtx,
    CurationOutcome,
    CurationStrategy,
    HumanApprovalCurationStrategy,
    SkillCurator,
    SkillProposal,
    list_history,
    list_proposals,
    promote_proposal,
    reject_proposal,
    revert_skill,
)
from .evals import (
    EvalCase,
    EvalCaseResult,
    EvalRun,
    EvalSuite,
    SCORERS,
    contains_scorer,
    evals_dir,
    excludes_scorer,
    list_suites,
    load_suite,
    regex_scorer,
    run_suite,
    run_suite_sync,
    scaffold_suite,
)
from .events import EventLog, EventRecord
from .registry import (
    ARTIFACT_KINDS,
    CachedArtifact,
    cache_artifact,
    cache_path,
    delete_artifact,
    list_artifacts,
    parse_slug,
    pull_artifact,
    resolve_artifact,
    verify_cache,
)
from .errors import (
    AgentHostError,
    AgentNotFoundError,
    AgentSpecError,
    HostConfigError,
    MarkdownParseError,
)
from .gateway import (
    AgentsRoutingRules,
    Gateway,
    build_router,
    load_agents_md,
    parse_agents_md,
)
from .hooks import (
    HookDispatcher,
    HookRegistry,
    HookResult,
    default_hook_resolver,
    matches as hook_matches,
    record_to_jsonl,
    redact_secrets,
)
from .mcp import (
    McpBridge,
    MCPServerConfig,
    MCPToolSpec,
    load_mcp_servers,
    scaffold_mcp_tool,
)
from .scheduler import (
    CronEntry,
    CronFireResult,
    Scheduler,
    default_cron_resolver,
    load_crons,
    parse_expression,
    scaffold_cron,
)
from .layout import AgentPaths, HostPaths, default_root
from .loader import AgentLoader, LoadedAgent
from .markdown import MarkdownDoc, parse_markdown
from .markdown_sync import (
    list_memory_facts,
    list_user_facts,
    reload_agent,
    sync_all,
    sync_memory_facts,
    sync_user_facts,
)
from .rules import (
    build_chat_prompt_template,
    build_system_prompt,
    render_memory_block,
    render_persona_block,
    render_rules_block,
    render_user_block,
)
from .skills import (
    SkillValidationReport,
    build_keyword_skill_strategy,
    scaffold_skill,
    select_skills_for,
    validate_skills,
)

__all__ = [
    "AgentHostError",
    "AgentLoader",
    "AgentNotFoundError",
    "AgentPaths",
    "AgentRouter",
    "AgentSpecError",
    "AgentsRoutingRules",
    "AutoPromoteCurationStrategy",
    "ChatSession",
    "CronEntry",
    "CronFireResult",
    "CurationCtx",
    "CurationOutcome",
    "CurationStrategy",
    "EventLog",
    "EventRecord",
    "HumanApprovalCurationStrategy",
    "SkillCurator",
    "SkillProposal",
    "Gateway",
    "MCPServerConfig",
    "MCPToolSpec",
    "McpBridge",
    "Scheduler",
    "HookDispatcher",
    "HookRegistry",
    "HookResult",
    "HostConfig",
    "HostConfigError",
    "HostPaths",
    "LoadedAgent",
    "MarkdownDoc",
    "MarkdownParseError",
    "ProviderConfig",
    "SkillValidationReport",
    "build_chat_callable",
    "build_chat_prompt_template",
    "build_keyword_skill_strategy",
    "build_router",
    "build_system_prompt",
    "chat_repl",
    "default_cron_resolver",
    "default_hook_resolver",
    "default_root",
    "hook_matches",
    "list_history",
    "list_memory_facts",
    "list_proposals",
    "list_user_facts",
    "load_agents_md",
    "load_crons",
    "load_mcp_servers",
    "parse_agents_md",
    "parse_expression",
    "parse_markdown",
    "promote_proposal",
    "record_to_jsonl",
    "redact_secrets",
    "reject_proposal",
    "reload_agent",
    "revert_skill",
    "render_chat_preview",
    "render_memory_block",
    "render_persona_block",
    "render_rules_block",
    "render_user_block",
    "scaffold_cron",
    "scaffold_mcp_tool",
    "scaffold_skill",
    "select_skills_for",
    "sync_all",
    "sync_memory_facts",
    "sync_user_facts",
    "thread_log_path",
]
