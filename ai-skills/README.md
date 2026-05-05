# ai-skills/

Skills for AI coding assistants working on **projects that depend on
atomr-agents** — not for editing atomr-agents itself. They follow the
standard `SKILL.md` + frontmatter convention used by Claude Code,
Claude Agent SDK, and other agentic tools.

These skills are deliberately separate from the repo's own dev
tooling so that distributing them to consumers does not entangle
atomr-agents' internal development workflow.

## What's here

| Skill | Use when… |
|---|---|
| `atomr-agents-quickstart` | Standing up the first agent — picking feature flags, wiring `Pipeline`, building an `Agent`, running against `MockRunner` |
| `atomr-agents-pipeline` | Composing `Callable`s with `Pipeline` + decorators (`with_retry`, `with_fallbacks`, `with_config`, `with_timeout`, `Branch`) |
| `atomr-agents-state` | Channelled state — picking reducers, declaring `StateSchema`, persisting via `Checkpointer`, fork-with-edit |
| `atomr-agents-hitl` | Human-in-the-loop — `interrupt()`, static breakpoints, `Command::{Continue, Resume, Update, Goto}` resume |
| `atomr-agents-rag` | Building a retriever pipeline — picking from the zoo, ingesting docs, writing to `LongStore` |
| `atomr-agents-tools` | Authoring tools — `Tool` vs `RichTool`, `ToolReturn`, parallel dispatch semantics, `HandoffTool`, memory tools |
| `atomr-agents-middleware` | Wrapping an agent's per-turn pipeline with `AgentMiddleware` hooks |
| `atomr-agents-multi-agent` | Org / team / department topologies, the four routing patterns, `swarm_loop`, `HandoffTool` |
| `atomr-agents-eval` | Eval suites, judge / pairwise / rubric scorers, `RegressionGate`, `AnnotationQueue` |
| `atomr-agents-observability` | `EventBus`, `RunTreeBuilder`, tracers (`Stdout`, `Jsonl`, `LangSmith`) |
| `atomr-agents-troubleshooting` | Debugging atomr-agents-flavored errors — `BudgetExceeded`, `PolicyDenied`, parser failures, retry exhaustion, channel mismatches |
| `atomr-agents-langgraph-migration` | Mapping LangChain / LangGraph idioms onto atomr-agents — concept table + concrete code translations |

Each `SKILL.md` is a thin router: it points at canonical docs in this
repo (`docs/*.md`, `examples/*`) and at the relevant crate's API. It
deliberately does **not** restate API surfaces that belong in
rustdoc, because those drift faster than docs.

## Installing

Pick the path that matches your assistant. The skills themselves are
vendor-neutral `SKILL.md` files — only the install mechanism differs.

### Claude Code (recommended: marketplace)

If you use Claude Code, install via the plugin marketplace — this
keeps the skills updated as atomr-agents releases, with no manual
copy step:

```text
/plugin marketplace add rustakka/atomr-agents
/plugin install atomr-agents-ai-skills@atomr-agents
```

You can also install from a local checkout (useful when developing
against an atomr-agents fork):

```text
/plugin marketplace add /path/to/atomr-agents
/plugin install atomr-agents-ai-skills@atomr-agents
```

Skills auto-activate based on the `description` frontmatter — no
need to invoke them explicitly.

### Claude Agent SDK / project-local `.claude/skills/`

For SDK-based agents or project-local Claude Code setups that read
from `.claude/skills/`, copy or symlink the skills in:

```bash
# copy (snapshot)
cp -r ai-skills/skills/* .claude/skills/

# symlink (track upstream)
ln -s "$(pwd)/ai-skills/skills/"* .claude/skills/
```

## Stylistic conventions

These skills follow atomr's:

1. **`SKILL.md` + frontmatter** — `name`, `description`. The
   `description` is what triggers auto-activation, so it's specific
   about *when* to invoke.
2. **Mental model first.** Each skill opens with a one-paragraph
   mental model of the subsystem before diving into API.
3. **Working code blocks.** Snippets compile against the published
   crate version; copy-paste is the intended use.
4. **Pointer to canonical docs.** Every skill ends with a "Canonical
   references" list — paths to `docs/*.md`, `examples/*`, and the
   relevant crate.
5. **"Common mistakes" coda.** Failure modes the framework's
   architecture makes possible (channel-write to wrong key, missing
   reducer, mid-`handle` `ask`, etc.).

## Authoring a new skill

```text
ai-skills/skills/atomr-agents-<topic>/
└── SKILL.md
```

`SKILL.md` frontmatter must include:

```yaml
---
name: atomr-agents-<topic>
description: Use when … . Triggers on … .
---
```

Keep skills focused. If you find yourself documenting two unrelated
subsystems, split them. The `atomr-agents-troubleshooting` skill is
the only deliberately multi-topic one — its job is "given an error,
where do I look?".
