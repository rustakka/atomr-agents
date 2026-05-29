# Hedgehog Upstream Feature Requests — Implementation

This page documents the atomr-agents capabilities added to satisfy the
Hedgehog feature requests in
[`docs/upstream-feature-requests/from-hedgehog.md`](upstream-feature-requests/from-hedgehog.md).
The themes are **record-and-replay determinism**, **governed HITL**,
**typed security context**, and **financial-grade eval**.

All types referenced from `atomr-orgs` are **framework-local
abstractions** owned by atomr-agents (traits/structs that atomr-orgs
implements), so atomr-agents stays self-contained and independently
testable. Durable stores ship a real SQL backend (sqlx over the `any`
driver → SQLite + Postgres) and, where relevant, Redis; SQLite runs
in-process so the SQL paths are covered by runnable tests, while
Postgres/Redis paths run as integration tests gated on `ATOMR_IT_SQL_URL`
/ `REDIS_URL`.

---

## Foundations

### Typed context extensions (FR-6) — `atomr-agents-core`
`CallCtx`/`InvokeCtx` carry a typed [`Extensions`] map:

```rust
let mut ctx = CallCtx::new(agent_id, tokens, time, money, iters, trace);
ctx.insert_ext(clearance);           // host/runtime code only
let c = ctx.ext::<ClearanceContext>(); // typed read at the Tool seam
```
Extensions are `Arc`-backed (cloning `CallCtx` shares them), **never
serialized** (so secrets/clearance never reach a checkpoint, telemetry
record, or prompt) and **not LLM-writable** (no path from `raw_args`).

### Security substrate (FR-6) — `atomr-agents-security` (new crate)
- `ClearanceContext { subject, level: ClearanceLevel, compartments }`,
  `Compartment`, `ClearanceLevel` (ordered, `Mnpi` at top), `NeedToKnow`.
- `Mandate` trait (pre-trade boundary), `AllowAll`/`DenyAll`.
- `ScopedSecret` — use-and-drop credential, redacted `Debug`,
  scrub-on-`Drop`, **not** `Serialize`. `CapabilityHandle` +
  `CapabilityBroker` resolve a handle to a secret only after a clearance
  check (`StaticCapabilityBroker` for tests/single-process hosts).
- `WalledTool<T: Tool>` — reusable information-wall middleware: reads
  `ctx.ext::<ClearanceContext>()` (**fail-closed** → `AccessDenied`),
  runs an optional `Mandate`, then delegates. Wraps any tool without
  touching its constructor.

```rust
let tool = WalledTool::new(order_tool, NeedToKnow::level(ClearanceLevel::Restricted)
        .with_compartment("desk:credit"))
    .with_mandate(Arc::new(position_limit_mandate));
```

### Telemetry backbone (FR-19) — `atomr-agents-observability`
`RunEvent { kind, workflow, run, step, checkpoint_ref, model_pin, tokens, ts }`
with `RunEventKind` (`CheckpointCreated`, `ToolDispatched`, `ToolReturned`,
`InferenceCompleted`, `InterruptRaised/Resolved`, `ModelDrift`,
`BudgetSpent`, `Trigger`). Every event carries a `CheckpointRef`
resolvable through the `Checkpointer`, so an external projector
reconstructs a run's action timeline **without re-inference**.

`Telemetry` fans out to `TelemetrySink`s: `InMemoryTelemetrySink`,
`JsonlTelemetrySink`, `ChannelTelemetrySink` (tokio mpsc → wrap in an
`atomr-streams` Source). `Telemetry::bridge_from(&EventBus)` projects the
existing in-process `Event` taxonomy onto the backbone. The model-drift,
cost, interrupt, and trigger features publish onto this one backbone.

### Durable persistence — `atomr-agents-state`
`SqlCheckpointer` (sqlx `any` → SQLite + Postgres, reusing
`atomr_persistence_sql::SqlConfig`) and `RedisCheckpointer` replace the
former stubs and implement the full `Checkpointer` trait
(save/load/latest/list/fork). `SqliteCheckpointer`/`PostgresCheckpointer`
are dialect-named aliases; the dialect is auto-detected from the URL.
This is the shared pattern reused by the StepRecord, metric-history,
interrupt-registry, and outbox stores.

---

## Determinism

### Recording checkpointer + replay (FR-1) — `state` + `agent`
- `StepRecord { key, inference: Option<InferenceRecord>, tool_calls: Vec<ToolCallRecord>, state_snapshot }`.
  `InferenceRecord` captures provider/model_id/model_version/params_hash/
  prompt_hash/raw_completion/usage; `ToolCallRecord` captures the verbatim
  `ToolReturn` + `is_side_effecting`.
