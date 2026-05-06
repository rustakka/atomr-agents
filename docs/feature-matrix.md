# Feature matrix

Every feature flag in atomr-agents, what it pulls in, and the
canonical "shapes" most consumers reach for.

## The architectural invariant

`atomr-agents` (the umbrella) exposes feature flags that turn each
subsystem on. Crates beyond the *core / strategy / callable / context
/ observability* baseline are optional. Backend integrations (sqlite,
postgres, pgvector, qdrant, chroma, redis) are further-gated within
the relevant subsystem crate.

A `cargo build -p atomr-agents --no-default-features` compiles only
core + the four substrate crates. A `cargo build -p atomr-agents
--all-features` brings in every subsystem.

## Umbrella features

```toml
# crates/umbrella/Cargo.toml
[features]
default     = ["agent", "tool", "skill", "memory", "persona", "instruction"]
tool        = ["dep:atomr-agents-tool"]
skill       = ["dep:atomr-agents-skill", "tool"]
memory      = ["dep:atomr-agents-memory"]
embed       = ["dep:atomr-agents-embed", "memory", "tool"]
persona     = ["dep:atomr-agents-persona"]
instruction = ["dep:atomr-agents-instruction", "persona"]
agent       = ["dep:atomr-agents-agent", "tool", "skill", "memory", "persona", "instruction"]
workflow    = ["dep:atomr-agents-workflow", "tool"]
harness     = ["dep:atomr-agents-harness", "agent", "workflow"]
org         = ["dep:atomr-agents-org", "tool"]
registry    = ["dep:atomr-agents-registry", "harness"]
eval        = ["dep:atomr-agents-eval", "harness"]
testkit     = ["dep:atomr-agents-testkit", "agent", "harness"]
# Provider back-ends — forwarded to atomr-agents-agent's per-provider features.
provider-anthropic = ["agent", "atomr-agents-agent/provider-anthropic"]
provider-openai    = ["agent", "atomr-agents-agent/provider-openai"]
provider-gemini    = ["agent", "atomr-agents-agent/provider-gemini"]
full        = ["agent", "workflow", "harness", "org", "registry", "eval", "embed", "testkit"]
```

| Feature | Pulls in | Use when |
|---|---|---|
| `default` | `agent` + sub-deps | most consumers — building one or more agents |
| `agent` | agent + tool + skill + memory + persona + instruction | minimal agent runtime |
| `workflow` | workflow + tool | DAG execution; you don't need agents |
| `harness` | harness + agent + workflow | long-running, persistent loops |
| `org` | org + tool | multi-agent topologies |
| `registry` | registry + harness | versioned artifact publishing |
| `eval` | eval + harness | quality gates and CI eval suites |
| `embed` | embed + memory + tool | RAG over `LongStore` |
| `testkit` | testkit + agent + harness | depend on `atomr-infer-testkit` directly for `MockRunner`; the testkit crate itself is currently a stub |
| `provider-anthropic` | agent + `atomr-infer-runtime-anthropic` re-exported under `agent::providers::anthropic` | wire the Anthropic Messages API as a `ModelRunner` without touching atomr-infer directly |
| `provider-openai` | agent + `atomr-infer-runtime-openai` re-exported under `agent::providers::openai` | wire OpenAI Chat Completions / Anthropic-compatible providers |
| `provider-gemini` | agent + `atomr-infer-runtime-gemini` re-exported under `agent::providers::gemini` | wire Google Gemini (Vertex AI / AI Studio) |
| `full` | every above | demos, evaluations, "I want everything" |

## Subsystem-level feature flags

These live in individual crate `Cargo.toml`s — flip them in addition
to the umbrella feature.

### `atomr-agents-tool`

```toml
[features]
default     = ["openai", "anthropic"]
openai      = []   # OpenAI-style tool_call_delta parser arms
anthropic   = []   # Anthropic content_block_delta parser arms
gemini      = []   # placeholder — Gemini provider parsing
```

