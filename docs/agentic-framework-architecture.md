# Agentic Framework Architecture

A composable agentic framework built on Rust Actix actors with Akka-style supervision, designed to treat context shaping, composition, and persistence as first-class concerns.

## Design Thesis

Agentic systems don't fail because the models aren't good enough — they fail because the substrate underneath them treats context, composition, and persistence as afterthoughts. This architecture makes every context-shaping decision a pluggable strategy, every execution unit a supervised actor, and every proven pattern a versioned, testable harness. The result turns "build an agent" from a research project into an engineering discipline.

## Layered Composition

The system is organized as layers over a single substrate. Each layer is observable, swappable, versioned, and testable.

```
Harnesses        — tested, persistent loops (the product)
Workflows        — deterministic procedures (the guarantees)
Agents           — adaptive decision-makers (the flexibility)
Strategies       — pluggable behaviors for every component (the optimization surface)
Tool sets        — packaged, versioned capabilities (the permissions surface)
Skills           — bundled instruction+tool+sub-agent capabilities (the composition unit)
Actors + streams — bounded, supervised, backpressured execution (the runtime)
CUDA/Inference   — native model and GPU work (the floor)
Python bridge    — host and guest execution (the ergonomics layer)
```

## Runtime Substrate

The framework assumes existing Rust actors for low-level CUDA processing and inference. Every component above runs as an Actix actor with bounded mailboxes, supervision trees, and Actix streams for backpressure. Bounded channels at every boundary mean that saturation downstream applies pressure all the way upstream — through teams, organizations, and harnesses — without unbounded queues hiding latency.

## Agents

An agent is a single decision-making actor composed from pluggable strategies.

```rust
pub struct Agent<I, T, Ms, Ml, Sk>
where
    I: InstructionStrategy,
    T: ToolStrategy,
    Ms: MemoryStrategy,
    Ml: MemoryStrategy,
    Sk: SkillStrategy,
{
    id: AgentId,
    instructions: I,
    tools: T,
    short_term: Ms,
    long_term: Ml,
    skills: Sk,
    inference: Addr<InferenceActor>,
    inbox: BoundedReceiver<AgentMsg>,
}

impl<...> Actor for Agent<...> {
    type Context = Context<Self>;
}
```

Generics are used where monomorphized hot paths matter; `Box<dyn>` is used where strategies must be swapped at runtime via config.

### Per-Turn Pipeline

```
incoming msg
  → MemoryStrategy::retrieve     ┐
  → SkillStrategy::applicable    ├─ parallel, share TokenBudget
  → ToolStrategy::select         ┘
  → InstructionStrategy::render
  → ContextAssembler (priority merge under budget)
  → InferenceActor::call
  → tool execution loop (each tool is itself an actor address)
  → MemoryStrategy::store
```

Every arrow is observable, every box is swappable.

## Strategies

Strategies are the universal extension point. Every component answers one question: *given the current context and budget, what do you contribute?*

```rust
#[async_trait]
pub trait ContextStrategy: Send + Sync {
    type Output;
    async fn resolve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<Self::Output>;
}

#[async_trait]
pub trait ToolStrategy: Send + Sync {
    async fn select(&self, ctx: &AgentContext, budget: &mut TokenBudget)
        -> Result<Vec<ToolHandle>>;
}

#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    async fn retrieve(&self, ctx: &AgentContext, budget: &mut TokenBudget)
        -> Result<Vec<MemoryChunk>>;
    async fn store(&self, item: MemoryItem) -> Result<()>;
}
```

Strategies compose. `Chained(Recency, Semantic, Summarize)` is itself a `MemoryStrategy`. They can be A/B tested by routing a percentage of traffic to a variant config — agent code never changes.

### Tool Strategy Evolution

The same trait progresses through implementations of increasing sophistication:

| Version | Strategy | Behavior |
|---------|----------|----------|
| v0 | `StaticToolStrategy` | Hand-picked fixed list |
| v1 | `KeywordToolStrategy` | Lexical filter (TF-IDF) |
| v2 | `EmbeddingToolStrategy` | Vector search over a superset of tool descriptions via CUDA actor |
| v3 | `LearnedToolStrategy` | Predict from interaction history |
| v4 | `HierarchicalToolStrategy` | Category router → leaf tool selection |

`EmbeddingToolStrategy` is the critical jump: query embedding goes to the CUDA-backed embedding actor, ANN search runs over thousands of tool descriptors, and only the top-k that fit budget enter the prompt. The agent's context window only ever sees the relevant slice.

