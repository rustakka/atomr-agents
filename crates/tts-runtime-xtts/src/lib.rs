//! Coqui XTTS v2 backend.
//!
//! XTTS v2 is a Coqui-released zero-shot voice-cloning TTS model
//! supporting 17 languages and ~6-second reference clips. Like
//! MOSS-TTS this crate hosts the model out-of-process via Python
//! and talks to it over HTTP. The trait surface + [`CAPS`] ship
//! today; the HTTP client is gated behind `xtts-http`.

mod caps;
mod config;
mod runner;

pub use caps::CAPS;
pub use config::XttsConfig;
pub use runner::XttsRunner;