- `StepRecordStore` (in-memory + `SqlStepRecordStore`); `RecordingCheckpointer`
  wraps any `Checkpointer` and adds `record(StepRecord)`.
- `ReplayProvider` implements `InferenceClient`: serves recorded
  completions in order with **zero provider calls**; a missing record is a
  loud typed `ReplayError` (never silent re-inference). `record_turn` /
  `turn_from_record` round-trip a live `TurnResult` to/from a record
  byte-identically.

```rust
let rp = ReplayProvider::load(&store, &workflow, &run).await?;
// Construct an Agent with `inference: Arc::new(rp)` → model side replays.
```

### Replay sink (FR-2) — `atomr-agents-workflow`
`DecisionReplaySink` + `replay_to_sink(records, workflow, run, sink)`
walks recorded steps in `(workflow,run,super_step)` order and emits
side-effecting `ToolCallRecord`s to an external simulator — no model/tool
invocation, resumable via a step cursor. `ChannelReplaySink` bridges to a
reactive stream.

### Model pin + drift (FR-3) — `atomr-agents-agent`
`ModelPin { provider, model_id, model_version, params_hash }`;
`PinnedClient` (an `InferenceClient` middleware) resolves the concrete
model via a `ModelResolver`, refuses to run in `strict` mode without a
pin (`ModelPinViolation`), emits `RunEventKind::ModelDrift` on mismatch,
and can `refuse_on_drift`. `revalidate()` supports heartbeat re-checks of
provider-side silent upgrades. The resolved pin is stamped into every
`InferenceRecord`.

---

## Governed HITL — `atomr-agents-workflow`

### Durable interrupt registry (FR-4)
`InterruptRegistry` (in-memory + SQL): `register`, `list(filter)`,
`claim` (compare-and-set → `AlreadyClaimed`), `resume` (exactly-once),
SLA `tick` → `EscalationPolicy`. `PendingInterrupt` carries a stable id,
requested role/clearance, payload, deadline, assignee, and status; it
survives reload and remains resumable by id.

