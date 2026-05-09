use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MossModelVariant {
    /// MOSS-TTS-Delay-8B (production quality, larger).
    Delay8B,
    /// MOSS-TTS-Local-1.7B (lighter local model).
    Local1_7B,
    /// MOSS-TTSD (multi-speaker dialogue).
    Tssd,
    /// MOSS-VoiceGenerator (text-described voice synthesis).
    VoiceGenerator,
    /// MOSS-SoundEffect (foley / SFX from text prompts).
    SoundEffect,
    /// MOSS-TTS-Realtime (bidirectional streaming).
    Realtime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MossTtsConfig {
    /// HTTP endpoint of the colocated MOSS-TTS server (SGLang or
    /// the FastAPI reference wrapper).
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// Which MOSS variant the colocated server is loaded with.
    #[serde(default = "default_variant")]
    pub model_variant: MossModelVariant,
    /// Default voice ID (interpreted by the colocated server).
    #[serde(default)]
    pub default_voice: Option<String>,
    /// Optional bearer token for the colocated HTTP service.
    #[serde(default)]
    pub bearer_token: Option<String>,
}

fn default_endpoint() -> Url { Url::parse("http://127.0.0.1:30000/").expect("MOSS endpoint") }
fn default_variant() -> MossModelVariant { MossModelVariant::Local1_7B }

impl Default for MossTtsConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            model_variant: default_variant(),
            default_voice: None,
            bearer_token: None,
        }
    }
}