## Tool Sets

A `ToolSet` is a named, versioned bundle — the unit of tool packaging, distribution, and permission.

```rust
pub struct ToolSet {
    id: ToolSetId,
    version: SemVer,
    tools: Vec<Arc<dyn Tool>>,
    metadata: ToolSetMeta,
    dependencies: Vec<ToolSetId>,
    permissions: PermissionSpec,
}

pub trait Tool: Send + Sync {
    fn descriptor(&self) -> &ToolDescriptor;
    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value>;
}
```

Tool sets compose: `WebToolSet = HttpToolSet + BrowserToolSet + ScrapingToolSet`. The `ToolStrategy` pulls from a `ToolSetRegistry` filtered by what the agent (or its team, or its org) is granted. Tool sets themselves can be resolved by a `ToolSetStrategy::resolve(ctx) -> Vec<ToolSetId>`, allowing dynamic grant/revoke based on task context.

## Skills

A skill is a closure over instructions fragment, tool set, optional memory namespace, and optional sub-agents. `SkillStrategy::applicable(ctx)` decides which skills to inject this turn — same dynamic-vs-static evolution path as tools. Skills can register sub-agents that the parent can delegate to, which is how higher-level systems compose: a `ResearchSkill` brings along a search agent and a synthesis agent.

## Personas

The instruction strategy resolves what the agent sees as its system prompt. Adding a persona slot makes "who the agent is" a separately addressable, separately strategizable concern from "what the agent does." Identity and task are optimized differently — task instructions tighten with eval results, while persona shifts based on audience, register, and the social shape of the interaction.

```rust
pub struct ComposedInstructionStrategy<P, T, B>
where
    P: PersonaStrategy,
    T: TaskStrategy,
    B: BehaviorStrategy,
{
    persona: P,
    task: T,
    behavior: B,
    assembler: Box<dyn InstructionAssembler>,
}

#[async_trait]
impl<P, T, B> InstructionStrategy for ComposedInstructionStrategy<P, T, B> {
    async fn render(&self, ctx: &AgentContext, budget: &mut TokenBudget)
        -> Result<RenderedInstructions> {
        let (persona, task, behavior) = tokio::join!(
            self.persona.resolve(ctx, budget),
            self.task.resolve(ctx, budget),
            self.behavior.resolve(ctx, budget),
        );
        self.assembler.assemble(persona?, task?, behavior?, budget)
    }
}
```

The persona, task, and behavior slots share the token budget cooperatively. Under budget pressure, persona compresses preferentially — task fidelity dominates when context is tight.

### The Persona Strategy Trait

```rust
#[async_trait]
pub trait PersonaStrategy: Send + Sync {
    async fn resolve(&self, ctx: &AgentContext, budget: &mut TokenBudget)
        -> Result<RenderedPersona>;
}

pub struct RenderedPersona {
    identity: String,
    salient_traits: Vec<TraitFragment>,
    style_directives: StyleSpec,
    metadata: PersonaMetadata,
}
```

The trait is intentionally minimal. What varies is *how* a persona is structured and *how* it's emphasized — those are the strategy implementations.

### Persona Structure Strategies

Different psychological frameworks structure personality differently. Each becomes a strategy implementation that produces consistent, internally coherent personas.

#### Static String Strategy

The simplest. A hand-written persona description, possibly with templated variables.

```rust
pub struct StaticPersonaStrategy {
    template: String,
    variables: HashMap<String, String>,
}
```

Useful as a baseline and where authors want full control. No emphasis logic — the whole persona is always present.

#### Trait-Vector Strategy (Big Five / OCEAN)

Personality as a five-dimensional vector. Each dimension has a render template, and the strategy composes templates weighted by trait scores.

```rust
pub struct BigFivePersonaStrategy {
    openness: f32,
    conscientiousness: f32,
    extraversion: f32,
    agreeableness: f32,
    neuroticism: f32,
    rendering: Box<dyn TraitRenderer>,
}
```

Big Five has the strongest empirical support, and continuous values map naturally to "how much" of a trait to emphasize. A research agent might run high on openness and conscientiousness, low on extraversion. The strategy renders trait fragments proportionally and the assembler weaves them into prose.

#### Type-Based Strategy (Myers-Briggs)

MBTI structures personas as one of 16 four-letter types along four dichotomies: introversion/extraversion (I/E), sensing/intuition (S/N), thinking/feeling (T/F), judging/perceiving (J/P).

