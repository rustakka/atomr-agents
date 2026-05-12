//! Phonemizer trait — text → IPA + per-token list.
//!
//! Local TTS backends (Piper, Kokoro, …) need to convert text to
//! phonemes before running their ONNX graphs. This trait keeps the
//! phonemizer pluggable so callers can swap implementations
//! (espeak-ng, misaki, a custom in-process Python phonemizer, etc.)
//! without touching the per-backend runner.
//!
//! The default implementation lives in `atomr-agents-tts-phonemize`
//! (`EspeakNgPhonemizer`, behind the `espeak-ng` feature). Backends
//! accept an `Arc<dyn Phonemizer>` at construction time; a
//! [`MockPhonemizer`] is provided here for unit tests.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use atomr_agents_stt_core::Result;

#[async_trait]
pub trait Phonemizer: Send + Sync + 'static {
    /// Convert `text` (in `language`, BCP-47 tag like `"en-us"`) to a
    /// phonemized representation. Output `tokens` are voice-agnostic;
    /// callers map them to model-specific IDs via the per-voice
    /// phoneme→id table.
    async fn phonemize(&self, text: &str, language: &str) -> Result<PhonemizedText>;
}

pub type DynPhonemizer = Arc<dyn Phonemizer>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PhonemizedText {
    /// Concatenated IPA string (mostly for debugging / logging).
    pub ipa: String,
    /// One token per phoneme (typically a single IPA grapheme cluster
    /// or a stress marker). Backends look these up in their voice's
    /// phoneme-id table.
    pub tokens: Vec<String>,
}

impl PhonemizedText {
    pub fn new(ipa: impl Into<String>, tokens: Vec<String>) -> Self {
        Self { ipa: ipa.into(), tokens }
    }
}

/// Deterministic in-process phonemizer for tests. Returns one token
/// per non-whitespace character in the input — sufficient to exercise
/// runner glue without requiring a real phonemizer to be installed.
pub struct MockPhonemizer;

#[async_trait]
impl Phonemizer for MockPhonemizer {
    async fn phonemize(&self, text: &str, _language: &str) -> Result<PhonemizedText> {
        let tokens: Vec<String> = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_string())
            .collect();
        let ipa = tokens.join("");
        Ok(PhonemizedText { ipa, tokens })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_phonemizer_returns_one_token_per_non_whitespace_char() {
        let p = MockPhonemizer;
        let out = p.phonemize("hi there", "en-us").await.unwrap();
        assert_eq!(out.tokens, vec!["h", "i", "t", "h", "e", "r", "e"]);
        assert_eq!(out.ipa, "hithere");
    }

    #[tokio::test]
    async fn phonemized_text_serializes_to_json() {
        let p = MockPhonemizer;
        let out = p.phonemize("ab", "en").await.unwrap();
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["ipa"], "ab");
        assert_eq!(json["tokens"], serde_json::json!(["a", "b"]));
    }

    #[tokio::test]
    async fn dyn_phonemizer_alias_works() {
        let p: DynPhonemizer = Arc::new(MockPhonemizer);
        let out = p.phonemize("x", "en").await.unwrap();
        assert_eq!(out.tokens, vec!["x"]);
    }
}
