# Multi-agent patterns

The `agents-org` crate provides the org / department / team
hierarchy. The four canonical multi-agent patterns —
**supervisor**, **swarm**, **network**, **hierarchical** — all
compose from primitives already in the workspace: `Callable`,
`OrgRoutingStrategy`, `Policy::narrow`, `RichTool`'s
`ToolReturn::Command(Handoff)`, and (for swarm) a shared
`ActiveAgent` slot.

## Org / Department / Team

```rust
use atomr_agents_org::{Department, Org, OrgUnit, RoundRobinRouter, Team};
use atomr_agents_strategy::Policy;

let team = Team {
    id: TeamId::from("L1"),
    unit: OrgUnit {
        label: "L1".into(),
        children: vec![
            ("agent-a".into(), agent_a),
            ("agent-b".into(), agent_b),
        ],
        routing: Arc::new(RoundRobinRouter::new()),
        policy: Policy::default(),
        granted_toolsets: vec![],
    },
};
```

Each level (`Org` → `Department` → `Team`) is a `Callable` itself —
which means **a department can contain teams or other departments
indistinguishably**, and the whole hierarchy plugs into a `Pipeline`
or workflow step like any other callable.

## Routing strategies

| Strategy | Behavior |
|---|---|
| `RoundRobinRouter` | rotate through children; `AtomicUsize` cursor |
| `LoadAwareRouter::for_size(n)` | best-effort least-recently-picked counter |
| `CapabilityMatchRouter` | substring match `request.route` against child labels |

The dispatch input is a `Value`. Routers read `route` (a string) by
convention; `CapabilityMatchRouter` is the workhorse for supervisor
patterns:

```rust
let v = team.call(serde_json::json!({"route": "L2", "q": "complex"}), ctx).await?;
// CapabilityMatchRouter picks the child whose label contains "L2".
```

## Policy::narrow

Policy inherits and *narrows* downward — a child can never expand a
parent's grants:

```rust
use atomr_agents_strategy::Policy;

let parent = Policy {
    allowed_toolsets: vec!["a".into(), "b".into(), "c".into()],
    max_tokens_per_call: Some(10_000),
    ..Default::default()
};
let child = Policy {
    allowed_toolsets: vec!["b".into(), "d".into()],   // d is *not* allowed
    max_tokens_per_call: Some(2_000),
    ..Default::default()
};
let resolved = Policy::narrow(&parent, &child);
// resolved.allowed_toolsets == ["b"]
// resolved.max_tokens_per_call == Some(2_000)
```

`Org::resolved_policy()` walks the parent chain; `Department` and
`Team` can do the same by composing `Policy::narrow` against their
parent's resolved policy at construction.

## Pattern 1: Supervisor

A central `Team` (or `Org`) routes incoming requests to specialist
children based on `CapabilityMatchRouter`. This is the canonical
LangGraph supervisor pattern:

```rust
let support = Team {
    id: "support".into(),
    unit: OrgUnit {
        label: "support".into(),
        children: vec![
            ("L1-frontline".into(),  frontline_agent),
            ("L2-specialist".into(), specialist_agent),
            ("escalation".into(),    escalation_workflow),
        ],
        routing: Arc::new(CapabilityMatchRouter),
        policy: support_policy,
        granted_toolsets: vec!["crm".into(), "kb".into()],
    },
};

// Triage agent's tool calls embed the route hint.
support.call(serde_json::json!({"route": "L2", "ticket": ticket_id}), ctx).await?;
```

## Pattern 2: Swarm — peer-to-peer handoff

The swarm pattern shares an `ActiveAgent` slot in state. Each agent
decides whether to set the slot to a peer's id; the loop
re-dispatches:

```rust
use atomr_agents_org::{ActiveAgent, swarm_loop};

let active = ActiveAgent::new("planner");
let mut agents: HashMap<String, CallableHandle> = HashMap::new();
agents.insert("planner".into(),  planner_with_handoff(active.clone()));
agents.insert("executor".into(), executor_with_handoff(active.clone()));

let result = swarm_loop(&agents, &active, initial_input, /* max_iters */ 20).await?;
```