```rust
pub struct MbtiPersonaStrategy {
    mbti_type: MbtiType,
    cognitive_stack: CognitiveStack,
    expression: ExpressionLevel,
}

pub enum MbtiType { INTJ, INTP, ENTJ, ENTP, /* ... */ }

pub struct CognitiveStack {
    dominant: CognitiveFunction,    // Ni, Ne, Si, Se, Ti, Te, Fi, Fe
    auxiliary: CognitiveFunction,
    tertiary: CognitiveFunction,
    inferior: CognitiveFunction,
}
```

The cognitive stack is doing real work — it's not just the four-letter label but the underlying Jungian functions that determine how the persona thinks and decides. An INTJ leads with introverted intuition (Ni) and supports with extraverted thinking (Te); an ENFP leads with extraverted intuition (Ne) and supports with introverted feeling (Fi). The strategy renders behavior consistent with the cognitive stack, not just the letter pairs.

MBTI's empirical validity is contested, but it remains useful as an *organizing structure* — it gives authors and users a shared vocabulary for shaping agent personality. The framework is honest about this: MBTI is one structural option among several, not "the" model.

#### Jungian Archetype Strategy

Personas built on Jung's archetypal patterns — the Sage, Caregiver, Explorer, Hero, Magician, Outlaw, etc.

```rust
pub struct JungianArchetypeStrategy {
    primary: Archetype,
    shadow: Option<Archetype>,
    individuation: f32,
    expression: ArchetypeExpression,
}

pub enum Archetype {
    Sage, Caregiver, Explorer, Hero, Magician, Outlaw,
    Lover, Jester, Everyman, Innocent, Ruler, Creator,
}
```

Archetypes give personas narrative coherence. A Sage agent prioritizes truth and clarity; a Caregiver prioritizes user wellbeing; an Explorer prioritizes novelty and discovery. Adding a *shadow* archetype creates productive tension — a Sage with an Outlaw shadow questions authority where a pure Sage might defer.

This strategy works particularly well for user-facing agents where felt coherence matters more than psychological precision.

#### Composite Strategy

The most expressive. Combines multiple frameworks with weights.

```rust
pub struct CompositePersonaStrategy {
    layers: Vec<(Box<dyn PersonaStrategy>, f32)>,
    reconciliation: Box<dyn PersonaReconciler>,
}
```

A composite might combine a Big Five vector for trait-level granularity, a Jungian archetype for narrative shape, and a custom layer for domain expertise. The reconciler handles contradictions — if Big Five says "high agreeableness" but the archetype is Outlaw, the reconciler decides which dominates in which context.

#### Other Frameworks Worth Slotting In

| Framework | Useful For |
|-----------|-----------|
| Enneagram | Motivation modeling; nine types with wings and instinctual variants |
| HEXACO | Big Five plus Honesty-Humility; ethical disposition |
| DISC | Business contexts; Dominance, Influence, Steadiness, Conscientiousness |
| Schwartz Values | Cross-cultural agents; ten universal values |
| Custom org frameworks | Internal personality/competency models |

Each becomes a `PersonaStrategy` implementation. Authors pick the framework that matches their mental model.

### Contextual Emphasis Strategies

A static persona is the same in every turn. A contextual persona emphasizes different aspects based on what's happening. This is the second axis of variation — independent of which structural framework you chose.

```rust
#[async_trait]
pub trait PersonaEmphasisStrategy: Send + Sync {
    async fn emphasize(
        &self,
        full_persona: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona>;
}
```

| Strategy | Behavior |
|----------|----------|
| `StaticEmphasis` | Render the full persona every turn — predictable, debuggable, token-expensive |
| `AudienceAdaptive` | Detect audience signals and emphasize traits suited to the audience |
| `TaskAdaptive` | Different tasks call out different facets — analytical for debugging, generative for brainstorming, empathic for difficult conversations |
| `MoodState` | Persona carries state across turns — more focused under pressure, more playful when relaxed; small state machine driven by signals from the agent loop |
| `GoalConditioned` | Emphasis depends on the agent's current sub-goal — warmth/competence early, decisiveness later |
| `LearnedEmphasis` | Small policy model trained on which emphasis profiles produced better outcomes for which contexts |

Same evolution path as the tool strategy — start static, measure, learn.

### Composition With the Rest of the System

Personas plug into existing structures cleanly:

