---
name: atomr-agents-multi-agent
description: Use when wiring an `Org` / `Department` / `Team` topology, picking a routing strategy (`RoundRobin` / `LoadAware` / `CapabilityMatch`), or building a supervisor / swarm / network / hierarchical multi-agent pattern. Triggers on `Team { unit: OrgUnit { ... } }`, `swarm_loop(...)`, `HandoffTool::new(...)`, `Policy::narrow`, or porting a LangGraph supervisor.
---

# Multi-agent topologies in atomr-agents

`agents-org` provides the four-level hierarchy (`Org` →
`Department` → `Team` → unit) plus the four canonical multi-agent
patterns (supervisor / swarm / network / hierarchical), all built
from `Callable`, `OrgRoutingStrategy`, `Policy::narrow`, and
`RichTool::ToolReturn::Command(Handoff)`.

## Mental model

- Every level is a **`Callable`**. An `Org` containing a
  `Department` containing `Team`s is just nesting.
- Routing is per-level. Each `OrgUnit` carries an
  `Arc<dyn OrgRoutingStrategy>` that picks among `(label,
  CallableHandle)` children.
- Policy is **inherited and narrowed**. Children can only see
  toolsets / models the parent grants; numeric caps take the min.
- Memory is **namespaced**. Reads cascade outward (agent → team →
  org); writes are gated.
- Handoff is **a tool call**. `HandoffTool` returns
  `ToolReturn::Command(ToolControl::Handoff { target, payload })`;
  the surrounding pattern decides what "handoff" means.

## Picking a routing strategy

| Strategy | When |
|---|---|
| `RoundRobinRouter::new()` | load balancing across identical workers |
| `LoadAwareRouter::for_size(n)` | best-effort least-loaded child |
| `CapabilityMatchRouter` | route based on the input's `route` string field — supervisor pattern |

```rust
use atomr_agents_org::{CapabilityMatchRouter, RoundRobinRouter};
use std::sync::Arc;

let routing: Arc<dyn OrgRoutingStrategy> = Arc::new(CapabilityMatchRouter);
```

## Pattern 1: Supervisor

```rust
use std::sync::Arc;
use atomr_agents_callable::CallableHandle;
use atomr_agents_org::{CapabilityMatchRouter, OrgUnit, Team};
use atomr_agents_core::TeamId;
use atomr_agents_strategy::Policy;

let team = Team {
    id: TeamId::from("support"),
    unit: OrgUnit {
        label: "support".into(),
        children: vec![
            ("L1-frontline".into(),  frontline_agent),
            ("L2-specialist".into(), specialist_agent),
            ("escalation".into(),    escalation_workflow),
        ],
        routing: Arc::new(CapabilityMatchRouter),
        policy: Policy::default(),
        granted_toolsets: vec!["crm".into(), "kb".into()],
    },
};

// Triage agent's tool call embeds the route:
team.call(serde_json::json!({"route": "L2", "ticket": "abc-123"}), ctx).await?;
// CapabilityMatchRouter substring-matches "L2" against child labels;
// L2-specialist runs.
```

## Pattern 2: Swarm — peer-to-peer handoff

```rust
use std::collections::HashMap;
use atomr_agents_org::{ActiveAgent, swarm_loop};

let active = ActiveAgent::new("planner");
let mut agents: HashMap<String, CallableHandle> = HashMap::new();
agents.insert("planner".into(),  planner_agent_for(active.clone()));
agents.insert("executor".into(), executor_agent_for(active.clone()));
agents.insert("reviewer".into(), reviewer_agent_for(active.clone()));

let result = swarm_loop(
    &agents,
    &active,
    serde_json::json!({"task": "deploy-pipeline"}),
    /* max_iters */ 20,
).await?;
```

Each agent's tools include a `HandoffTool` that updates the
`ActiveAgent` slot on invocation. The loop terminates when an agent
returns `{"done": true}` or `max_iters` runs out.

## Pattern 3: Network — arbitrary handoffs

Same as swarm; the only difference is *who can hand off to whom* —
which the swarm helper doesn't constrain. An agent can hand off to
any peer:

```rust
agents.insert("a".into(), agent_handing_off_to(active.clone(), "b"));
agents.insert("b".into(), agent_handing_off_to(active.clone(), "a"));   // loops back
agents.insert("c".into(), terminating_agent());
```