Each agent's tools include a `HandoffTool { default_target:
"executor" }`; when the agent invokes it, the tool returns
`ToolReturn::Command(ToolControl::Handoff { target: "executor",
payload })`, the agent sets `active.set("executor")`, and
`swarm_loop` re-dispatches.

## Pattern 3: Network — arbitrary handoffs

Same shape as swarm; the only difference is that *any* agent can
hand off to *any* other (including jumping back). The
`swarm_loop` helper handles this — there's no topology constraint
beyond the shared `ActiveAgent` slot.

```rust
agents.insert("a".into(), agent_a_handoff_to(active.clone(), "b"));
agents.insert("b".into(), agent_b_handoff_to(active.clone(), "c"));
agents.insert("c".into(), agent_c_handoff_to(active.clone(), "a"));   // loops back
// swarm_loop terminates when an agent returns {"done": true}.
```

## Pattern 4: Hierarchical — supervisors of supervisors

Build it by nesting: an `Org` whose children are `Department`s
whose children are `Team`s. Policy narrows at every step; routing
runs at every level:

```rust
let team_l1: CallableHandle = Arc::new(team_l1);
let team_l2: CallableHandle = Arc::new(team_l2);

let support_dept = Department {
    id: "support".into(),
    unit: OrgUnit {
        label: "support".into(),
        children: vec![("L1".into(), team_l1), ("L2".into(), team_l2)],
        routing: Arc::new(CapabilityMatchRouter),
        policy: dept_policy,
        granted_toolsets: vec![],
    },
};

let org = Org {
    id: "support-org".into(),
    unit: OrgUnit {
        label: "root".into(),
        children: vec![("support".into(), Arc::new(support_dept))],
        routing: Arc::new(CapabilityMatchRouter),
        policy: org_policy,
        granted_toolsets: vec!["crm".into(), "kb".into(), "billing".into()],
    },
    parent_policy: None,
};

org.call(serde_json::json!({"route": "support"}), ctx).await?;
```

## Namespaced memory

`NamespacedMemory` enforces the read/write rules across a hierarchy:

| Namespace | Agent reads | Agent writes |
|---|---|---|
| `Agent(self)` | yes | yes |
| `Team(self.team)` | yes (cascade) | only if `allow_team_write = true` |
| `Org(self.org)` | yes (cascade) | always denied |

```rust
use atomr_agents_org::NamespacedMemory;

let mem = NamespacedMemory::new(OrgId::from("o"), AgentId::from("a-1"))
    .with_team(TeamId::from("t-1"))
    .with_team_write(true);

// `mem.put(item with namespace=Agent(a-1))` works.
// `mem.put(item with namespace=Team(t-1))` works because allow_team_write.
// `mem.put(item with namespace=Org(o))` returns PolicyDenied.
```

## HandoffTool

`HandoffTool` is the canonical helper for supervisor + swarm + network
flows:

```rust
use atomr_agents_tool::HandoffTool;

let handoff = HandoffTool::new("specialist");
// Tool name: "handoff_to_specialist"
// Tool descriptor: schema { target: string, payload: any }
// Invoke: returns ToolReturn::Command(ToolControl::Handoff { target, payload })
```

The agent layer translates `ToolControl::Handoff` into the
appropriate routing call (e.g. setting the `ActiveAgent` slot in a
swarm, or invoking the named team child in a supervisor topology).

## Where to go from here

- [Architecture](architecture.md) — where org / team / department
  fit in the crate stack.
- [Agent pipeline](agent-pipeline.md) — how `RichTool::invoke_rich`
  connects with `ToolReturn::Command` for handoff flows.
- [State and checkpointing](state-and-checkpointing.md) — durable
  shared state (the `ActiveAgent` slot in particular).
