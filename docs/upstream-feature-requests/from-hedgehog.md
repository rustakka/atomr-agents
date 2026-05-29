# Upstream Feature Requests — rustakka/atomr-agents

> Filed by the Hedgehog agentic-hedge-fund project. hedgehog is a fully-agentic, regulated, real-money hedge fund. atomr-agents is the L3 "workforce" substrate: every Role in the firm (atomr-orgs) is filled by an Agent<Instruction,Tool,Memory,Skill>, agents run as Pipelines/multi-agent patterns over the atomr actor runtime, HITL interrupts gate every money-moving action, the eval/regression machinery grades strategies, the retriever zoo backs the research/data plane, and the trigger harness fires opportunity-discovery and risk loops. hedgehog therefore depends on atomr-agents for (a) deterministic, audit-grade reproduction of non-deterministic LLM runs; (b) durable, governed, queryable HITL approval that bridges transactionally to the atomr-orgs Decision/Ledger system-of-record; (c) substrate-level information-wall and credential enforcement carried through the call/tool context rather than the prompt; (d) numeric/time-series and provenance-aware eval scorers for grading backtests and theses; (e) point-in-time, entitlement-filtered retrieval with a production vector backend; (f) a richer host trigger runtime (calendar cron + event/webhook + runtime control); and (g) cost/value accounting. The requests below consolidate 31 surfaced gaps into 23 independently-actionable feature requests. The dominant themes are RECORD-AND-REPLAY DETERMINISM, GOVERNED HITL, TYPED SECURITY CONTEXT, and FINANCIAL-GRADE EVAL — all required for books-and-records defensibility and trading integrity. Several requests cross the atomr-orgs boundary (org projection, persona clearance, ledger bridge) and should be coordinated with that repo.

> **Context:** Hedgehog ([implementation plan](../../docs/IMPLEMENTATION_PLAN.md)) is a fully-agentic, regulated, real-money hedge fund built on atomr-agents as its L3 "workforce" layer. Each FR below has a hedgehog-side fallback, so none is a hard blocker.

---

## FR-1: Recording Checkpointer + swappable ReplayProvider (provenance capture + side-effect-free replay)  — **P0**

**Motivation (hedge-fund use case).** Live LLM agents are non-deterministic, but hedgehog must reproduce EXACTLY what a money-moving agent did, for compliance/audit, without re-inference and without re-firing side-effecting broker tools. The agreed determinism rule is record-and-replay via per-step checkpoints, not re-inference.

**Current behavior / gap.** The base Checkpointer persists channelled State snapshots keyed by (workflow,run,step) but does not record the raw model completion or the verbatim ToolReturn. There is no documented replay mode that returns recorded outputs instead of invoking the provider / re-executing tools. Replaying a run today re-calls the model (non-deterministic) and would re-fire side-effecting tools (e.g. broker orders).

**Proposed API / behavior.**
```rust
Extend Checkpointer to a RecordingCheckpointer that, per (workflow,run,step), persists a StepRecord { inference: InferenceRecord { provider, model_id, model_version, params_hash, prompt_hash, raw_completion, usage:{prompt_tokens,completion_tokens} }, tool_calls: Vec<ToolCallRecord { tool, args_hash, return: ToolReturn (verbatim Content/ContentAndArtifact/Command), is_side_effecting:bool }>, state_snapshot }. Add a `ReplayProvider` trait implementing the model Callable that, given (workflow,run,step), returns the recorded raw_completion instead of calling inference; and a `ReplayToolStrategy` that returns recorded ToolReturns for is_side_effecting tools and (optionally) re-executes pure tools. AgentBuilder gains `.replay_from(checkpointer, RunRef)` to construct a run whose model + side-effecting tools are sourced from records. Replay MUST fail loudly if a requested (run,step) record is missing rather than falling back to live inference.
```

**Acceptance criteria.**
- A completed live run can be re-executed in replay mode producing byte-identical State transitions with zero provider calls.
- Tools flagged is_side_effecting are never invoked during replay; their recorded ToolReturn is served instead.
- Missing/incomplete records cause a typed ReplayError, never silent re-inference.
- InferenceRecord captures provider, model_id, model_version, params_hash and prompt_hash for every step.
- Records persist via existing atomr-persistence backends (sql/redis) and survive process restart.

**Fallback if declined.** hedgehog hand-rolls a wrapping Checkpointer + interceptor Callable in hedgehog-fleet that snapshots completions and tool returns to Postgres, and a custom replay model adapter — duplicative, error-prone, and likely to diverge from upstream checkpoint keying.

---

## FR-2: Deterministic replay sink: feed recorded decisions to an external simulator without re-inference  — **P1**

**Motivation (hedge-fund use case).** hedgehog regression-tests a strategy by replaying a recorded LIVE run against a deterministic SimulatedVenue (DES twin) to confirm behavior reproduces and to A/B a new model against historical decisions. This needs recorded tool outputs/decisions routed to an external simulator, not back to the live broker tool.

**Current behavior / gap.** ToolReturn::Command exists, and checkpoints exist, but there is no documented sink that streams a run's recorded decisions/tool-returns to an external deterministic consumer in order, decoupled from the live tool implementation. Replay (FR above) reconstructs the agent; this FR is the outbound contract to drive a non-agent simulator from those records.

