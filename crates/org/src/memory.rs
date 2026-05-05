use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, AgentId, MemoryItem, MemoryNamespace, OrgId, Result, TeamId};
use atomr_agents_memory::{InMemoryStore, MemoryStore};

/// Memory namespaced by the org/team/agent triple. Reads cascade
/// outward (agent → team → org). Writes are gated: an agent can write
/// to its own namespace and (with permission) to a team scratchpad,
/// but never to org-level memory.
pub struct NamespacedMemory {
    pub org: OrgId,
    pub team: Option<TeamId>,
    pub agent: AgentId,
    pub allow_team_write: bool,
    pub backing: Arc<dyn MemoryStore>,
}

impl NamespacedMemory {
    pub fn new(org: OrgId, agent: AgentId) -> Self {
        Self {
            org,
            team: None,
            agent,
            allow_team_write: false,
            backing: Arc::new(InMemoryStore::new()),
        }
    }

    pub fn with_team(mut self, team: TeamId) -> Self {
        self.team = Some(team);
        self
    }

    pub fn with_team_write(mut self, allow: bool) -> Self {
        self.allow_team_write = allow;
        self
    }
}

/// Read-only view for callers that want to enumerate.
pub struct OrgMemoryView<'a>(&'a NamespacedMemory);

#[async_trait]
impl MemoryStore for NamespacedMemory {
    async fn put(&self, item: MemoryItem) -> Result<()> {
        match &item.namespace {
            MemoryNamespace::Agent(id) if id.as_str() == self.agent.as_str() => self.backing.put(item).await,
            MemoryNamespace::Team(id) => {
                if !self.allow_team_write || self.team.as_ref().map(|t| t.as_str()) != Some(id.as_str()) {
                    return Err(AgentError::PolicyDenied(format!(
                        "write to team namespace {} denied",
                        id.as_str()
                    )));
                }
                self.backing.put(item).await
            }
            MemoryNamespace::Org(_) => Err(AgentError::PolicyDenied(
                "agents cannot write to org-level memory".into(),
            )),
            MemoryNamespace::Agent(other) => Err(AgentError::PolicyDenied(format!(
                "agent {} cannot write to {}",
                self.agent.as_str(),
                other.as_str()
            ))),
        }
    }

    async fn list(&self, namespace: &MemoryNamespace, limit: usize) -> Result<Vec<MemoryItem>> {
        // Reads cascade — caller may ask for any of {agent, team, org}.
        // We delegate to the backing store; the in-memory store only
        // returns items it owns, so cascade requires multiple lookups.
        let mut out = self.backing.list(namespace, limit).await?;
        // For the v0 in-memory case, we additionally allow agent
        // queries to also see team/org reads.
        if matches!(namespace, MemoryNamespace::Agent(_)) {
            if let Some(team) = &self.team {
                let team_items = self
                    .backing
                    .list(&MemoryNamespace::Team(team.clone()), limit)
                    .await?;
                out.extend(team_items);
            }
            let org_items = self
                .backing
                .list(&MemoryNamespace::Org(self.org.clone()), limit)
                .await?;
            out.extend(org_items);
            out.sort_by_key(|i| std::cmp::Reverse(i.timestamp_ms));
            out.truncate(limit);
        }
        Ok(out)
    }
}

#[allow(dead_code)]
fn _view_alive(_v: OrgMemoryView<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::MemoryKind;

    fn item(ns: MemoryNamespace, id: &str, ts: i64) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            kind: MemoryKind::Episodic,
            namespace: ns,
            payload: serde_json::json!({"x": id}),
            timestamp_ms: ts,
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn agent_cannot_write_to_org_memory() {
        let m = NamespacedMemory::new(OrgId::from("o-1"), AgentId::from("a-1"));
        let r = m
            .put(item(MemoryNamespace::Org(OrgId::from("o-1")), "x", 1))
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn agent_can_write_to_team_when_allowed() {
        let m = NamespacedMemory::new(OrgId::from("o-1"), AgentId::from("a-1"))
            .with_team(TeamId::from("t-1"))
            .with_team_write(true);
        let r = m
            .put(item(MemoryNamespace::Team(TeamId::from("t-1")), "x", 1))
            .await;
        assert!(r.is_ok());
    }
}