- **At the agent level**, the persona is part of the agent's spec. Two agents can share task instructions and tool sets while differing only in persona — useful for ensembles where diverse perspectives matter.
- **At the team level**, persona policies enforce consistency. A customer-facing team might require all agents share a baseline persona (the brand voice) while specializing on top. The team's `PolicyStrategy` validates persona specs at agent construction.
- **At the org level**, persona libraries are managed as versioned artifacts — a `PersonaSet`, analogous to `ToolSet`. An org publishes its approved personas; teams grant them; agents instantiate them. Brand consistency, regulatory compliance, and persona evolution all become governable.
- **In skills**, a skill can carry a persona overlay — a "negotiation skill" might temporarily emphasize assertiveness traits while active, returning to baseline afterward.
- **In harnesses**, persona is part of the harness spec and tested against the eval suite. A `CodingHarness` ships with a tested coding-appropriate persona; swapping personas requires re-evaluation. This prevents the common failure mode where someone tweaks "tone" and silently degrades task performance.

### Why This Slot Pays Off

Personas in most agent frameworks are buried inside a system-prompt string, opaque to introspection and impossible to vary independently. Making the persona a strategy slot — with structural strategies (Big Five, MBTI, Jungian, composite) and emphasis strategies (audience, task, state, learned) as separate axes — gives four properties at once:

1. **Comparability** — two agents differing only in persona become a clean A/B
2. **Reusability** — personas become artifacts that travel across agents, teams, harnesses
3. **Governance** — orgs can approve, version, and audit personas as first-class objects
4. **Evolution** — emphasis can move from static to learned without disturbing the structural definition

The design honors that personality is genuinely multi-dimensional — there is no single right framework — by making the framework itself a strategy choice. Authors who think in Big Five vectors get Big Five vectors. Authors who think in archetypes get archetypes. The substrate doesn't take a side; it just makes whichever model you pick observable, swappable, and testable like everything else in the system.

## Workflows

Workflows are the deterministic counterpart to agents. Same actor model, same composition primitives, but execution is graph-driven instead of LLM-driven.

```rust
pub struct Workflow {
    id: WorkflowId,
    graph: Dag<StepId, Step>,
    state: WorkflowState,
    policy: Arc<Policy>,
}

pub enum Step {
    Invoke(Box<dyn Tool>),
    CallAgent(AgentRef, InputMapping),
    CallWorkflow(WorkflowRef, InputMapping),
    Branch(Box<dyn Predicate>, StepId, StepId),
    Parallel(Vec<StepId>, JoinStrategy),
    Loop(Box<dyn Predicate>, StepId),
    Map(Box<dyn IntoIter>, StepId, Concurrency),
    Human(ApprovalSpec),
}
```

State is event-sourced for durability. On crash, the supervisor replays the log and resumes from the last committed step.

### The Callable Abstraction

The linchpin of the design: agents, workflows, tools, and harnesses are all interchangeable behind a single trait.

```rust
#[async_trait]
pub trait Callable: Send + Sync {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value>;
}

impl Callable for AgentRef { ... }
impl Callable for WorkflowRef { ... }
impl Callable for Arc<dyn Tool> { ... }
impl Callable for HarnessRef { ... }
```

An agent can call a workflow as if it were a tool. A workflow step can be an agent invocation. A team can route to either. This lets you mix deterministic and probabilistic execution at every boundary, choosing rigor where you need it and flexibility where you don't.

## Harnesses

A harness is a *tested, packaged, persistent execution loop* — the highest-level composable unit. Where an agent is a single decision-maker and a workflow is a deterministic procedure, a harness is a long-running, opinionated loop combining them into a proven pattern for a domain.

### Defining Properties

1. **Persistent loop** — runs until a termination condition, not just request/response
2. **Tested** — ships with eval suites, traces, and known-good behavior
3. **Versioned and pinned** — like a library release; consumers get reproducibility
4. **Self-contained** — bundles agents, workflows, tool sets, skills, memory strategies, termination logic
5. **Observable by design** — every iteration produces structured events for replay and debugging

```rust
pub struct Harness {
    id: HarnessId,
    version: SemVer,
    spec: HarnessSpec,
    state: HarnessState,
    eval_suite: EvalSuiteId,
}

pub struct HarnessSpec {
    loop_strategy: Box<dyn LoopStrategy>,
    agents: Vec<AgentSpec>,
    workflows: Vec<WorkflowSpec>,
    toolsets: Vec<ToolSetId>,
    memory: MemoryConfig,
    termination: Box<dyn TerminationStrategy>,
    budget: BudgetSpec,
    observers: Vec<Box<dyn Observer>>,
}

#[async_trait]
pub trait LoopStrategy: Send + Sync {
    async fn step(&self, state: &mut HarnessState, ctx: &HarnessCtx)
        -> Result<StepOutcome>;
}

#[async_trait]
pub trait TerminationStrategy: Send + Sync {
    fn should_terminate(&self, state: &HarnessState) -> Termination;
}
```