**Proposed API / behavior.**
```rust
Add a `DecisionReplaySink` trait: `fn emit(&self, step: StepRef, decision: &ToolCallRecord) -> Result<()>` and a runner `replay_to_sink(checkpointer, RunRef, sink)` that walks recorded steps in (workflow,run,step) order and emits each side-effecting decision (e.g. an order Command) to the sink, applying recorded timestamps for ordering. Provide a `ChannelSink` (atomr-streams Source adapter) so hedgehog's SimulatedVenue can subscribe as a reactive stream. Sink emission is at-least-once with a monotonic step cursor so an interrupted replay resumes deterministically.
```

**Acceptance criteria.**
- Recorded order-Commands from a live run can be replayed in original order into an external sink with no model or tool invocation.
- Replay-to-sink exposes a resumable cursor keyed by (workflow,run,step).
- An atomr-streams Source adapter is provided so a downstream simulator consumes decisions with backpressure.
- Ordering uses recorded step timestamps and is stable across reruns.

**Fallback if declined.** hedgehog reads the recording tables directly and writes its own ordered iterator into the DES twin, coupling hedgehog-backtest to atomr-agents' internal checkpoint schema.

---

## FR-3: Enforced model/provider version pinning as run metadata + 'model changed' drift event  — **P0**

**Motivation (hedge-fund use case).** The architecture treats a model change as a GATE-GOVERNED promotion event; the org transition ladder carries a model_pin_drift RollbackTriggerSpec. Without enforced pins and a drift signal, a silently upgraded model changes live trading behavior un-gated and breaks regulatory traceability ('which model produced this trade?').

**Current behavior / gap.** allowed_models is a Vec<String> and Provider is an enum; pinning is convention. There is no first-class (provider, model, version, params) pin that the runtime refuses to execute without, and no event emitted when the resolved model differs from the pin.

**Proposed API / behavior.**
```rust
Add `ModelPin { provider, model_id, model_version: SemVerOrDigest, params_hash }` settable on AgentBuilder via `.pin_model(ModelPin)`. The model Callable resolves the concrete model+version at call time, compares to the pin, and: refuses to run (typed `ModelPinViolation`) if no pin is set in `strict_pin` mode; emits a `ModelDriftEvent { run, step, expected:ModelPin, actual:ResolvedModel, kind: VersionDrift|ParamsDrift }` onto the telemetry topic (see telemetry FR) whenever actual != expected. The resolved ModelPin is stamped into every StepRecord/InferenceRecord (see recording FR). Provide `provider.resolve_version()` so a deployed pin can be re-validated on a heartbeat to detect provider-side silent upgrades.
```

**Acceptance criteria.**
- In strict_pin mode a run without a ModelPin fails to start with a typed error.
- A resolved model/version/params differing from the pin emits a structured ModelDriftEvent consumable by an external projector.
- Every checkpoint/StepRecord carries the resolved ModelPin.
- A periodic re-resolve can detect a provider-side version change for an unchanged pin string.

**Fallback if declined.** hedgehog wraps every model Callable with a pin-checking middleware and maintains its own version table — works but cannot stamp the pin into upstream checkpoints, so audit linkage is lost.

---

## FR-4: Durable, queryable, fleet-wide HITL interrupt registry / governed approval queue  — **P0**

**Motivation (hedge-fund use case).** Every money-moving action parks on a HITL approval. hedgehog needs a single inbox listing all pending approvals addressed to a Role across the whole fleet, each resumable by a stable id, surviving node failover/rebalance of a money-bearing StrategyActor, with assignment, SLA/timeout, and escalation. Losing a parked approval could drop or duplicate a trade.

**Current behavior / gap.** Interrupts/breakpoints can pause a run, but the brief documents no fleet-wide query of pending interrupts, no guaranteed durability across entity-actor migration, no stable interrupt_id, and no assignment/SLA/escalation. Each agent would otherwise maintain its own pending-approval projection.

**Proposed API / behavior.**
```rust
Add a durable `InterruptRegistry` backed by atomr-persistence: when a run interrupts it writes a `PendingInterrupt { interrupt_id (stable ULID), workflow, run, step, requested_role, requested_clearance, payload, created_at, deadline, assignee:Option<Person>, status: Pending|Claimed|Resolved|Expired|Escalated }`. API: `registry.list(filter: { role?, status?, workflow?, deadline_before? }) -> Vec<PendingInterrupt>`, `registry.claim(interrupt_id, person)`, `registry.resume(interrupt_id, Resolution)`, with SLA timers that move Pending->Escalated per an `EscalationPolicy { after: Duration, to: Role }`. The interrupt_id is the same token regardless of which node currently hosts the StrategyActor; on rebalance the parked run rehydrates from the journal and remains resumable by id. Emit registry transitions onto the telemetry topic.
```

**Acceptance criteria.**
- A pending interrupt survives a kill+restart and entity-actor rebalance and is still resumable by its interrupt_id.
- list() returns all pending interrupts across all runs filterable by requested_role.
- resume(interrupt_id, resolution) delivers the human input to the correct parked run exactly once.
- SLA expiry triggers EscalationPolicy and a registry transition event.
- Two concurrent claims of the same interrupt_id are arbitrated (one succeeds, one gets AlreadyClaimed).

**Fallback if declined.** hedgehog builds a PendingApprovalActor + Postgres table and has every agent emit/maintain its own projection — high duplication, fragile across rebalance, and detached from the framework's resume mechanism.

---

## FR-5: Transactional HITL resume -> external system-of-record commit (saga / transactional outbox)  — **P0**

**Motivation (hedge-fund use case).** Promotion (paper->live) and order approval must be ATOMIC across atomr-agents and the atomr-orgs Ledger: either the Ledger records the approval AND the venue swaps/order releases, or neither. A torn promotion (Ledger says live, venue still paper, or vice versa) is a compliance and trading-integrity failure.

