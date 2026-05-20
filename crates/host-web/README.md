# atomr-agents-host-web

Axum + embedded React SPA companion for the [`atomr-agents-host`](../host)
runtime — a unified dashboard over the host's full concept surface (agents,
SOUL/MEMORY/RULES/USER docs, skills, curator proposals, crons, hooks,
channels/routing, branches, registry, evals, MCP servers, config) plus a live
event stream.

See [`docs/agent-host/concepts.md`](../../docs/agent-host/concepts.md) for the
unified concept system this crate is organized around.

## Run it

```bash
# Embed the prebuilt React SPA (build it first: cd ui && npm install && npm run build).
cargo run -p atomr-agents-host-web --features embed-ui
# default bind: 127.0.0.1:7400
```

Open `http://127.0.0.1:7400/` to see the SPA. Point it at a host root other than
the default (`$ATOMR_HOST_ROOT` or `~/.atomr/host`) with the `ATOMR_HOST_ROOT`
env var.

The `embed-ui` feature is opt-in: without it the server still serves the full
REST + SSE API and returns a JSON hint in place of the SPA, so the crate builds
and publishes without a prebuilt `ui/dist`.

## Routes

| Method        | Path                                       | Description                                  |
| ------------- | ------------------------------------------ | -------------------------------------------- |
| GET           | `/healthz`                                 | Liveness probe                               |
| GET           | `/api/concepts`                            | Unified concept catalog                      |
| GET           | `/api/agents`                              | Agent summaries                              |
| GET           | `/api/agents/:id`                          | Full agent detail                            |
| POST · DELETE | `/api/agents/:id/{spawn,reload}` · `:id`   | Spawn / reload / stop an agent               |
| POST          | `/api/agents/:id/chat`                     | Deterministic chat preview (no live LLM)     |
| GET · PUT     | `/api/agents/:id/docs/:doc`                | Read / write SOUL·RULES·MEMORY·USER + reload |
| GET · POST    | `/api/agents/:id/skills`                   | List / scaffold skills                       |
| PUT · DELETE  | `/api/agents/:id/skills/:sid`              | Write / delete a SKILL.md                    |
| GET           | `/api/agents/:id/skills/validate`          | Validate skills                              |
| GET · POST    | `/api/agents/:id/curator/proposals[...]`   | List / approve / reject curator proposals    |
| GET · POST    | `/api/agents/:id/curator/{history,revert}` | Skill version history / revert               |
| GET           | `/api/agents/:id/hooks`                    | Hook definitions                             |
| GET · POST    | `/api/agents/:id/branches`                 | List / fork branches                         |
| GET · POST · DELETE | `/api/agents/:id/branches[...]`      | Diff / switch / delete branches              |
| GET · POST · DELETE | `/api/crons`                         | List / create / delete crons                 |
| GET           | `/api/routes` · `/api/channels`            | Routing rules / channel files                |
| GET · DELETE  | `/api/registry`                            | List / delete cached artifacts               |
| GET · POST    | `/api/evals`                               | List / load / run eval suites                |
| GET · POST    | `/api/mcp`                                 | List / scaffold MCP servers                  |
| GET · PUT     | `/api/config`                              | Read / write `config.yaml`                   |
| GET           | `/api/events`                              | Recent event records                         |
| GET           | `/api/events/stream`                       | SSE poll-tail of `events.jsonl`              |

Run on port 7400 (next slot after coding-cli's 7300).