A harness is itself a `Callable` and an actor with a bounded inbox.

### Canonical Harness Examples

**CodingHarness** — Loop: read task → plan → edit → run tests → observe → decide. Bundles a planner agent, an edit-and-execute workflow, file system + shell + test-runner tool sets, episodic memory of attempts. Terminates on green tests or exhausted budget. Ships with a repo bug-fix eval suite.

**ResearchHarness** — Loop: clarify → search → read → synthesize → identify gaps → repeat → produce report. Sub-harnesses for deep dives. Terminates on coverage threshold or budget.

**InvestmentHarness** — Loop: ingest market data → screen → analyze → risk check → propose → wait for approval → execute. Heavy on workflows, light on agent autonomy. Human-approval gates first-class.

**StatusHarness** — Loop on schedule: collect signals → diff → classify → draft → publish. Mostly workflows; agents only for narrative generation.

The pattern is the same; what differs is loop shape, agent/workflow ratio, tool sets, and termination criteria.

### Why "Tested" is Load-Bearing

The eval suite isn't a footnote — it's what makes a harness composable. When `ProductDevelopmentHarness` calls `CodingHarness@2.3.1`, it relies on a contract: "on this eval suite, this version achieves this success rate at this cost." Without that, you're composing black boxes. With it, harnesses become library dependencies — upgrade, pin, A/B, roll back with the same discipline as code.

The framework provides first-class support: `harness eval run`, regression detection on version bumps, automated gating before publish to a registry.

### Persistence and Resumability

Harnesses run for hours or days and must survive process restarts.

```rust
pub struct HarnessState {
    iteration: u64,
    history: Vec<StepEvent>,
    working_memory: Value,
    spawned: Vec<CallableHandle>,
    checkpoints: Vec<Checkpoint>,
}
```

Every step appends events to a durable log. On crash, the supervisor restarts the harness actor, replays events, and resumes from the last checkpoint. Spawned children are reattached or rerun based on idempotency markers.

## Organizational Hierarchy

Four levels, same primitives at each:

```
Organization
└── Department (or sub-org)
    └── Team
        └── Unit (Agent, Workflow, or Harness)
```

Every level is an actor with: a bounded inbox, a `RoutingStrategy`, a `PolicyStrategy` (rate limits, cost caps, allowed tool sets, allowed models), a memory namespace, and a supervision spec for its children.

```rust
pub struct Org {
    id: OrgId,
    children: HashMap<UnitId, Addr<Unit>>,
    routing: Box<dyn RoutingStrategy>,
    policy: Arc<Policy>,
    memory: OrgMemory,
    granted_toolsets: Vec<ToolSetId>,
}
```

### Policy Inheritance

Policy is inherited and *narrowed* downward — a team cannot grant itself tool sets the org didn't grant. Memory is namespaced and inherited with read/write rules: an agent reads its own + team + org memory but writes only to its own and (with permission) team scratchpad. This gives natural blast-radius control: a malfunctioning agent cannot poison org-level memory, and a compromised team cannot escalate tool access.

### Routing

The same `RoutingStrategy` trait at every level. `LLMRouter` at the org picks a department; `CapabilityMatch` at the team picks an agent. `RoundRobin`, `LoadAware`, and other strategies are interchangeable. Backpressure flows up the tree because every inbox is bounded.

### Mixed Composition

Agents, workflows, and harnesses all slot in at any level. A `HarnessSet` packages related harnesses (`EngineeringHarnessSet` ships `CodingHarness`, `CodeReviewHarness`, `IncidentResponseHarness`). HarnessSets are granted by policy the same way tool sets are. A workflow team can contain agent teams as resources, and an agent team can have workflow teams as resources — the hierarchy is a graph of organizational units parameterized by what each unit contains.

### Example Topology

```
SupportOrg
├── policy: $0.50/ticket cap, models {haiku, sonnet}, toolsets {crm, kb, web}
├── TriageTeam (workflow team)
│   └── ClassifyTicket workflow → routes to one of:
├── L1Team (agent team)
│   ├── routing: LoadAware
│   ├── toolsets: {crm-read, kb-search}
│   └── 20× FrontlineAgent (Haiku, FAQ skill set)
├── L2Team (agent team)
│   ├── toolsets: {crm-write, kb-search, runbooks}
│   └── 5× SpecialistAgent (Sonnet, diagnostic skill set)
└── EscalationTeam (workflow team)
    └── EscalateToHuman workflow (deterministic, audit-logged)
```