**Current behavior / gap.** Resuming an interrupt delivers the human decision back into the run, but there is no documented contract binding the resume to an external write so that both commit or both roll back. The bridge to the atomr-orgs Decision+Gate+Ledger is unspecified.

**Proposed API / behavior.**
```rust
Make resume accept a transactional effect: `registry.resume_with(interrupt_id, resolution, effect: ResumeEffect)` where `ResumeEffect` is either (a) a transactional-outbox record persisted in the SAME atomr-persistence transaction as the interrupt resolution and the run-state advance (so an external relay reliably forwards it to the Ledger), or (b) a `SagaStep` with `commit`/`compensate` callables that the framework drives with at-least-once delivery and idempotency keys. Provide an `OutboxSink` consumable by hedgehog's Ledger writer. Guarantee: the run does NOT advance past the interrupt unless the resolution + outbox record are durably committed together; on commit failure the interrupt stays Pending.
```

**Acceptance criteria.**
- Interrupt resolution, run-state advance, and outbox record commit in a single persistence transaction (or a saga with compensation).
- Crash between resolution and external commit leaves a recoverable outbox record; the relay completes the Ledger write on restart.
- External commit carries an idempotency key so retries do not double-promote/double-release.
- If the external commit is refused, the run remains parked and no venue swap occurs.

**Fallback if declined.** hedgehog wraps resume in an application-level 2-phase protocol with its own outbox table; doable but the framework advancing the run independently of the external write is the exact torn-state hazard we must avoid.

---

## FR-6: Typed context extensions + capability/secrets broker + reusable walled-tool middleware  — **P0**

**Motivation (hedge-fund use case).** hedgehog's core guarantee is that agents NEVER hold credentials and every order is checked against a mandate Boundary at the Tool seam, with information walls enforced at the substrate, not the prompt. This requires the caller's atomr-orgs ClearanceContext + mandate to reach Tool::invoke through a typed, tamper-evident path, and credentials to be resolved by capability handle the LLM never sees.

**Current behavior / gap.** CallCtx (core/context.rs) carries only budgets + trace. InvokeCtx carries budgets, tool_call_id, raw_args — no caller identity, no ClearanceContext, no capability/secret handle. There is no built-in secrets vault or capability broker. Clearance must be smuggled via trace strings/globals or baked into each tool's constructor, preventing a generic walled-tool middleware.

**Proposed API / behavior.**
```rust
Add a typed extensions map to CallCtx and propagate it into InvokeCtx: `ctx.insert_ext(value)` / `ctx.ext::<T>() -> Option<&T>`, so hedgehog attaches a ClearanceContext + Mandate boundary that flows unmodified to Tool::invoke. Add a `CapabilityBroker` trait: `broker.resolve(handle: CapabilityHandle, ctx) -> Result<ScopedSecret>` where ScopedSecret is a use-and-drop credential never serialized into prompts/checkpoints, and resolution is denied if ctx clearance fails need-to-know. Ship a generic `WalledTool<T>` middleware that, given a NeedToKnow spec, validates `ctx.ext::<ClearanceContext>()` (fail-closed) and a mandate pre-trade check before delegating to the inner Tool. Extensions are excluded from checkpoint serialization by default (secrets never persisted).
```

**Acceptance criteria.**
- A ClearanceContext attached to CallCtx is readable by typed accessor inside Tool::invoke without string parsing.
- WalledTool middleware denies invocation (typed AccessDenied) when ctx clearance lacks the required compartments/level.
- CapabilityBroker resolves a scoped secret only when clearance passes; the secret value never appears in any checkpoint, telemetry, or prompt.
- Context extensions are tamper-evident (cannot be set from within an LLM tool argument) and not LLM-writable.
- The same WalledTool wraps any Tool without modifying the inner tool's constructor.

**Fallback if declined.** hedgehog bakes clearance+mandate into every tool constructor (WalledTool per tool) and uses a side-channel for secrets — workable but precludes generic middleware and risks a tool that forgets the wrapper, an MNPI wall hole.

---

## FR-7: Numeric / time-series MetricScorer + historical TimeSeriesRegressionGate  — **P0**

**Motivation (hedge-fund use case).** Grading a backtest means scoring financial metrics (Sharpe, max drawdown, turnover, hit-rate) against numeric thresholds and detecting regression versus a metric's historical golden series — not regex-on-text. Overfitting gates compare this run's metrics to prior approved runs.

**Current behavior / gap.** Eval scorers (Equality, Regex, LLM-Judge, Rubric, Pairwise) are oriented at LLM text output. There is no native numeric threshold scorer nor a regression gate over a metric's historical values.

**Proposed API / behavior.**
```rust
Add `MetricScorer { metric: &str, predicate: Threshold (AtLeast|AtMost|Between|Outside) }` operating on a numeric f64 (mirroring atomr-orgs GateCriterion threshold semantics for consistency). Add `TimeSeriesRegressionGate { metric, baseline: GoldenSeries, tolerance: Abs(f64)|Pct(f64)|ZScore(f64), direction: HigherIsBetter|LowerIsBetter }` that loads prior golden values from a pluggable `MetricHistoryStore` and fails when the new value regresses beyond tolerance. Both implement the existing Scorer trait so they compose in eval suites and regression gates. Provide a `MetricHistoryStore` backed by atomr-persistence (sql) keyed by (suite, metric).
```

