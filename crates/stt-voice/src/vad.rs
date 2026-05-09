//! Voice-activity detection.
//!
//! The trait is intentionally narrow — `is_speech(frame)` over a
//! short frame (typically 30 ms of i16 PCM at 16 kHz). Two impls
//! ship in this crate: the always-available [`EnergyVad`] (RMS-energy
//! threshold; cheap, decent for quiet rooms) and, behind the
//! `vad-silero` feature, [`SileroVad`] which wraps the well-known
//! Silero ONNX VAD via the `voice_activity_detector` crate.

pub trait Vad: Send + Sync {
    /// Examine one PCM frame (interleaved f32, mono). Returns `true`
    /// if the frame is likely speech.
    fn is_speech(&mut self, frame: &[f32], sample_rate: u32) -> bool;
}

/// Cheap RMS-energy VAD. Useful as a baseline and as a default when
/// `vad-silero` isn't enabled.
pub struct EnergyVad {
    pub threshold: f32,
}

impl Default for EnergyVad {
    fn default() -> Self {
        Self { threshold: 0.01 }
    }
}

impl Vad for EnergyVad {
    fn is_speech(&mut self, frame: &[f32], _sample_rate: u32) -> bool {
        if frame.is_empty() {
            return false;
        }
        let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
        let rms = (sum_sq / frame.len() as f32).sqrt();
        rms >= self.threshold
    }
}

#[cfg(feature = "vad-silero")]
pub struct SileroVad {
    inner: voice_activity_detector::VoiceActivityDetector,
    threshold: f32,
}

#[cfg(feature = "vad-silero")]
impl SileroVad {
    pub fn new(sample_rate: u32, chunk_size: usize, threshold: f32) -> Self {
        let inner = voice_activity_detector::VoiceActivityDetector::builder()
            .sample_rate(sample_rate as i64)
            .chunk_size(chunk_size)
            .build()
            .expect("silero vad init");
        Self { inner, threshold }
    }
}

#[cfg(feature = "vad-silero")]
impl Vad for SileroVad {
    fn is_speech(&mut self, frame: &[f32], _sample_rate: u32) -> bool {
        let prob = self.inner.predict(frame.iter().copied());
        prob >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_vad_silence_below_threshold() {
        let mut v = EnergyVad::default();
        let frame = vec![0.0; 480];
        assert!(!v.is_speech(&frame, 16_000));
    }

    #[test]
    fn energy_vad_loud_above_threshold() {
        let mut v = EnergyVad::default();
        let frame: Vec<f32> = (0..480).map(|_| 0.5).collect();
        assert!(v.is_speech(&frame, 16_000));
    }
}