### `atomr-agents-state` (checkpointer backends)

```toml
[features]
default  = []    # InMemoryCheckpointer always available
sqlite   = []    # SqliteCheckpointer stub — wire to sqlx in deployment
postgres = []    # PostgresCheckpointer stub
```

### `atomr-agents-memory` (long-store backends)

```toml
[features]
default  = []    # InMemoryLongStore always available
pgvector = []    # PgvectorStore stub
qdrant   = []    # QdrantStore stub
chroma   = []    # ChromaStore stub
```

### `atomr-agents-cache` (LLM cache backends)

```toml
[features]
default = []     # InMemoryLlmCache + SemanticLlmCache always available
sqlite  = []     # SqliteLlmCache stub
redis   = []     # RedisLlmCache stub
```

### `atomr-agents-ingest`

```toml
[features]
default = []
pdf     = []     # placeholder — PDF loader (pdf-extract)
html    = []     # placeholder — HTML loader (scraper)
web     = []     # placeholder — Web loader (reqwest)
```

## Canonical shapes

### Shape A: Minimal agent (default)

```toml
[dependencies]
atomr-agents = "0.2"
atomr-infer  = { version = "0.6", features = ["openai"] }
```

Pulls: agent + tool + skill + memory + persona + instruction.
Suitable for: any single-agent flow.

### Shape B: RAG agent

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["agent", "embed"] }
atomr-agents-retriever = "0.2"
atomr-agents-ingest    = "0.2"
atomr-infer = { version = "0.6", features = ["openai"] }
```

Pulls: agent + retriever zoo + ingestion. Ship a `VectorRetriever`
backed by `LongStore` plus `RecallMemoryTool` exposed to the agent.

### Shape C: Production harness with eval gating

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["harness", "registry", "eval"] }
atomr-agents-state = { version = "0.2", features = ["postgres"] }
atomr-infer = { version = "0.6", features = ["openai", "anthropic"] }
```

Pulls: harness + registry + eval + Postgres-backed checkpoints.
Wire `Registry::publish_gated` to your CI to block harness publishes
on regression.

### Shape D: Multi-agent topology

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["agent", "org", "workflow"] }
atomr-infer  = { version = "0.6", features = ["openai", "anthropic"] }
```

Pulls: agent + org + workflow. Build supervisor / swarm / network /
hierarchical patterns; route through `CapabilityMatchRouter` or the
`swarm_loop` helper.

### Shape E: Pluggable provider backend (no direct atomr-infer dep)

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["agent", "provider-anthropic"] }
# add features = ["provider-openai"] / ["provider-gemini"] as needed
```

Pulls: agent + the chosen provider runtime. Construct an agent against
`atomr_agents::agent::providers::anthropic::AnthropicRunner::new(
AnthropicConfig::from_env())`, wrap it in `LocalRunnerClient`, and skip
the explicit `atomr-infer` dep. Multiple provider features can be
enabled at once for fallback / multi-provider scenarios.

### Shape F: Full kitchen-sink (demos, eval rigs)

```toml
[dependencies]
atomr-agents = { version = "0.2", features = ["full"] }
atomr-agents-retriever = "0.2"
atomr-agents-ingest    = "0.2"
atomr-agents-parser    = "0.2"
atomr-agents-cache     = { version = "0.2", features = ["sqlite"] }
atomr-agents-state     = { version = "0.2", features = ["sqlite"] }
atomr-agents-memory    = { version = "0.2", features = ["pgvector"] }
atomr-infer            = { version = "0.6", features = ["all-runtimes"] }
```

Pulls: every subsystem + sqlite-backed checkpointing/cache +
pgvector-backed long-term store. Suitable for end-to-end demos and
evaluation rigs.

## Where to go from here

- [Architecture](architecture.md) — what each subsystem does.
- [README](../README.md) — quick start and crate inventory.