**Acceptance criteria.**
- MetricScorer evaluates AtLeast/AtMost/Between/Outside on a numeric metric and returns a pass/fail + score.
- TimeSeriesRegressionGate fails when a metric regresses beyond Abs/Pct/ZScore tolerance vs stored golden values.
- Both compose in an eval suite alongside text scorers and feed the existing regression-gate machinery.
- Golden values persist and are queryable by (suite, metric).

**Fallback if declined.** hedgehog implements a numeric scorer + Postgres golden table in hedgehog-backtest, diverging from the upstream Scorer/regression-gate contract used elsewhere in the fund.

---

## FR-8: Composite/weighted scorer with ranked TriageResult and per-scorer breakdown  — **P1**

**Motivation (hedge-fund use case).** Opportunity triage and thesis review need ONE reproducible composite score that aggregates Rubric + LLM-Judge + Pairwise + deterministic numeric gates, with auditable per-scorer contributions and percentile/rank output, so candidates can be ranked consistently and the decision is defensible.

**Current behavior / gap.** Scorers exist individually; there is no first-class weighted aggregator producing a single ranked result with per-scorer breakdown and percentile/ranking. Each subsystem would hand-roll aggregation, risking divergence.

**Proposed API / behavior.**
```rust
Add `CompositeScorer { components: Vec<(Box<dyn Scorer>, weight:f64)>, aggregation: WeightedSum|WeightedMean|Min|Custom(fn) }` returning `TriageResult { composite: f64, breakdown: Vec<ScorerContribution { name, raw, weighted }> }`. Add a `Ranker` over a batch: `rank(items, scorer) -> Vec<RankedItem { item, triage: TriageResult, rank: usize, percentile: f64 }>`. Deterministic given identical component outputs; LLM components participate via recorded outputs in replay mode (see recording FR) for reproducibility.
```

**Acceptance criteria.**
- CompositeScorer returns a single composite plus a per-component weighted breakdown.
- Ranker produces stable ranks + percentiles over a batch.
- Mixed deterministic + LLM components compose; replay reproduces the same composite when component outputs are recorded.
- Weights and aggregation are declarative and serialized into the eval record.

**Fallback if declined.** hedgehog builds a bespoke triage aggregator; risks scoring drift from the rest of the eval/regression-gate machinery.

---

## FR-9: Native provenance/citation-coverage scorer (non-LLM)  — **P1**

**Motivation (hedge-fund use case).** Books-and-records and regulatory defensibility require that every quantitative claim in a published thesis trace to a sourced document (a retrieved EvidenceBundle with a valid doc_hash). This must be a deterministic, non-LLM check, not a Rubric/LLM-Judge approximation.

**Current behavior / gap.** Rubric and LLM-Judge can subjectively assess citations, but there is no deterministic scorer that verifies each flagged claim is backed by an EvidenceBundle whose doc_hash validates against the retrieval store.

**Proposed API / behavior.**
```rust
Add `ProvenanceScorer { claim_extractor: fn(&Output) -> Vec<Claim>, evidence_index: &EvidenceIndex }` that, for each Claim carrying a citation reference, verifies (a) an EvidenceBundle with the cited doc_hash exists in the index and (b) the doc_hash recomputes from stored content (integrity). Returns `coverage: f64` (fraction of claims backed) + a list of uncovered/invalid claims. Define an `EvidenceBundle { doc_hash, source_uri, retrieved_at, snippet }` type and an `EvidenceIndex` trait the retriever zoo can populate at retrieval time.
```

**Acceptance criteria.**
- Scorer fails when any quantitative claim lacks a backing EvidenceBundle.
- Scorer fails when a cited doc_hash does not recompute from stored content (tamper/missing source).
- Returns numeric coverage plus the list of uncovered/invalid claims.
- Deterministic: no LLM call in the scoring path.

**Fallback if declined.** hedgehog hand-builds a claim->doc_hash checker; a reusable upstream type avoids each desk defining EvidenceBundle differently.

---

## FR-10: Production vector store backend + embeddings provider wiring for the retriever zoo  — **P0**