### Transactional resume / outbox (FR-5)
`resume_with(interrupt_id, resolution, ResumeEffect)` commits the
resolution **and** an `OutboxRecord` together (one transaction / critical
section) before advancing; on failure the interrupt stays `Pending`. An
`OutboxSink` (implemented by hedgehog's Ledger writer) is driven
at-least-once with idempotency keys; `ResumeEffect::Saga` offers
commit/compensate instead.

### Contract-validate operator (FR-15)
`ContractValidate(DataContract { schema, freshness, continuity, custom })`
yields `Valid`/`Breach(ContractViolation{ TypeDrift|StaleData|Gap|Custom })`.
`OnBreach::{Drop, RouteTo, Interrupt}` — `Interrupt` raises a **durable**
FR-4 interrupt carrying the violation payload from inside a pipeline stage.

---

## Financial-grade eval — `atomr-agents-eval`

- **MetricScorer (FR-7)** — `Threshold::{AtLeast,AtMost,Between,Outside}`
  over a numeric metric read from the output. **TimeSeriesRegressionGate**
  — fails when a metric regresses beyond `Tolerance::{Abs,Pct,ZScore}` vs
  a `GoldenSeries`, loaded from a `MetricHistoryStore` (in-memory +
  `SqlMetricHistoryStore`). Both implement the existing `Scorer` trait.
- **CompositeScorer + Ranker (FR-8)** — weighted aggregation
  (`WeightedSum|WeightedMean|Min|Custom`) → `TriageResult { composite,
  breakdown }`; `Ranker` produces stable ranks + percentiles.
- **ProvenanceScorer (FR-9)** — deterministic, non-LLM: verifies each
  cited `Claim` has a backing `EvidenceBundle` in an `EvidenceIndex` whose
  `doc_hash` recomputes from stored content; returns coverage + the
  uncovered/invalid claim list.

---

## Retrieval — `retriever` / `embed` / `memory` / `ingest`

- **VectorStore + Embeddings (FR-10)** — `VectorStore`
  (upsert/query/delete with a `MetadataFilter` honored at query time) with
  an `InMemoryVectorStore`, a `pgvector` backend, and a Redis backend; an
  `Embeddings` trait bridging the existing `Embedder`. `VectorRetriever`
  is constructible from injectable `(VectorStore, Embeddings)`.
- **Retriever filters (FR-11)** — non-LLM `RetrieverFilter::{AsOf,
  Entitlement, And}` applied inside a `FilteredRetriever` wrapper so the
  guarantee holds across Bm25/Vector/MultiQuery/Ensemble/SelfQuery: no
  doc newer than the as-of ceiling and no out-of-compartment doc is ever
  returned (AND-composed with any SelfQuery predicate, never OR). Ingest
  stamps `system_time` + `compartment` metadata.
- **Novelty + incremental ingest (FR-16)** — `NoveltyRetriever::assess`
  (max similarity + merge candidates) and `IncrementalIngest::ingest_if_novel`
  (append-only write + index update; idempotent re-ingest).

---

## Host trigger runtime (FR-12) — `atomr-agents-host`

`crates/host/src/triggers.rs` adds, alongside the existing interval
`Scheduler`:
- `EventTrigger` (`Webhook`/`Stream`/`PubSub` source + filter) →
  `EventTriggerRegistry::fire`;
- `CronTrigger` with real cron fields (via the `cron` crate) and a
  `TradingCalendar` so "EOD on trading days" skips weekends/holidays
  (`next_fire_after`);
- `TriggerControl` — `list_triggers`, `pause`/`resume`/`set_cooldown`;
- `RateLimit { max_fires, per, on_exceed: Drop|Queue|Backpressure }`.
Trigger fires + control actions are observable (bridged to the FR-19
backbone).

---

## Cross-boundary org / personas — `org` / `persona` / `instruction`

- **Org projection (FR-13)** — `OrgProjection::project(&CompiledOrgModel)`
  maps units→departments/teams, roles→routing targets, reporting edges→
  routing, and each `Compartment`→`Policy::narrow` scope + memory
  namespace; emits a `WallSyncReport` (unmapped compartment = fail-closed).
  `verify()` reports drift for CI. `CompiledOrgSource` is the
  framework-local trait atomr-orgs implements.
- **Persona clearance (FR-14)** — persona schema gains
  `clearance`/`compartments`; `bind_persona_to_role(..)` fails closed
  (`ClearanceMismatch`) at host/compose time when a persona declares
  access the Role's `ClearanceContext` does not grant.
- **Debate protocol (FR-17)** — `DebateStrategy { roles, max_rounds }`
  over a `CritiqueChannel` (built on the `AppendMessages` reducer);
  `DissentTermination` ends on convergence or `max_rounds`, surfacing
  `DebateOutcome { converged, unresolved, transcript }`. Replayable via
  the recording checkpointer.

---

## Cost & trust — `agent` / `strategy` / `context` / `tool`

- **Cost/budget (FR-18)** — `CostMeter` (`estimate` pre-flight, `record`
  post-call over a `Pricing` table); `DecisionKey { desk, strategy,
  decision_id }` carried on `CallCtx` extensions; `SpendLedger` attributes
  spend by key (`spend_by`) and emits `RunEventKind::BudgetSpent`;
  `Budget { cap, scope }` + `BudgetExceeded`, wired into the harness via
  `BudgetTermination` to terminate cleanly on overspend.
- **Model attributes (FR-20)** — `ModelRegistry` of `ModelAttributes
  { region, residency, contractual, tags }`; a declarative serializable
  `ModelPredicate` (e.g. `All([RegionIs(Eu), HasTag("mnpi-approved")])`)
  derives `allowed_models` via `ModelRegistry::allowlist`, with auditable
  `evidence`.
- **Content trust (FR-21)** — `Trust::{Trusted,Untrusted}`,
  `TrustedContent`, and a `TrustPolicy` that fences untrusted spans into a
  delimited non-instruction region during prompt assembly; optional
  `InjectionScreen` (`KeywordInjectionScreen`) flags suspicious untrusted
  spans. Taint provenance (untrusted sources consumed) is queryable.
- **Evidence trace (FR-22)** — `EvidenceTrace { inputs: Vec<EvidenceRef>,
  rationale }` carried in the tool-return artifact channel (so it
  auto-persists into `ToolCallRecord`); `ExplainabilityPolicy::Require`
  via `ExplainedTool` middleware rejects a money-moving tool return that
  lacks a trace (`MissingEvidence`).

---

## Testing & feature flags

- Unit tests accompany every FR; SQL backends are exercised against
  in-process `sqlite::memory:`. Postgres/Redis/pgvector integration tests
  run only when `ATOMR_IT_SQL_URL` / `REDIS_URL` are set.
- New feature flags: `atomr-agents-state` `{sqlite, postgres, redis}`,
  `atomr-agents-eval` `{sqlite, postgres}`, `atomr-agents-workflow`
  `{sqlite, postgres}`, `atomr-agents-memory` `{pgvector, redis}`, and the
  umbrella `security` feature (in `full`).

[Extensions]: ../crates/core/src/context.rs
