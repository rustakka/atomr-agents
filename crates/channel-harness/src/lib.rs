//! Channel orchestrator.
//!
//! [`ChannelHarness`] is a long-lived runtime that:
//!
//! - Attaches one or more [`ChannelProvider`]s (WhatsApp, Signal, Discord,
//!   memory) under stable [`ChannelId`]s.
//! - Runs an **inbound loop** that consumes every provider's inbound
//!   stream, dedups by `(channel, provider_msg_id)`, looks up (or
//!   auto-opens) a [`Thread`] keyed by `(channel, peer)`, dispatches to
//!   its bound [`ThreadTarget`], and enqueues a reply onto the channel's
//!   outbound queue.
//! - Runs one **outbound worker per channel** that drains the queue and
//!   calls `provider.send` (with cooperative retry).
//! - Fans out [`ChannelEvent`]s on a `tokio::broadcast` so the `-web`
//!   companion or a Python observer can stream lifecycle events.
//! - Persists channels, threads, and message records through a
//!   pluggable [`ChannelStore`]; the default
//!   [`InMemoryChannelStore`] is sufficient for tests and worked
//!   examples.
//!
//! The harness is purely additive: existing [`AgentRef::turn`] and
//! [`HarnessRef::run`] call sites are untouched. A channel is just a
//! new way to invoke an existing [`Callable`].

#![forbid(unsafe_code)]

mod builder;
mod harness;
mod inbound;
mod outbound;

pub use atomr_agents_channel_core::{
    Callable, CallableHandle, Capabilities, ChannelEvent, ChannelEventStream, ChannelId,
    ChannelMessageRecord, ChannelProvider, ChannelSpec, ChannelStore, Direction,
    HarnessInputAdapter, InMemoryChannelStore, InboundMessage, MessageContent, OutboundMessage,
    PeerId, ProviderAck, ProviderHandle, ProviderKind, Thread, ThreadId, ThreadPolicy, ThreadRef,
    ThreadTarget,
};

pub use builder::ChannelHarnessBuilder;
pub use harness::ChannelHarness;

/// `Result` alias for harness errors.
pub use atomr_agents_channel_core::{ChannelError, Result};
