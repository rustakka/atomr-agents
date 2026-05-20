# Unified concept system

The host borrows vocabulary from three pieces of prior art — **OpenClaw**,
**Hermes Agent**, and **[AionUi](https://github.com/iOfficeAI/AionUi)** (which
manages many backend CLIs from one dashboard, and lists OpenClaw + Hermes among
its backends). Rather than inventing a parallel runtime, the host maps every
borrowed idea onto an *existing* atomr-agents-host primitive.

This page is the single source of truth that the web API
(`crates/host-web`), its REST routes, and the SPA sidebar are all organized
around. The same catalog is served programmatically at
[`GET /api/concepts`](#web-companion) and mirrored in TypeScript at
`crates/host-web/ui/src/lib/concepts.ts`.

## Concept catalog

| Concept | Host primitive | Borrowed from | API resource | UI section |
|---------|----------------|---------------|--------------|------------|
| **Agent** | `AgentSpec` + `AgentHostActor` | AionUi Assistants/Agents | `/api/agents` | Agents |
| **Identity (SOUL)** | `SOUL.md` → `Persona` | OpenClaw / Hermes SOUL | `/api/agents/:id/docs/soul` | Agent ▸ Identity |
| **Rules** | `RULES.md` → instruction prefix | OpenClaw / Hermes RULES | `/api/agents/:id/docs/rules` | Agent ▸ Rules |
| **Memory** | `MEMORY.md` → memory facts | OpenClaw / Hermes MEMORY | `/api/agents/:id/docs/memory` | Agent ▸ Memory |
| **User profile** | `USER.md` → user profile | OpenClaw / Hermes USER | `/api/agents/:id/docs/user` | Agent ▸ User |
| **Skill** | `SKILL.md` → `Skill` / `SkillSet` | AionUi Skills / Hermes auto-curation | `/api/agents/:id/skills` | Agent ▸ Skills |
| **Curator** | `SkillProposal` + `CurationStrategy` | Hermes auto-curation | `/api/agents/:id/curator/proposals` | Agent ▸ Skills |
| **Hook** | `HookDefinition` + `HookDispatcher` | Claude Code hooks | `/api/agents/:id/hooks` | Agent ▸ Hooks |
| **Cron** | `CronEntry` + `Scheduler` | AionUi Scheduled Tasks / Hermes heartbeat | `/api/crons` | Crons |
| **Channels & Routing** | `AGENTS.md` → `AgentRouter` / `Gateway` | OpenClaw gateway / Hermes channels | `/api/routes` | Channels & Routing |
| **Branch / Checkpoint** | `Checkpoint` + branch ops | Claude Code checkpoints | `/api/agents/:id/branches` | Agent ▸ Branches |
| **Registry artifact** | `CachedArtifact` | atomr registry | `/api/registry` | Registry |
| **Eval suite** | `EvalSuite` + `run_suite` | atomr eval | `/api/evals` | Agent ▸ Evals |
| **MCP server** | `MCPServerConfig` + `McpBridge` | AionUi MCP integration | `/api/mcp` | MCP |
| **Event** | `EventRecord` + `EventLog` | Claude Code event log | `/api/events` | Events |
| **Config** | `HostConfig` (`config.yaml`) | — | `/api/config` | Settings |

## AionUi parallels

AionUi is a unified desktop UI over many backend CLIs. The host's web companion
plays the analogous role for a single host root, so the nouns line up directly:

| AionUi | Host |
|--------|------|
| Assistants / Agents | **Agents** (one `agents/<id>/` directory each) |
| Conversations | **Chat** preview pane (deterministic preview — see below) |
| Scheduled Tasks | **Crons** |
| Skills | **Skills** (+ auto-curated proposals) |
| MCP integration | **MCP servers** |

## Web companion

`crates/host-web` (`atomr-agents-host-web`) is an Axum server that exposes the
full surface above as REST + a live SSE event stream, and embeds a React SPA.
It mirrors the sibling `*-harness-web` crates (default bind `127.0.0.1:7400`).

```sh
# scaffold a host root if you don't have one, then:
cargo run -p atomr-agents-host-web --features embed-ui
# open http://127.0.0.1:7400
```

Run against an arbitrary root with `ATOMR_HOST_ROOT=/path/to/host`.

### Surface notes

- **Mutations write to disk and hot-reload.** Editing a SOUL/RULES/MEMORY/USER
  doc or a SKILL.md `PUT`s the new content, writes the file, and — if the agent
  is currently running — calls `runtime.reload(id)` so the change takes effect
  without a restart.
- **Events are append-only.** Every mutation appends an `EventRecord` to
  `events.jsonl`. `GET /api/events?limit=N` reads the tail; `GET
  /api/events/stream` is a Server-Sent-Events poll-tail of the same file
  (emitted as named `host_event` messages).
- **Chat is preview-only.** The `/api/agents/:id/chat` endpoint returns a
  deterministic `render_chat_preview` reply, not a live LLM turn. The UI labels
  this clearly.
- **HTTP status mapping.** Missing resources → `404`; invalid user-supplied
  content (bad markdown/skill/spec/config) → `400`; state/precondition refusals
  (e.g. *refusing to delete `main` without force*, *no checkpoint to fork from*)
  → `409`; infrastructure faults → `500`.

### Endpoint map

| Method + path | Backing host function |
|---------------|-----------------------|
| `GET /api/concepts` | static `concept_catalog()` |
| `GET /api/agents` | `HostPaths::list_agent_ids` + `AgentLoader::load` |
| `GET /api/agents/:id` | `AgentLoader::load` → `AgentDetail` |
| `POST /api/agents/:id/spawn` · `reload` · `DELETE` | `HostRuntime::{spawn_agent,reload,stop_agent}` |
| `POST /api/agents/:id/chat` | `AgentHandle::preview` |
| `GET·PUT /api/agents/:id/docs/:doc` | `MarkdownDoc::read` / `MarkdownDoc::write` + reload |
| `GET·POST /api/agents/:id/skills` | `AgentDefinition.skills` / `scaffold_skill` |
| `PUT·DELETE /api/agents/:id/skills/:sid` | `write_skill` / `delete_skill` |
| `GET /api/agents/:id/skills/validate` | `validate_skills` |
| `GET /api/agents/:id/curator/proposals` | `curator::list_proposals` |
| `POST .../proposals/:sid/approve` · `reject` | `promote_proposal` / `reject_proposal` |
| `GET .../curator/history/:sid` · `POST .../revert/:sid` | `list_history` / `revert_skill` |
| `GET /api/agents/:id/hooks` | `AgentDefinition.hooks` |
| `GET·POST /api/agents/:id/branches` | `list_branches` / `fork_branch` |
| `GET .../branches/diff` · `POST .../:b/switch` · `DELETE .../:b` | `diff_branches` / `switch_branch` / `delete_branch` |
| `GET·POST /api/crons` · `DELETE /api/crons/:id` | `load_crons` / `scaffold_cron` (+ `parse_expression`) / file delete |
| `GET /api/routes` · `GET /api/channels` | `gateway::load_agents_md` / `channels_dir` listing |
| `GET /api/registry` · `DELETE /api/registry/:kind/:id/:version` | `list_artifacts` / `delete_artifact` |
| `GET /api/evals` · `GET /api/evals/:id` · `POST /api/evals/:id/run` | `list_suites` / `load_suite` / `run_suite_sync` |
| `GET·POST /api/mcp` | `load_mcp_servers` / `scaffold_mcp_server` |
| `GET·PUT /api/config` | `HostConfig::to_yaml_string` / validate + write |
| `GET /api/events` · `GET /api/events/stream` | `EventLog::read_all` / SSE poll-tail |
