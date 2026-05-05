//! Reference multi-agent patterns: supervisor, swarm, network,
//! hierarchical. Each is built from primitives already in the
//! workspace; the tests below exercise them end-to-end against
//! deterministic stub agents.

use std::sync::Arc;

use atomr_agents_callable::{Callable, CallableHandle};
use atomr_agents_core::{
    CallCtx, IterationBudget, MoneyBudget, Result, TimeBudget, TokenBudget, Value,
};

/// Identifies the agent the next turn should target. Multi-agent
/// patterns set this in shared state; routers consult it to dispatch.
#[derive(Clone)]
pub struct ActiveAgent(pub Arc<parking_lot::Mutex<String>>);

impl ActiveAgent {
    pub fn new(initial: impl Into<String>) -> Self {
        Self(Arc::new(parking_lot::Mutex::new(initial.into())))
    }
    pub fn get(&self) -> String {
        self.0.lock().clone()
    }
    pub fn set(&self, v: impl Into<String>) {
        *self.0.lock() = v.into();
    }
}

/// Build a default `CallCtx` for these reference patterns.
pub fn default_ctx() -> CallCtx {
    CallCtx {
        agent_id: None,
        tokens: TokenBudget::new(8192),
        time: TimeBudget::new(std::time::Duration::from_secs(30)),
        money: MoneyBudget::from_usd(1.0),
        iterations: IterationBudget::new(16),
        trace: vec![],
    }
}

