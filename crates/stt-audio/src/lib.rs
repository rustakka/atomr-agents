//! Audio I/O helpers shared by every backend that needs to ingest
//! or emit raw PCM.
//!
//! Two independent feature surfaces:
//!
//! - `decode` — `symphonia`-based decoder. Converts `AudioInput`
//!   (file or in-memory bytes of any common container/codec) into a
//!   normalized [`atomr_agents_stt_core::PcmBuffer`].
//! - `resample` — `rubato`-based sample-rate conversion. Used by
//!   whisper-rs to downsample to 16 kHz mono.
//! - `mic` — `cpal`-based microphone capture. See [`mic`] module
//!   for the [`MicCaptureSession`] producer.
//!
//! Cloud backends generally don't need `decode` (they accept
//! containers directly) but local backends always do.

#[cfg(feature = "decode")]
pub mod decode;

#[cfg(feature = "decode")]
pub mod wav;

#[cfg(feature = "resample")]
pub mod resample;

#[cfg(feature = "mic")]
pub mod mic;
