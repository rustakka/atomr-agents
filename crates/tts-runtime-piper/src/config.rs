use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiperConfig {
    /// Voice models keyed by an ID the caller passes in
    /// `VoiceRef::Library { id }`. The first entry doubles as the
    /// default when the caller does not specify a voice.
    pub voices: Vec<PiperVoiceModel>,
    /// Length scale (slower = larger; 1.0 default).
    #[serde(default = "default_length_scale")]
    pub length_scale: f32,
    /// Noise scale (variation; 0.667 default).
    #[serde(default = "default_noise_scale")]
    pub noise_scale: f32,
    /// Noise-W scale (cadence variation; 0.8 default).
    #[serde(default = "default_noise_w")]
    pub noise_w: f32,
    /// Use CUDA execution provider when compiled with `piper-ort`.
    #[serde(default)]
    pub use_gpu: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiperVoiceModel {
    pub id: String,
    pub onnx_path: PathBuf,
    pub config_path: PathBuf,
    #[serde(default)]
    pub language: Option<String>,
}

fn default_length_scale() -> f32 {
    1.0
}
fn default_noise_scale() -> f32 {
    0.667
}
fn default_noise_w() -> f32 {
    0.8
}

impl Default for PiperConfig {
    fn default() -> Self {
        Self {
            voices: Vec::new(),
            length_scale: default_length_scale(),
            noise_scale: default_noise_scale(),
            noise_w: default_noise_w(),
            use_gpu: false,
        }
    }
}
