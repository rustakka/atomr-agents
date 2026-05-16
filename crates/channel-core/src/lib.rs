//! Channel + thread domain layer.
//!
//! A **channel** is a provider-specific messaging transport — WhatsApp,
//! Signal, Discord, or the in-process [`memory::InMemoryProvider`] used
//! for tests. A **thread** is a long-lived conversation between a
//! channel peer and a bound [`ThreadTarget`]; the target can be any
//! [`Callable`](atomr_agents_callable::Callable) (so agents, harnesses,
//! teams, workflows, or plain closures all qualify) or a [`HarnessRef`]
//! routed through a [`HarnessInputAdapter`].
//!
//! Channels are an **optional, additive** layer over existing agent
//! conversation interactions: nothing in [`AgentRef::turn`](atomr_agents_core)
//! or [`HarnessRef::run`](atomr_agents_callable) changes — channels
//! just expose the same callables behind a messaging surface.

#![forbid(unsafe_code)]

mod content;
mod error;
mod events;
mod ids;
pub mod memory;
mod provider;
mod spec;
mod store;
mod target;
mod thread;

pub use atomr_agents_callable::{Callable, CallableHandle};

pub use content::{
    ChannelMessageRecord, Direction, InboundMessage, MessageContent, OutboundMessage, ProviderAck,
};
pub use error::{ChannelError, Result};
pub use events::{ChannelEvent, ChannelEventStream};
pub use ids::{ChannelId, PeerId, ThreadId};
pub use provider::{ChannelProvider, ProviderHandle};
pub use spec::{Capabilities, ChannelSpec, ProviderKind};
pub use store::{ChannelStore, InMemoryChannelStore, ThreadSummary};
pub use target::{HarnessInputAdapter, ThreadTarget};
pub use thread::{Thread, ThreadPolicy, ThreadRef};
