//! M7 — Gateway actor: multi-channel ingress, routes inbound messages
//! to per-agent ChatSessions. The actor surface is intentionally
//! minimal in v1; ChatSession objects are constructed eagerly via the
//! [`HostRuntime`] handle.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::chat::AgentRouter;
use crate::error::HostResult;
use crate::routes::{parse_agents_md, AgentsRoutingRules};
use crate::runtime::HostRuntime;

#[derive(Clone)]
pub struct Gateway {
    runtime: HostRuntime,
    router: Arc<AgentRouter>,
    sessions: Arc<RwLock<HashMap<(String, String), String>>>,
}

impl Gateway {
    pub fn new(runtime: HostRuntime, router: AgentRouter) -> Self {
        Self { runtime, router: Arc::new(router), sessions: Default::default() }
    }

    pub fn router(&self) -> Arc<AgentRouter> {
        self.router.clone()
    }

    pub fn route(&self, channel_id: &str, peer_id: &str) -> Option<String> {
        self.router.route(channel_id, peer_id)
    }

    pub async fn handle(&self, channel_id: &str, peer_id: &str, user_message: &str) -> HostResult<String> {
        let agent_id = self
            .router
            .route(channel_id, peer_id)
            .ok_or_else(|| crate::error::HostError::Gateway(format!(
                "no agent bound for channel={channel_id} peer={peer_id}"
            )))?;
        self.sessions
            .write()
            .insert((channel_id.to_string(), peer_id.to_string()), agent_id.clone());
        let handle = self.runtime.spawn_agent(&agent_id).await?;
        handle.preview(user_message).await
    }
}

/// Build a router from `<root>/AGENTS.md` + host config defaults.
pub fn build_router_from_rules(rules: AgentsRoutingRules, default_from_config: Option<String>) -> AgentRouter {
    let default_agent = rules.default_agent.or(default_from_config);
    let router = AgentRouter::new(default_agent);
    for (channel, agent) in rules.channel_pins {
        router.pin_channel(channel, agent);
    }
    for ((channel, peer), agent) in rules.peer_pins {
        router.pin_peer(channel, peer, agent);
    }
    router
}

pub fn load_agents_md(path: &std::path::Path) -> HostResult<AgentsRoutingRules> {
    if !path.is_file() {
        return Ok(AgentsRoutingRules::default());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::error::HostError::io(path, e))?;
    Ok(parse_agents_md(&text))
}
