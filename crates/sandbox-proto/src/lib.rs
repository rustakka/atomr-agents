//! `atomr-agents-sandbox-proto` — the host ↔ guest wire protocol for the
//! microVM sandbox.
//!
//! Communication is **length-prefixed `postcard` frames** (`u32` little-endian
//! length + postcard body) over AF_VSOCK. This keeps the in-VM guest agent a
//! tiny static binary — no tonic / h2 / tower stack inside the VM, which
//! matters for cold-start size and the musl static link. gRPC is used only
//! *host-to-host* in the Tier-3 cluster, where vsock frames are tunneled inside
//! the node ↔ control-plane stream.
//!
//! The crate is transport-agnostic: [`read_frame`] / [`write_frame`] work over
//! any [`tokio::io::AsyncRead`] / [`AsyncWrite`], so they are exercised over
//! in-memory pipes in tests and over a real `AF_VSOCK` socket in the guest.

#![forbid(unsafe_code)]

mod error;
mod frame;
mod message;

pub use error::ProtoError;
pub use frame::{read_frame, write_frame, MAX_FRAME_BYTES};
pub use message::{GuestRequest, GuestResponse, Language};
