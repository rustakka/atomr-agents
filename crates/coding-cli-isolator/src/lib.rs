//! Process isolation backends for the coding-cli harness.
//!
//! - [`LocalIsolator`] spawns the CLI on the host (`tokio::process`
//!   for headless, `portable-pty` for interactive).
//! - [`DockerIsolator`] spawns it inside a Docker container using
//!   `bollard`. Feature-gated behind `docker`.
//!
//! Both back ends produce a [`ProcessHandle`] with a uniform async
//! interface for the harness to drive.

#![forbid(unsafe_code)]

mod error;
mod handle;
mod local;
mod pty_bridge;
mod traits;

#[cfg(feature = "docker")]
mod docker;

pub use error::IsolatorError;
pub use handle::{ExitStatus, IsolationOpts, ProcessHandle};
pub use local::LocalIsolator;
pub use traits::Isolator;

#[cfg(feature = "docker")]
pub use docker::DockerIsolator;