A ticket arrives at `SupportOrg`, routes to `TriageTeam`, runs `ClassifyTicket` (which calls a small classifier agent and branches to L1 or L2). The L2 agent, mid-conversation, calls the `RefundWorkflow` (deterministic, idempotent, audit-logged) as if it were a tool. If the refund exceeds policy, the workflow's `Human` step pauses and waits for approval, persisting state.

## Python Interface

The Rust framework exposes itself as a Python library via `pyo3`, supporting two distinct usage modes.

### Mode 1: Python as Host

Python drives the framework — defines harnesses, kicks off runs, awaits results. The hot path (actor scheduling, message routing, tool dispatch, inference batching, CUDA calls) stays in Rust.

```python
from akka_agents import Harness, Agent, ToolSet, ToolStrategy
from akka_agents.strategies import EmbeddingToolStrategy

coding = Harness.load("coding-harness", version="2.3.1")
result = await coding.run(
    task="fix the off-by-one in pagination",
    repo="/path/to/repo",
    budget={"tokens": 200_000, "wall_clock": "30m"},
)
```

`Harness.run` is a thin wrapper that submits to the Rust runtime over a tokio bridge and streams events back as an async iterator.

### Mode 2: Python as Guest

Custom strategies, tools, and agent logic in Python, executed inside the Rust actor system. Essential because most ML and domain code lives in Python — pandas, scikit-learn, custom embedding models, business logic.

```rust
pub struct PyActor {
    interpreter: PyInterpreter,
    handler: Py<PyAny>,
}

#[async_trait]
impl Tool for PyTool {
    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value> {
        self.py_actor.send(PyInvoke { args }).await?
    }
}
```

```python
@tool(toolset="finance")
def discounted_cash_flow(cashflows: list[float], rate: float) -> float:
    return sum(cf / (1 + rate) ** i for i, cf in enumerate(cashflows))

@strategy
class CustomToolStrategy(ToolStrategy):
    async def select(self, ctx, budget):
        ...
```

### GIL Containment

The crucial design choice: Python code never blocks the Rust scheduler. Every Python call dispatches to a `PyActor` with its own interpreter (subinterpreters where viable, separate processes where not), bounded inbox, and timeout. The GIL is contained within the actor; the rest of the system runs at native speed.

CPU-heavy Python releases the GIL through native extensions (numpy, torch). Inference calls short-circuit to the Rust CUDA/inference actors directly, so a Python-defined agent loop still gets native-speed model calls without crossing the FFI boundary on the hot path.

This makes the framework equally legitimate as a Rust crate and as a Python library, with the same harnesses, agents, workflows, and tool sets reachable from both.

## Cross-Cutting Concerns

### Backpressure

Every actor has a bounded inbox. Every cross-actor call is a bounded channel. Saturation propagates upstream automatically. For batched inference, the inference actor implements `Sink` so the runtime coalesces requests.

### Observability

Every strategy resolution, tool invocation, agent decision, workflow step, and harness iteration emits a structured event. Events feed traces, metrics, and replay. The same event stream powers debugging and eval-suite execution — a harness run can be replayed deterministically (modulo model nondeterminism) by re-feeding events.

### Budgets

`TokenBudget`, time budget, money budget, and iteration budget are passed through the call stack as first-class arguments. Strategies consume from shared budgets cooperatively. Policy at each org level can cap budgets that flow downward.

### Versioning

Tool sets, skills, agents, workflows, and harnesses all carry semantic versions. Eval suites tie versions to measured behavior. A registry holds published artifacts; consumers pin versions. Upgrades are deliberate, gated by regression tests.

## Summary

The architecture provides a single substrate — bounded, supervised, backpressured actors — with a single composition abstraction (`Callable`) layered into progressively higher-level units: strategies, tool sets, skills, agents, workflows, harnesses. The same hierarchical organization applies uniformly across all unit types, with policy inherited and narrowed at every boundary.

Agents give you adaptability. Workflows give you guarantees. Harnesses give you reproducibility. The Python interface gives you the ecosystem. Putting them all in the same compositional substrate lets you place the boundary between "figured out at runtime" and "decided at design time" exactly where each part of your system needs it — and move that boundary later without rewriting anything.
