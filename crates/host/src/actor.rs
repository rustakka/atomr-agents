//! `AgentHostActor` — one actor per loaded agent. Owns the
//! [`LoadedAgent`] state and replies to lifecycle / identity / chat
//! preview queries. Heavier turn execution layers on in M2.

use async_trait::async_trait;
use tokio::sync::oneshot;

use atomr_core::actor::{Actor, Context};

use crate::loader::LoadedAgent;

/// User-facing messages the actor handles.
pub enum AgentHostMsg {
    /// Reply with a short identity blob — agent id, model, persona
    /// identity if any.
    Identify(oneshot::Sender<IdentitySnapshot>),
    /// Reply with a structural snapshot (counts of skills / rules /
    /// memory facts).
    Status(oneshot::Sender<StatusSnapshot>),
    /// Render a deterministic preview reply (used by tests and the
    /// no-LLM happy path). M2 layers a real chat callable on top.
    Preview {
        user_message: String,
        reply: oneshot::Sender<String>,
    },
    /// Replace the entire loaded state — used by hot reload.
    Reload(Box<LoadedAgent>),
}

#[derive(Debug, Clone)]
pub struct IdentitySnapshot {
    pub agent_id: String,
    pub model: String,
    pub persona_identity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StatusSnapshot {
    pub agent_id: String,
    pub model: String,
    pub persona_identity: Option<String>,
    pub rules_count: usize,
    pub memory_facts_count: usize,
    pub user_profile_len: usize,
    pub skills_count: usize,
}

pub struct AgentHostActor {
    pub loaded: LoadedAgent,
}

impl AgentHostActor {
    pub fn new(loaded: LoadedAgent) -> Self {
        Self { loaded }
    }

    fn identity(&self) -> IdentitySnapshot {
        IdentitySnapshot {
            agent_id: self.loaded.spec.id.to_string(),
            model: self.loaded.spec.model.clone(),
            persona_identity: self.loaded.persona.as_ref().map(|p| p.identity.clone()),
        }
    }

    fn status(&self) -> StatusSnapshot {
        StatusSnapshot {
            agent_id: self.loaded.spec.id.to_string(),
            model: self.loaded.spec.model.clone(),
            persona_identity: self.loaded.persona.as_ref().map(|p| p.identity.clone()),
            rules_count: self.loaded.rules.len(),
            memory_facts_count: self.loaded.memory_facts.len(),
            user_profile_len: self.loaded.user_profile.len(),
            skills_count: self.loaded.skill_set.skills.len(),
        }
    }

    /// Deterministic preview reply — used by tests and the no-LLM path.
    pub fn preview(&self, user_message: &str) -> String {
        crate::chat::render_chat_preview(&self.loaded, user_message)
    }
}

#[async_trait]
impl Actor for AgentHostActor {
    type Msg = AgentHostMsg;

    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: Self::Msg) {
        match msg {
            AgentHostMsg::Identify(tx) => {
                let _ = tx.send(self.identity());
            }
            AgentHostMsg::Status(tx) => {
                let _ = tx.send(self.status());
            }
            AgentHostMsg::Preview { user_message, reply } => {
                let resp = self.preview(&user_message);
                let _ = reply.send(resp);
            }
            AgentHostMsg::Reload(new) => {
                self.loaded = *new;
            }
        }
    }
}
