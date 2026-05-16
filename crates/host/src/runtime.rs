//! `HostRuntime` — owns the [`atomr_core::actor::ActorSystem`] and a
//! registry of running [`crate::actor::AgentHostActor`]s keyed by
//! agent id.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::oneshot;

use atomr_config::Config;
use atomr_core::actor::{ActorRef, ActorSystem, Props};

use crate::actor::{AgentHostActor, AgentHostMsg, IdentitySnapshot, StatusSnapshot};
use crate::config::HostConfig;
use crate::error::{HostError, HostResult};
use crate::loader::{AgentLoader, LoadedAgent};

/// Handle to an [`AgentHostActor`] running inside a [`HostRuntime`].
#[derive(Clone)]
pub struct AgentHandle {
    pub agent_id: String,
    pub actor: ActorRef<AgentHostMsg>,
}

impl AgentHandle {
    pub async fn identify(&self) -> HostResult<IdentitySnapshot> {
        let (tx, rx) = oneshot::channel();
        self.actor.tell(AgentHostMsg::Identify(tx));
        rx.await
            .map_err(|_| HostError::ActorSystem(format!("agent {} dropped before reply", self.agent_id)))
    }

    pub async fn status(&self) -> HostResult<StatusSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.actor.tell(AgentHostMsg::Status(tx));
        rx.await
            .map_err(|_| HostError::ActorSystem(format!("agent {} dropped before reply", self.agent_id)))
    }

    pub async fn preview(&self, user_message: impl Into<String>) -> HostResult<String> {
        let (tx, rx) = oneshot::channel();
        self.actor.tell(AgentHostMsg::Preview {
            user_message: user_message.into(),
            reply: tx,
        });
        rx.await
            .map_err(|_| HostError::ActorSystem(format!("agent {} dropped before reply", self.agent_id)))
    }

    pub fn reload(&self, loaded: LoadedAgent) {
        self.actor.tell(AgentHostMsg::Reload(Box::new(loaded)));
    }

    pub fn stop(&self) {
        self.actor.stop();
    }
}

/// Owns the actor system and the per-agent registry.
#[derive(Clone)]
pub struct HostRuntime {
    inner: Arc<HostRuntimeInner>,
}

struct HostRuntimeInner {
    config: HostConfig,
    system: ActorSystem,
    agents: RwLock<HashMap<String, AgentHandle>>,
}

impl HostRuntime {
    /// Build a runtime rooted at `config`. Boots a fresh actor system
    /// named after the host root.
    pub async fn start(config: HostConfig) -> HostResult<Self> {
        let system_name = format!(
            "atomr-host-{}",
            config
                .paths
                .root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("default")
        );
        let system = ActorSystem::create(system_name, Config::empty())
            .await
            .map_err(|e| HostError::ActorSystem(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(HostRuntimeInner {
                config,
                system,
                agents: RwLock::new(HashMap::new()),
            }),
        })
    }

    pub fn config(&self) -> &HostConfig {
        &self.inner.config
    }

    pub fn system(&self) -> &ActorSystem {
        &self.inner.system
    }

    pub fn loader(&self) -> AgentLoader {
        AgentLoader::new(self.inner.config.clone())
    }

    /// Look up a running handle without spawning.
    pub fn lookup(&self, agent_id: &str) -> Option<AgentHandle> {
        self.inner.agents.read().get(agent_id).cloned()
    }

    /// Spawn — or return the running handle for — `agent_id`.
    pub async fn spawn_agent(&self, agent_id: &str) -> HostResult<AgentHandle> {
        if let Some(existing) = self.lookup(agent_id) {
            return Ok(existing);
        }
        let loaded = self.loader().load(agent_id)?;
        self.spawn_loaded(loaded).await
    }

    /// Spawn an actor with the provided loaded agent (used by hot-reload paths).
    pub async fn spawn_loaded(&self, loaded: LoadedAgent) -> HostResult<AgentHandle> {
        let agent_id = loaded.spec.id.to_string();
        let factory_state = loaded.clone();
        let props = Props::create(move || AgentHostActor::new(factory_state.clone()));
        let actor = self
            .inner
            .system
            .actor_of(props, &format!("agent-{}", sanitize(&agent_id)))
            .map_err(|e| HostError::ActorSystem(e.to_string()))?;
        let handle = AgentHandle {
            agent_id: agent_id.clone(),
            actor,
        };
        self.inner.agents.write().insert(agent_id, handle.clone());
        Ok(handle)
    }

    /// Stop a running agent. No-op when not present.
    pub fn stop_agent(&self, agent_id: &str) {
        if let Some(handle) = self.inner.agents.write().remove(agent_id) {
            handle.stop();
        }
    }

    /// Reload an agent's state from disk and push the new state into
    /// its running actor (or spawn fresh if not running).
    pub async fn reload(&self, agent_id: &str) -> HostResult<AgentHandle> {
        let loaded = self.loader().load(agent_id)?;
        if let Some(existing) = self.lookup(agent_id) {
            existing.reload(loaded);
            return Ok(existing);
        }
        self.spawn_loaded(loaded).await
    }

    /// Stop the runtime — all child actors terminate.
    pub async fn shutdown(self) {
        for (_, handle) in self.inner.agents.write().drain() {
            handle.stop();
        }
        // Give in-flight messages a beat to drain.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let system = self.inner.system.clone();
        system.terminate().await;
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(p: &std::path::Path, body: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    fn fixture_agent(root: &std::path::Path) {
        let agent = root.join("agents").join("alpha");
        write(&agent.join("agent.yaml"), "id: alpha\nmodel: gpt-4o\n");
        write(&agent.join("SOUL.md"), "---\nidentity: Alpha\n---\nA terse agent.\n");
        write(&agent.join("RULES.md"), "- be helpful\n");
        write(&agent.join("MEMORY.md"), "- fact one\n");
        write(&agent.join("USER.md"), "user is Matt\n");
    }

    #[tokio::test]
    async fn runtime_spawns_and_identifies() {
        let tmp = tempdir().unwrap();
        fixture_agent(tmp.path());
        let cfg = HostConfig::load(tmp.path()).unwrap();
        let rt = HostRuntime::start(cfg).await.unwrap();
        let handle = rt.spawn_agent("alpha").await.unwrap();
        let id = handle.identify().await.unwrap();
        assert_eq!(id.agent_id, "alpha");
        assert_eq!(id.model, "gpt-4o");
        assert_eq!(id.persona_identity.as_deref(), Some("Alpha"));
        let status = handle.status().await.unwrap();
        assert_eq!(status.rules_count, 1);
        assert_eq!(status.memory_facts_count, 1);
        rt.shutdown().await;
    }
}
