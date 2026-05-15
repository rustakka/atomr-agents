use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XttsConfig {
    /// Endpoint of the colocated Coqui XTTS Python server.
    #[serde(default = "default_endpoint")]
    pub endpoint: Url,
    /// Default speaker preset (when caller does not pass a clone clip).
    #[serde(default)]
    pub default_speaker: Option<String>,
    /// Default language hint.
    #[serde(default = "default_language")]
    pub default_language: String,
    #[serde(default)]
    pub bearer_token: Option<String>,
}

fn default_endpoint() -> Url {
    Url::parse("http://127.0.0.1:8020/").expect("XTTS endpoint")
}
fn default_language() -> String {
    "en".to_string()
}

impl Default for XttsConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            default_speaker: None,
            default_language: default_language(),
            bearer_token: None,
        }
    }
}