**Motivation (hedge-fund use case).** The semantic layer over filings/news/research depends on a concrete, swappable, production vector store (Redis-backed in hedgehog's hot tier) and a pluggable embeddings interface. The retriever zoo (Bm25/Vector/MultiQuery/Ensemble/SelfQuery) and ingest module are listed, but a production vector backend is unverified (atomr-ontology-embed exists separately with no vector backend listed).

**Current behavior / gap.** Vector/Ensemble retrievers and an embeddings concept are referenced, but no concrete vector store backend (e.g. Redis vector, pgvector) and embeddings-provider wiring are documented as shipping.

**Proposed API / behavior.**
```rust
Define a `VectorStore` trait: `upsert(Vec<(id, Vec<f32>, Metadata)>)`, `query(Vec<f32>, k, filter: MetadataFilter) -> Vec<Hit { id, score, metadata }>`, `delete(ids)`. Ship at least a Redis-backed implementation (reusing atomr-persistence-redis) and a pgvector implementation. Define an `Embeddings` trait `embed(Vec<&str>) -> Vec<Vec<f32>>` with provider implementations behind the existing provider feature-flags (anthropic/openai/gemini). Wire VectorRetriever to take `(VectorStore, Embeddings)` so backends are swappable at host time.
```

**Acceptance criteria.**
- A Redis-backed VectorStore implementation ships and passes a round-trip upsert/query/delete test.
- VectorRetriever is constructed from injectable VectorStore + Embeddings.
- Embeddings provider is selectable via provider feature-flags.
- MetadataFilter is honored at query time by the backend (not post-filtered).

**Fallback if declined.** hedgehog implements a VectorStore over Redis and an embeddings client itself; acceptable but means the retriever zoo's Vector/Ensemble retrievers can't be used out of the box.

---

## FR-11: Retriever filter hook: hard point-in-time (as-of) + compartment entitlement, enforced below SelfQuery  — **P0**

**Motivation (hedge-fund use case).** Lookahead-free research is non-negotiable: a thesis built on post-dated evidence produces unrealizable strategies on real capital. And substrate entitlement (an analyst sees only documents in their compartments) must be enforced at query time, not post-filtered in the prompt. SelfQuery can filter by metadata but is LLM-driven and therefore not a guarantee.

**Current behavior / gap.** SelfQuery filters via LLM-generated metadata predicates; there is no hard, retriever-level guarantee that no document with system_time > as_of is ever returned, and no Compartment/clearance entitlement filter applied below the prompt.

**Proposed API / behavior.**
```rust
Add a non-LLM `RetrieverFilter` applied inside every retriever (Bm25/Vector/MultiQuery/Ensemble/SelfQuery) before results are returned: `AsOf(system_time: Timestamp)` excludes any doc whose recorded system_time/ingested_at exceeds the ceiling; `Entitlement(ClearanceContext)` excludes any doc whose Compartment is not held by the subject. Expose via `retriever.with_filter(RetrieverFilter)` and a `RetrievalCtx { as_of: Option<Timestamp>, clearance: Option<ClearanceContext> }` carried alongside the query so the guarantee holds even when SelfQuery generates its own metadata predicates (filters AND, never OR, with SelfQuery output). Documents must carry system_time and compartment metadata at ingest.
```

**Acceptance criteria.**
- With an as_of ceiling, no document recorded after that instant is ever returned by any retriever, including SelfQuery.
- With an Entitlement filter, documents outside the subject's compartments are never returned.
- Filters are enforced in the retriever (deterministic), not via the LLM prompt, and AND with any SelfQuery predicate.
- Ingest records system_time and compartment metadata required by the filters.

**Fallback if declined.** hedgehog wraps each retriever with a post-filter; risky because SelfQuery's LLM predicate can still leak and a post-filter cannot fix relevance/ranking already skewed by lookahead documents.

---

## FR-12: Host trigger runtime: event/webhook triggers + calendar-aware cron + runtime control surface  — **P0**

**Motivation (hedge-fund use case).** Real-time risk breakers are event-driven (on-fill, on-mark, on-breach); reconciliation/settlement/venue sessions are trading-calendar bound ('EOD on business days', '16:00 ET on trading days'); broker drop-copy needs a push event for low-latency capture; and a misconfigured live trigger on real markets must be throttleable/pausable instantly by ops. Demotion via the org ladder is too slow for a runaway market-event trigger.

**Current behavior / gap.** The verified host Scheduler only parses 'every:Ns/m/h/d' interval expressions (parse_expression). There is no host-level event/webhook trigger (webhook handling exists only in channel-harness-web), no calendar/holiday-aware cron, and no runtime introspection/control or per-trigger rate limiting. The brief lists cron/event/webhook/heartbeat triggers but the verified host has only fixed intervals.

**Proposed API / behavior.**
```rust
Extend the host trigger layer with: (1) `EventTrigger { source: WebhookSource|StreamSource|PubSubTopic, filter }` firing a run on inbound events (broker drop-copy, on-fill), reusing atomr-streams Sources; (2) `CronTrigger { schedule: CronExpr, calendar: Option<TradingCalendar>, tz }` supporting real cron fields and a holiday/business-day calendar so 'EOD on trading days' is expressible; (3) a `TriggerControl` surface: `list_triggers() -> Vec<TriggerStatus>`, `pause(trigger_id)`, `resume(trigger_id)`, `set_cooldown(trigger_id, Duration)`, and per-trigger `RateLimit { max_fires, per: Duration, on_exceed: Drop|Queue|Backpressure }` applicable to cron/heartbeat/event triggers. Trigger fires and control actions emit onto the telemetry topic.
```

**Acceptance criteria.**
- An inbound webhook/stream event fires a registered run with payload delivered to it.
- A CronTrigger with a TradingCalendar fires only on business days and skips holidays.
- Ops can list active triggers and pause/resume/cooldown a specific trigger at runtime without redeploy.
- A per-trigger RateLimit caps fire rate and applies the configured on_exceed policy.
- Trigger lifecycle and control actions are emitted as observable events.

**Fallback if declined.** hedgehog builds its own event bus + cron-with-calendar + control plane around the host; feasible but exactly the upstream gap we want closed, and risks divergence from the documented trigger primitives.

---

## FR-13: atomr-orgs CompiledOrg -> atomr-agents-org projection adapter (with compartment-wall sync)  — **P1**

**Motivation (hedge-fund use case).** atomr-orgs is the system-of-record for who/what the firm is; atomr-agents-org is the workforce routing org. Drift between them silently breaks information walls or routing. A supported, canonical, testable projection makes the reporting-hierarchy->routing mapping and compartment-wall sync first-class instead of hand-written in hedgehog.

**Current behavior / gap.** The two org models are independent. There is no documented adapter to build an agents-org Org/Department/Team from an atomr-orgs CompiledOrg, and keeping Policy::narrow / NamespacedMemory in sync with atomr-orgs Compartments is manual.

**Proposed API / behavior.**
```rust
Add an `OrgProjection` adapter: `project(compiled: &atomr_orgs::CompiledOrg) -> AgentsOrg` mapping Unit->Department/Team, Role->routing target, and reporting hierarchy->RoutingStrategy edges. Crucially, map each atomr-orgs Compartment to the corresponding Policy::narrow scope + NamespacedMemory namespace, and emit a `WallSyncReport` listing any compartment present in the SOR but missing in the routing org (fail-closed). Provide `OrgProjection::verify(compiled, agents_org) -> Vec<Drift>` for CI so a divergence between SOR and routing org is a test failure, not a runtime wall breach.
```

**Acceptance criteria.**
- project() produces an agents-org whose routing edges reflect the CompiledOrg reporting hierarchy.
- Every atomr-orgs Compartment maps to a Policy::narrow scope + memory namespace; unmapped compartments fail closed.
- verify() detects and reports drift between the SOR org and the routing org.
- Projection is deterministic and round-trippable in a test.

**Fallback if declined.** hedgehog hand-writes the projection in hedgehog-fleet and a custom drift test; the wall-sync logic is then duplicated and easy to get subtly wrong.

---

## FR-14: Compile-time validation of @persona declared clearance/compartments against RoleActor ClearanceContext  — **P1**

**Motivation (hedge-fund use case).** If a compiled persona's declared clearance/compartments drift from the Role's actual ClearanceContext, an analyst could be granted or denied access inconsistently — an MNPI information-wall hazard. This must fail closed at COMPILE/host time, not at runtime when money is moving.

**Current behavior / gap.** Personas declare access via the @persona decorator/guest-mode, and RoleActor injects ClearanceContext, but there is no documented binding that validates the two agree at compile/host time; wiring is manual in hedgehog-fleet.

**Proposed API / behavior.**
```rust
Extend the @persona schema with `clearance: ClearanceLevel` and `compartments: Vec<Compartment>` declarations, and add a host-time check `bind_persona_to_role(persona, role_clearance: &ClearanceContext) -> Result<(), ClearanceMismatch>` that fails closed when the persona declares access the Role's ClearanceContext does not grant (or vice versa per policy). Run this during OrgProjection/host composition so a mismatch aborts startup with a typed error naming the offending compartment/level. Emit the validated binding to the audit telemetry topic.
```

**Acceptance criteria.**
- @persona can declare clearance level + required compartments.
- Host composition fails closed with a typed ClearanceMismatch naming the offending compartment when persona and Role disagree.
- Validation runs at compile/host time, not first tool call.
- Successful bindings are emitted for audit.

**Fallback if declined.** hedgehog validates persona vs ClearanceContext in its own host bootstrap; works but each integrator must remember to call it, and the failure is not enforced by the framework.

---

## FR-15: Schema / data-contract validation operator with HITL-interrupt binding in pipelines  — **P1**

**Motivation (hedge-fund use case).** Data-quality gating (type drift, bar gaps, late/out-of-order data) is core to trustworthy ingestion feeding money-moving decisions. A contract breach should be able to raise a HITL interrupt from inside a pipeline stage so a human reviews suspect data before it propagates.

**Current behavior / gap.** Pipelines compose prompt->model->parser with .then/.fan_out/.assign, but there is no documented data-contract/schema-drift validation operator, and no documented way to raise a HITL interrupt from within a pipeline stage on a contract breach.

**Proposed API / behavior.**
```rust
Add a `ContractValidate(DataContract)` pipeline operator where `DataContract { schema: TypeSchema, freshness: MaxLag, continuity: ExpectedCadence, custom: Vec<CheckFn> }` validates each record/batch and yields `Valid(T)` or `Breach(ContractViolation { kind: TypeDrift|StaleData|Gap|Custom, detail })`. Add an `on_breach` policy: `Drop | RouteTo(sink) | Interrupt(InterruptSpec)` where Interrupt raises a durable HITL interrupt (see registry FR) carrying the ContractViolation as payload and parking the pipeline branch until resolved. Expose as `.validate(contract).on_breach(policy)` composable in the existing pipeline DSL.
```

**Acceptance criteria.**
- Operator detects type drift, stale data beyond MaxLag, and cadence gaps.
- A breach can raise a durable HITL interrupt carrying the violation payload from inside a pipeline stage.
- on_breach policies Drop/RouteTo/Interrupt are selectable per stage.
- Valid records flow through unchanged with no measurable overhead when contracts pass.

**Fallback if declined.** hedgehog writes a custom stream operator and manually triggers interrupts; the interrupt-from-pipeline path in particular is unverified upstream and risky to assume.

---

## FR-16: Dedup / novelty retriever primitive + incremental-ingest hook for an append-only artifact store  — **P2**

**Motivation (hedge-fund use case).** Alert-fatigue control depends on cheap, consistent near-duplicate/novelty scoring before inference, so opportunity-discovery doesn't re-surface the same signal. Reusing one upstream primitive avoids each hedgehog subsystem reimplementing similarity thresholds and merge semantics differently.

**Current behavior / gap.** Vector/Ensemble retrievers exist, but there is no first-class dedup/novelty primitive returning similarity + merge candidates, and no standard incremental-ingest hook for an append-only artifact store.

**Proposed API / behavior.**
```rust
Add a `NoveltyRetriever` (built on Vector/Ensemble): `assess(item) -> Novelty { max_similarity: f64, is_novel: bool, merge_candidates: Vec<(id, score)> }` against a corpus, with a configurable threshold and `merge: KeepFirst|KeepLatest|Cluster`. Add an `IncrementalIngest` hook: `ingest_if_novel(item, NoveltyPolicy) -> Ingested { id }|Merged { into }` writing to an append-only artifact store and updating the vector index in one step, so dedup and ingest share one similarity definition.
```

**Acceptance criteria.**
- NoveltyRetriever returns max similarity + merge candidates against a corpus with a configurable threshold.
- ingest_if_novel writes novel items append-only and reports merges for duplicates.
- Dedup and ingest use the same embeddings/similarity configuration.
- Idempotent re-ingest of an identical item is a no-op (Merged).

**Fallback if declined.** Each hedgehog subsystem hand-rolls a similarity threshold + merge rule; inconsistent dedup across desks and duplicated code.

---

## FR-17: First-class debate/critique protocol primitive (DebateStrategy + CritiqueChannel + dissent termination)  — **P1**

**Motivation (hedge-fund use case).** Adversarial verification of investment theses is the core mechanism for trustworthy research with real money. The supervisor/swarm patterns exist but there is no built-in structured adversarial-debate loop; reimplementing ResearchConvergenceLoop per desk (research, risk review, strategy review) is duplicative and error-prone.

**Current behavior / gap.** Multi-agent patterns include supervisor, swarm, and hierarchical routing, but no documented adversarial-debate primitive with rebuttal routing and a dissent-detecting termination condition.

**Proposed API / behavior.**
```rust
Add a `DebateStrategy` (a multi-agent pattern) parameterized by `roles: { proponent, critic[, judge] }`, `max_rounds`, and a `CritiqueChannel` typed State channel (reducer AppendMessages) capturing each round's claim/rebuttal with author and stance. Add a `DissentTermination: TerminationStrategy` that ends when consensus is reached (no unresolved dissent) or max_rounds, surfacing a `DebateOutcome { converged: bool, unresolved: Vec<Dissent>, transcript }`. Reusable across desks by supplying different personas. Composes with the recording Checkpointer so a debate is replayable.
```

**Acceptance criteria.**
- DebateStrategy runs N adversarial rounds with rebuttal routing between proponent and critic.
- DissentTermination ends on convergence or max_rounds and reports unresolved dissents.
- The full transcript is captured in a typed CritiqueChannel and is checkpointed/replayable.
- The same primitive is reused by supplying different personas (research vs risk review).

**Fallback if declined.** hedgehog builds ResearchConvergenceLoop by hand and copies it to each desk; divergent behavior and no shared termination semantics.

---

## FR-18: Cost/budget enforcement primitive + spend-to-decision-key attribution  — **P1**

**Motivation (hedge-fund use case).** A hedge fund must hard-cap inference spend per desk/strategy and kill runs on overspend, AND continuously know whether each agent decision earns more than it costs to compute. Today budgets are hand-rolled around every model call and there is no attribution of spend to a business outcome/decision-key.

**Current behavior / gap.** Cost/token accounting is per-call at best (usage on a completion). There is no pre-flight cost estimator, no spend-accounting hook integrated with TerminationStrategy, and no attribution of model spend to a decision-key for cost-vs-alpha analysis.

**Proposed API / behavior.**
```rust
Add a `CostMeter` component on the model Callable: `estimate(request) -> CostEstimate` (pre-flight) and `record(usage, model) -> Spend` (post-call), with a `Budget { cap: Money|Tokens, scope: BudgetScope }` and a `BudgetExceeded` signal wired into TerminationStrategy so a run terminates cleanly on overspend. Add a `decision_key` tag on CallCtx (e.g. (desk, strategy, decision_id)) so every Spend is attributed; expose a `SpendLedger` query `spend_by(decision_key) -> Money` for cost-vs-value analysis (value supplied externally by hedgehog's PnL attribution). Spends emit onto the telemetry topic.
```

**Acceptance criteria.**
- Pre-flight estimate is available before a model call; post-call spend is recorded.
- A Budget cap at desk/strategy scope triggers BudgetExceeded and terminates the run via TerminationStrategy.
- Every Spend is attributed to a decision_key carried on CallCtx.
- SpendLedger can total spend by decision_key for external cost-vs-alpha analysis.

**Fallback if declined.** hedgehog wraps every model Callable with budget + spend tagging middleware; works but no standard estimate/draw/terminate primitive and attribution keys diverge across desks.

---

## FR-19: Structured per-step run telemetry emitted to a journal / pub-sub topic with checkpoint pointer  — **P0**

**Motivation (hedge-fund use case).** The visibility layer's action drill-down ('agent -> action -> source data') and record-and-replay inspection depend on per-step events being externally observable and resolvable to a checkpoint. If telemetry is internal-only, the UI cannot reconstruct what a live agent did without re-inference, which the determinism rule forbids.

**Current behavior / gap.** Runs produce internal trace/budget info, but there is no documented structured emission of per-step events (checkpoint created, tool dispatched, token I/O, pinned model/provider version) onto an external journal or pub-sub topic that a projector can consume, with a pointer back to (workflow,run,step).

**Proposed API / behavior.**
```rust
Add a `TelemetrySink` (atomr-streams Sink or atomr distributed pub/sub topic) that the runtime emits `RunEvent` to per step: `RunEvent { kind: CheckpointCreated|ToolDispatched|ToolReturned|InferenceCompleted|InterruptRaised|InterruptResolved|ModelDrift|BudgetSpent, workflow, run, step, checkpoint_ref, model_pin, tokens, ts }`. Configure via `AgentBuilder.with_telemetry(sink)`. Every event carries a `checkpoint_ref` resolvable through the Checkpointer so an external projector can fetch the exact recorded state/inputs. This is the single backbone that the model-drift, budget, and interrupt-registry FRs publish onto.
```

**Acceptance criteria.**
- Every step emits structured RunEvents to a configurable external sink/topic.
- Each event carries a checkpoint_ref resolvable to the (workflow,run,step) record.
- Events include pinned model/provider version and token I/O.
- An external projector can reconstruct the full action timeline of a run without re-inference.

**Fallback if declined.** hedgehog instruments each agent to emit its own events; duplicative, inconsistent, and unlikely to carry a stable checkpoint pointer.

---

## FR-20: Machine-readable provider/model compliance attestation metadata (region, BAA, MNPI-approved)  — **P1**

**Motivation (hedge-fund use case).** Gating which LLM an MNPI compartment may use requires per-model data-residency/region, contractual (BAA) coverage, and 'MNPI-approved' attributes so the compartment->model policy is derived and auditable, not maintained as a brittle hardcoded string allowlist.

**Current behavior / gap.** allowed_models is a Vec<String> and Provider is an enum; there is no machine-readable attribute for region/residency, BAA/contractual coverage, or MNPI-approval. policy_for must hardcode an in-region model allowlist.

**Proposed API / behavior.**
```rust
Add a `ModelAttributes { provider, model_id, region: Region, residency: DataResidency, contractual: Vec<Coverage (BAA|DPA|...)>, tags: Set<String> (e.g. 'mnpi-approved') }` registry queryable as `model_registry.attributes(model_id)`. Let policy be derived: `PolicyStrategy` can filter allowed_models by a predicate over ModelAttributes (e.g. require region==EU AND tags contains 'mnpi-approved' for an MNPI compartment). Attributes are versioned and the source of a policy decision is auditable (emit to telemetry).
```

**Acceptance criteria.**
- Each model exposes machine-readable region/residency/contractual/tags attributes.
- A compartment->model policy can be expressed as a predicate over attributes, not a string list.
- A model lacking required attributes is excluded from the allowlist for that compartment.
- The attribute set backing a policy decision is auditable.

**Fallback if declined.** hedgehog maintains its own model-attribute table and derives policy outside the framework; the framework's allowed_models then can't enforce it consistently.

---

## FR-21: Content-trust / taint boundary separating untrusted retrieved content from instructions  — **P1**

**Motivation (hedge-fund use case).** For a fund where ingested text drives money-moving decisions, prompt injection from untrusted corpus/tool content is a direct path to manipulated trades. The framework currently offers no taint/trust boundary distinguishing instructions from retrieved/ingested content.

**Current behavior / gap.** Retrieved/ingested content and instructions are not separated in agent context; there is no built-in defense against injection from tool/retriever returns.

**Proposed API / behavior.**
```rust
Add a `Trust` tag on context content: `Trusted(instructions/system)` vs `Untrusted(retrieved/tool-return/ingested)`. ToolReturn and retriever Hits are tagged Untrusted by default. Provide a `TrustPolicy` the runtime applies when assembling a prompt (e.g. wrap untrusted content in a delimited, non-instruction region; optionally run an `InjectionScreen` scorer over untrusted spans) and a RichTool/retriever contract that preserves the taint through the pipeline. Expose whether a given decision consumed untrusted content for the abuse-guardrail layer.
```

**Acceptance criteria.**
- Tool/retriever returns are tagged Untrusted by default and instructions Trusted.
- Prompt assembly applies a TrustPolicy that isolates untrusted content from the instruction channel.
- An optional InjectionScreen can flag suspicious untrusted spans before they reach the model.
- The taint provenance of content consumed by a decision is queryable.

**Fallback if declined.** hedgehog wraps retrievers/tools to delimit untrusted content and screens it; doable but a framework-level taint type is far more robust and reusable than per-tool string wrapping.

---

## FR-22: Required evidence-trace on ToolReturn for decision explainability/auditability  — **P1**

**Motivation (hedge-fund use case).** Autonomous trading requires every order to be explainable to its evidence for regulators and abuse-prevention. ToolReturn carries content/artifact but no required evidence-trace tying a decision to its inputs, so 'why did the agent place this order?' cannot be answered from the record.

**Current behavior / gap.** ToolReturn::{Content,ContentAndArtifact,Command} carries outputs but no required mechanism linking the return to the evidence/inputs that justified it.

**Proposed API / behavior.**
```rust
Add an optional-but-enforceable `EvidenceTrace { inputs: Vec<EvidenceRef (doc_hash|checkpoint_ref|measure_id)>, rationale: String }` field to ToolReturn (or a RichTool contract that must populate it). Add an `ExplainabilityPolicy` configurable per tool/desk: `Require | Warn | Off`, so money-moving tools (order placement) MUST attach an EvidenceTrace or the call is rejected (typed `MissingEvidence`). EvidenceTrace is persisted into the StepRecord (recording FR) and emitted via telemetry so the visibility layer can render agent->action->source-data.
```

**Acceptance criteria.**
- ToolReturn can carry a structured EvidenceTrace referencing the inputs/evidence behind the decision.
- ExplainabilityPolicy::Require rejects a money-moving tool return lacking an EvidenceTrace.
- EvidenceTrace is persisted in the step record and externally observable via telemetry.
- The trace resolves to retrievable evidence (doc_hash/checkpoint_ref/measure_id).

**Fallback if declined.** hedgehog enforces evidence on its own order tools and stashes rationale in artifacts; works for hedgehog's tools but not a reusable framework guarantee and easy to omit on a new tool.

---
