//! Chat preview + (M2) `ChatSession`/`AgentRouter` placeholders.
//!
//! The deterministic preview lives here so [`crate::actor::AgentHostActor`]
//! can use it without depending on M2-specific types.

use std::collections::HashMap;

use parking_lot::RwLock;
use std::sync::Arc;

use crate::loader::LoadedAgent;

/// Render the deterministic preview reply for a user message.
///
/// The shape mirrors the pre-port Python implementation so existing
/// suites and humans recognize the signature.
pub fn render_chat_preview(loaded: &LoadedAgent, user_message: &str) -> String {
    let persona_identity = loaded
        .persona
        .as_ref()
        .map(|p| p.identity.clone())
        .unwrap_or_else(|| loaded.spec.id.to_string());
    let rules_count = loaded.rules.len();
    let facts_count = loaded.memory_facts.len();
    let skill_count = loaded.skill_set.skills.len();
    let style_tone = loaded
        .persona
        .as_ref()
        .and_then(|p| p.style.tone.clone())
        .unwrap_or_else(|| "neutral".to_string());

    format!(
        "[{persona_identity} | model={model} | rules:{rules_count} memory facts:{facts_count} skills:{skill_count} tone:{style_tone}]\nuser: {user_message}",
        model = loaded.spec.model,
    )
}

/// Router maps (channel_id, peer_id) tuples and channel/peer pins to
/// agent ids. The M2-quality routing logic lives in [`crate::gateway`]
/// — this struct stays here so it can be used standalone without the
/// full Gateway actor.
#[derive(Debug, Clone, Default)]
pub struct AgentRouter {
    default_agent: Option<String>,
    channel_pins: Arc<RwLock<HashMap<String, String>>>,
    peer_pins: Arc<RwLock<HashMap<(String, String), String>>>,
}

impl AgentRouter {
    pub fn new(default_agent: Option<String>) -> Self {
        Self {
            default_agent,
            channel_pins: Default::default(),
            peer_pins: Default::default(),
        }
    }

    pub fn pin_channel(&self, channel_id: impl Into<String>, agent_id: impl Into<String>) {
        self.channel_pins.write().insert(channel_id.into(), agent_id.into());
    }

    pub fn pin_peer(
        &self,
        channel_id: impl Into<String>,
        peer_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) {
        self.peer_pins
            .write()
            .insert((channel_id.into(), peer_id.into()), agent_id.into());
    }

    pub fn route(&self, channel_id: &str, peer_id: &str) -> Option<String> {
        if let Some(a) = self.peer_pins.read().get(&(channel_id.to_string(), peer_id.to_string())) {
            return Some(a.clone());
        }
        if let Some(a) = self.channel_pins.read().get(channel_id) {
            return Some(a.clone());
        }
        self.default_agent.clone()
    }

    pub fn default_agent(&self) -> Option<&str> {
        self.default_agent.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_precedence() {
        let r = AgentRouter::new(Some("default".into()));
        assert_eq!(r.route("cli", "u1").as_deref(), Some("default"));
        r.pin_channel("cli", "support");
        assert_eq!(r.route("cli", "u1").as_deref(), Some("support"));
        r.pin_peer("cli", "u1", "alpha");
        assert_eq!(r.route("cli", "u1").as_deref(), Some("alpha"));
        assert_eq!(r.route("cli", "u2").as_deref(), Some("support"));
        assert_eq!(r.route("discord", "u1").as_deref(), Some("default"));
    }
}
