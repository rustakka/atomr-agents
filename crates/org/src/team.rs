use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_callable::{Callable, CallableHandle};
use atomr_agents_core::{
    CallCtx, DepartmentId, OrgId, Result, TeamId, ToolSetId, Value,
};
use atomr_agents_strategy::Policy;

use crate::routing::OrgRoutingStrategy;

/// Common shape: every level holds children + routing + policy +
/// granted tool sets.
pub struct OrgUnit {
    pub label: String,
    pub children: Vec<(String, CallableHandle)>,
    pub routing: Arc<dyn OrgRoutingStrategy>,
    pub policy: Policy,
    pub granted_toolsets: Vec<ToolSetId>,
}

impl OrgUnit {
    fn resolve_policy(&self, parent: Option<&Policy>) -> Policy {
        match parent {
            Some(p) => Policy::narrow(p, &self.policy),
            None => self.policy.clone(),
        }
    }

    async fn route_and_call(
        &self,
        input: Value,
        ctx: CallCtx,
        request_label: &str,
    ) -> Result<Value> {
        let child = self.routing.pick(&self.children, request_label).await?;
        child.call(input, ctx).await
    }
}

pub struct Team {
    pub id: TeamId,
    pub unit: OrgUnit,
}

#[async_trait]
impl Callable for Team {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let label = input
            .get("route")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        self.unit.route_and_call(input, ctx, &label).await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}

pub struct Department {
    pub id: DepartmentId,
    pub unit: OrgUnit,
}

#[async_trait]
impl Callable for Department {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let label = input
            .get("route")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        self.unit.route_and_call(input, ctx, &label).await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}

pub struct Org {
    pub id: OrgId,
    pub unit: OrgUnit,
    pub parent_policy: Option<Policy>,
}

impl Org {
    pub fn resolved_policy(&self) -> Policy {
        self.unit.resolve_policy(self.parent_policy.as_ref())
    }
}

#[async_trait]
impl Callable for Org {
    async fn call(&self, input: Value, ctx: CallCtx) -> Result<Value> {
        let label = input
            .get("route")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        self.unit.route_and_call(input, ctx, &label).await
    }

    fn label(&self) -> &str {
        self.id.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::routing::{CapabilityMatchRouter, RoundRobinRouter};
    use atomr_agents_callable::FnCallable;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};
    use std::time::Duration;

    fn ctx() -> CallCtx {
        CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(0.10),
            iterations: IterationBudget::new(5),
            trace: vec![],
        }
    }

    fn child_returning(s: &'static str) -> CallableHandle {
        Arc::new(FnCallable::labeled(s, move |_v: Value, _ctx| async move {
            Ok(serde_json::json!(s))
        }))
    }

    #[tokio::test]
    async fn round_robin_alternates() {
        let team = Team {
            id: TeamId::from("t-1"),
            unit: OrgUnit {
                label: "L1".into(),
                children: vec![
                    ("a".into(), child_returning("alice")),
                    ("b".into(), child_returning("bob")),
                ],
                routing: Arc::new(RoundRobinRouter::new()),
                policy: Policy::default(),
                granted_toolsets: vec![],
            },
        };
        let v1 = team.call(serde_json::json!({}), ctx()).await.unwrap();
        let v2 = team.call(serde_json::json!({}), ctx()).await.unwrap();
        let v3 = team.call(serde_json::json!({}), ctx()).await.unwrap();
        assert_eq!(v1, serde_json::json!("alice"));
        assert_eq!(v2, serde_json::json!("bob"));
        assert_eq!(v3, serde_json::json!("alice"));
    }

    #[tokio::test]
    async fn capability_match_picks_label() {
        let team = Team {
            id: TeamId::from("support"),
            unit: OrgUnit {
                label: "support".into(),
                children: vec![
                    ("L1".into(), child_returning("frontline")),
                    ("L2".into(), child_returning("specialist")),
                ],
                routing: Arc::new(CapabilityMatchRouter),
                policy: Policy::default(),
                granted_toolsets: vec![],
            },
        };
        let r = team.call(serde_json::json!({"route": "l2"}), ctx()).await.unwrap();
        assert_eq!(r, serde_json::json!("specialist"));
    }

    #[tokio::test]
    async fn policy_narrows_downward_through_org() {
        let parent = Policy {
            allowed_toolsets: vec!["a".into(), "b".into(), "c".into()],
            max_tokens_per_call: Some(10_000),
            ..Default::default()
        };
        let org = Org {
            id: OrgId::from("o-1"),
            unit: OrgUnit {
                label: "org".into(),
                children: vec![],
                routing: Arc::new(RoundRobinRouter::new()),
                policy: Policy {
                    allowed_toolsets: vec!["a".into(), "b".into()],
                    max_tokens_per_call: Some(2_000),
                    ..Default::default()
                },
                granted_toolsets: vec![],
            },
            parent_policy: Some(parent),
        };
        let r = org.resolved_policy();
        let names: Vec<&str> = r.allowed_toolsets.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(r.max_tokens_per_call, Some(2_000));
    }
}
