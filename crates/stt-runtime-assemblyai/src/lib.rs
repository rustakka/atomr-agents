//! AssemblyAI STT backend.
//!
//! - Batch: REST `POST /v2/upload` + `POST /v2/transcript` + poll
//!   `GET /v2/transcript/{id}` until `status == "completed"`.
//! - Streaming: Universal-Streaming WS at
//!   `wss://streaming.assemblyai.com/v3/ws?sample_rate=…`.

mod caps;
mod config;
mod runner;
mod stream;
mod wire;

pub use caps::CAPS;
pub use config::AssemblyAiConfig;
pub use runner::AssemblyAiRunner;
