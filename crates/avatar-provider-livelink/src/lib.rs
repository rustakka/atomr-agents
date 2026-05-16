//! Avatar Live Link UDP sink.
//!
//! Streams [`atomr_agents_avatar_core::AvatarFrame`]s out a UDP
//! socket using the `avatar-core` CBOR wire format. The receiver is
//! an Unreal Engine 5 plugin that implements `ILiveLinkSource` and
//! pushes frames into a MetaHuman animation blueprint. See
//! [`atomr_agents_avatar_core::wire`] for the framing spec.
//!
//! **x86_64-only.** The Unreal Engine tooling pipeline is x86_64-first
//! and NVIDIA-Audio2Face is x86_64-only; keeping the sender symmetric
//! avoids a half-supported matrix. The whole crate compiles to an
//! empty library on other architectures via the `#![cfg]` below, so
//! workspace builds on aarch64 don't fail.

#![cfg(target_arch = "x86_64")]
#![forbid(unsafe_code)]

mod config;
mod sink;

pub use config::LiveLinkConfig;
pub use sink::LiveLinkSink;