## Pattern 4: Hierarchical — supervisors of supervisors

```rust
use atomr_agents_org::{Department, Org};
use atomr_agents_core::{DepartmentId, OrgId};

let dept = Department {
    id: DepartmentId::from("support"),
    unit: OrgUnit {
        label: "support".into(),
        children: vec![
            ("L1".into(), Arc::new(team_l1) as CallableHandle),
            ("L2".into(), Arc::new(team_l2) as CallableHandle),
        ],
        routing: Arc::new(CapabilityMatchRouter),
        policy: dept_policy,
        granted_toolsets: vec![],
    },
};

let org = Org {
    id: OrgId::from("co"),
    unit: OrgUnit {
        label: "root".into(),
        children: vec![("support".into(), Arc::new(dept) as CallableHandle)],
        routing: Arc::new(CapabilityMatchRouter),
        policy: org_policy,
        granted_toolsets: vec!["crm".into(), "kb".into(), "billing".into()],
    },
    parent_policy: None,
};
```

`org.call({route: "support"}, ctx)` runs the dept; the dept's
router picks `L1` or `L2`; that team's router picks an agent.

## Policy inheritance

```rust
use atomr_agents_strategy::Policy;

let parent = Policy {
    allowed_toolsets: vec!["a".into(), "b".into(), "c".into()],
    max_tokens_per_call: Some(10_000),
    allowed_models: vec!["gpt-4o".into(), "claude-3-opus".into()],
    ..Default::default()
};
let child = Policy {
    allowed_toolsets: vec!["b".into(), "d".into()],
    max_tokens_per_call: Some(2_000),
    allowed_models: vec![],
    ..Default::default()
};

let resolved = Policy::narrow(&parent, &child);
// resolved.allowed_toolsets == ["b"]
// resolved.max_tokens_per_call == Some(2_000)
// resolved.allowed_models == ["gpt-4o", "claude-3-opus"] (child empty inherits)
```

Empty child lists inherit; non-empty lists intersect with parent.
Numeric caps take min.

## Namespaced memory

```rust
use atomr_agents_org::NamespacedMemory;

let mem = NamespacedMemory::new(OrgId::from("co"), AgentId::from("a-1"))
    .with_team(TeamId::from("support-l2"))
    .with_team_write(true);
```

| Namespace | Reads | Writes |
|---|---|---|
| `Agent(self)` | yes | yes |
| `Team(self.team)` | yes | only when `allow_team_write = true` |
| `Org(self.org)` | yes (cascade on agent reads) | always denied |

## HandoffTool

```rust
use atomr_agents_tool::HandoffTool;

let handoff = HandoffTool::new("specialist");
// Tool name: "handoff_to_specialist"
// On invoke: ToolReturn::Command(ToolControl::Handoff { target: "specialist", payload })
```

The `target` and `payload` fields can be overridden per-call if
the model supplies them in tool args. The surrounding pattern
(supervisor / swarm) decides what to do with the `Handoff` value.

## Canonical references

- [`docs/multi-agent-patterns.md`](https://github.com/rustakka/atomr-agents/blob/main/docs/multi-agent-patterns.md)
- [`crates/org/src/patterns.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/org/src/patterns.rs) — supervisor / swarm / network / hierarchical tests
- [`crates/org/src/team.rs`](https://github.com/rustakka/atomr-agents/blob/main/crates/org/src/team.rs)

## Common mistakes

- **Forgetting the `route` field.** `CapabilityMatchRouter` reads
  `input.route` (string). Without it, you fall through to the first
  child.
- **Expanding policy in a child.** `Policy::narrow` intersects;
  there's no way to *add* a toolset the parent didn't grant.
- **Writing to org-level memory from an agent.**
  `NamespacedMemory::put(item with namespace=Org(_))` returns
  `PolicyDenied`. Promote the write to a workflow step at the org
  level.
- **`swarm_loop` without a termination signal.** Some agent must
  return `{"done": true}` or the loop runs to `max_iters` and
  returns the last output.
- **Hardcoding agent ids in tool descriptors.** Use a registry or
  config for handoff targets so a topology change doesn't ripple
  through tool docstrings.
