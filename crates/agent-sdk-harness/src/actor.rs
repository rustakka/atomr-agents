//! The interactive session as an `atomr_core::actor::Actor`.
//!
//! `ClaudeSDKClient` is a stateful, bidirectional session — a natural fit
//! for an actor. Mirrors `AgentHostActor` in `crates/host`. Gated behind
//! the `actor` feature so the default build, the web companion, and the
//! PyO3 layer stay free of the actor runtime dependency.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{broadcast, oneshot};

use atomr_core::actor::{Actor, Context};

use atomr_agents_agent_sdk_core::{AgentSdkEvent, PermissionMode};

use crate::error::HarnessError;
use crate::session::InteractiveAgentSession;

/// Messages the [`AgentSdkActor`] handles — a 1:1 map of the
/// `ClaudeSDKClient` surface.
pub enum AgentSdkMsg {
    /// Send a user prompt and start streaming its response.
    Query {
        prompt: String,
        reply: oneshot::Sender<Result<(), HarnessError>>,
    },
    /// Interrupt the in-flight turn.
    Interrupt(oneshot::Sender<Result<(), HarnessError>>),
    /// Change permission mode mid-conversation.
    SetPermissionMode {
        mode: PermissionMode,
        reply: oneshot::Sender<Result<(), HarnessError>>,
    },
    /// Change model mid-conversation.
    SetModel {
        model: String,
        reply: oneshot::Sender<Result<(), HarnessError>>,
    },
    /// Hand back a fresh broadcast receiver for the event stream.
    Subscribe(oneshot::Sender<broadcast::Receiver<AgentSdkEvent>>),
    /// Close the session and tear down its `claude` subprocess.
    Shutdown,
}

pub struct AgentSdkActor {
    session: Arc<InteractiveAgentSession>,
}

impl AgentSdkActor {
    pub fn new(session: Arc<InteractiveAgentSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Actor for AgentSdkActor {
    type Msg = AgentSdkMsg;

    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: Self::Msg) {
        match msg {
            AgentSdkMsg::Query { prompt, reply } => {
                let _ = reply.send(self.session.send(prompt).await);
            }
            AgentSdkMsg::Interrupt(reply) => {
                let _ = reply.send(self.session.interrupt().await);
            }
            AgentSdkMsg::SetPermissionMode { mode, reply } => {
                let _ = reply.send(
                    self.session
                        .set_permission_mode(mode.as_sdk_str().to_string())
                        .await,
                );
            }
            AgentSdkMsg::SetModel { model, reply } => {
                let _ = reply.send(self.session.set_model(model).await);
            }
            AgentSdkMsg::Subscribe(reply) => {
                let _ = reply.send(self.session.subscribe_receiver());
            }
            AgentSdkMsg::Shutdown => {
                let _ = self.session.close().await;
            }
        }
    }
}