/// Run the swarm/network turn loop until the active agent emits
/// `{"done": true}`. The loop calls the active agent, lets it (or its
/// tools, in production) update `active`, then re-dispatches.
pub async fn swarm_loop(
    agents: &std::collections::HashMap<String, CallableHandle>,
    active: &ActiveAgent,
    initial_input: Value,
    max_iters: u32,
) -> Result<Value> {
    let mut current = initial_input;
    for _ in 0..max_iters {
        let key = active.get();
        let handle = agents.get(&key).ok_or_else(|| {
            atomr_agents_core::AgentError::Workflow(format!("unknown agent: {key}"))
        })?;
        current = handle.call(current, default_ctx()).await?;
        if current.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(current);
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{CapabilityMatchRouter, OrgRoutingStrategy, RoundRobinRouter};
    use crate::team::{Department, Org, OrgUnit, Team};
    use atomr_agents_callable::FnCallable;
    use atomr_agents_core::{DepartmentId, OrgId, TeamId};
    use atomr_agents_strategy::Policy;
    use std::collections::HashMap;

    fn frontline() -> CallableHandle {
        Arc::new(FnCallable::labeled("L1", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({"answer": "frontline"}))
        }))
    }
    fn specialist() -> CallableHandle {
        Arc::new(FnCallable::labeled("L2", |_v: Value, _ctx| async move {
            Ok(serde_json::json!({"answer": "specialist"}))
        }))
    }

    /// Supervisor pattern: a `Team` with `CapabilityMatchRouter`
    /// delegates to specialist agents based on the request.route.
    #[tokio::test]
    async fn supervisor_routes_by_capability() {
        let team = Team {
            id: TeamId::from("support"),
            unit: OrgUnit {
                label: "support".into(),
                children: vec![("L1".into(), frontline()), ("L2".into(), specialist())],
                routing: Arc::new(CapabilityMatchRouter),
                policy: Policy::default(),
                granted_toolsets: vec![],
            },
        };
        let v = team
            .call(serde_json::json!({"route": "L2", "q": "complex"}), default_ctx())
            .await
            .unwrap();
        assert_eq!(v["answer"], "specialist");
    }

    /// Swarm pattern: peer-to-peer handoff. The active-agent slot is
    /// the shared state; each agent decides whether to set it.
    #[tokio::test]
    async fn swarm_handoff_via_active_agent_slot() {
        let active = ActiveAgent::new("planner");
        let active_clone1 = active.clone();
        let active_clone2 = active.clone();
        let mut agents: HashMap<String, CallableHandle> = HashMap::new();
        agents.insert(
            "planner".into(),
            Arc::new(FnCallable::labeled("planner", move |_v: Value, _ctx| {
                let active = active_clone1.clone();
                async move {
                    active.set("executor");
                    Ok(serde_json::json!({"phase": "plan"}))
                }
            })),
        );
        agents.insert(
            "executor".into(),
            Arc::new(FnCallable::labeled("executor", move |_v: Value, _ctx| {
                let active = active_clone2.clone();
                async move {
                    active.set("done");
                    Ok(serde_json::json!({"phase": "exec", "done": true}))
                }
            })),
        );
        let r = swarm_loop(&agents, &active, serde_json::json!({}), 10).await.unwrap();
        assert_eq!(r["phase"], "exec");
        assert_eq!(r["done"], true);
    }

    /// Network pattern: any agent can route to any other; the
    /// `swarm_loop` helper is the same. Tested by an agent that
    /// jumps backward.
    #[tokio::test]
    async fn network_allows_arbitrary_handoffs() {
        let active = ActiveAgent::new("a");
        let calls = Arc::new(parking_lot::Mutex::new(Vec::<&'static str>::new()));
        let make_agent = |label: &'static str, next: &'static str, finish: bool| -> CallableHandle {
            let calls = calls.clone();
            let active = active.clone();
            Arc::new(FnCallable::labeled(label, move |_v: Value, _ctx| {
                let calls = calls.clone();
                let active = active.clone();
                async move {
                    calls.lock().push(label);
                    active.set(next);
                    Ok(if finish {
                        serde_json::json!({"done": true})
                    } else {
                        serde_json::json!({})
                    })
                }
            }))
        };
        let mut agents: HashMap<String, CallableHandle> = HashMap::new();
        agents.insert("a".into(), make_agent("a", "b", false));
        agents.insert("b".into(), make_agent("b", "a", false));
        // Loop ends after a → b → a (3 iters): we'll mark a-second-time as done.
        agents.insert(
            "a".into(),
            {
                let calls = calls.clone();
                let active = active.clone();
                Arc::new(FnCallable::labeled("a", move |_v: Value, _ctx| {
                    let calls = calls.clone();
                    let active = active.clone();
                    async move {
                        let mut c = calls.lock();
                        c.push("a");
                        let n = c.len();
                        drop(c);
                        if n >= 3 {
                            return Ok(serde_json::json!({"done": true}));
                        }
                        active.set("b");
                        Ok(serde_json::json!({}))
                    }
                }))
            },
        );
        let _ = swarm_loop(&agents, &active, serde_json::json!({}), 10).await.unwrap();
        let trace = calls.lock().clone();
        assert!(trace.iter().filter(|x| **x == "a").count() >= 2);
    }

    /// Hierarchical pattern: an `Org` containing a `Department`
    /// containing `Team`s. Policy narrows downward at each level.
    #[tokio::test]
    async fn hierarchical_org_dept_team_routes_through() {
        let team = Team {
            id: TeamId::from("L1"),
            unit: OrgUnit {
                label: "L1".into(),
                children: vec![("a".into(), frontline()), ("b".into(), frontline())],
                routing: Arc::new(RoundRobinRouter::new()),
                policy: Policy::default(),
                granted_toolsets: vec![],
            },
        };
        let team_h: CallableHandle = Arc::new(team);
        let dept = Department {
            id: DepartmentId::from("dept"),
            unit: OrgUnit {
                label: "dept".into(),
                children: vec![("L1".into(), team_h)],
                routing: Arc::new(CapabilityMatchRouter),
                policy: Policy::default(),
                granted_toolsets: vec![],
            },
        };
        let dept_h: CallableHandle = Arc::new(dept);
        let org = Org {
            id: OrgId::from("org"),
            unit: OrgUnit {
                label: "root".into(),
                children: vec![("dept".into(), dept_h)],
                routing: Arc::new(CapabilityMatchRouter),
                policy: Policy {
                    allowed_models: vec!["a".into(), "b".into()],
                    ..Default::default()
                },
                granted_toolsets: vec![],
            },
            parent_policy: Some(Policy {
                allowed_models: vec!["a".into(), "b".into(), "c".into()],
                ..Default::default()
            }),
        };
        let v = org
            .call(serde_json::json!({"route": "dept"}), default_ctx())
            .await
            .unwrap();
        assert_eq!(v["answer"], "frontline");
        let resolved = org.resolved_policy();
        assert_eq!(resolved.allowed_models, vec!["a", "b"]);
    }
}
