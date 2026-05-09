//! `TranscribeTool` — a [`Tool`] the model can elect to call to
//! transcribe an audio file at a given path.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result, ToolId, Value};
use atomr_agents_stt_core::{
    AudioInput, DynSpeechToText, TranscribeOptions,
};
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;

/// Tool wrapper around any [`SpeechToText`](atomr_agents_stt_core::SpeechToText)
/// implementation. Accepts `{"audio_path": "...", "language": null,
/// "diarize": false}`.
pub struct TranscribeTool {
    descriptor: ToolDescriptor,
    stt: DynSpeechToText,
}

impl TranscribeTool {
    pub fn new(stt: DynSpeechToText) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("stt.transcribe"),
            name: "transcribe_audio".into(),
            description: "Transcribe an audio file to text. Returns {text, language, segments}.".into(),
            schema: ToolSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "audio_path": {
                        "type": "string",
                        "description": "Absolute path to a WAV/MP3/FLAC/Ogg/MP4/WebM audio file."
                    },
                    "language": {
                        "type": ["string", "null"],
                        "description": "Optional BCP-47 hint (e.g. 'en'). Leave null for auto-detect."
                    },
                    "diarize": {
                        "type": "boolean",
                        "description": "Request speaker diarization (only when the backend supports it).",
                        "default": false
                    },
                    "initial_prompt": {
                        "type": ["string", "null"],
                        "description": "Optional context hint to bias decoding."
                    }
                },
                "required": ["audio_path"]
            })),
        };
        Self { descriptor, stt }
    }

    pub fn from_arc(stt: DynSpeechToText) -> Arc<dyn Tool> {
        Arc::new(Self::new(stt))
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    audio_path: PathBuf,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    diarize: bool,
    #[serde(default)]
    initial_prompt: Option<String>,
}

#[async_trait]
impl Tool for TranscribeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let parsed: Args = serde_json::from_value(args).map_err(|e| {
            AgentError::Tool(format!("transcribe_audio: bad args: {e}"))
        })?;
        let opts = TranscribeOptions {
            language: parsed.language,
            diarize: parsed.diarize,
            initial_prompt: parsed.initial_prompt,
            punctuation: true,
            ..Default::default()
        };
        let t = self
            .stt
            .transcribe(AudioInput::File(parsed.audio_path), opts)
            .await
            .map_err(|e| AgentError::Tool(format!("transcribe_audio: {e}")))?;
        // Round-trip the transcript through serde so the model sees
        // a JSON object with all fields populated.
        serde_json::to_value(&t).map_err(AgentError::Serde)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{
        CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
    };
    use atomr_agents_stt_core::MockSpeechToText;
    use std::sync::Arc;
    use std::time::Duration;

    fn ctx() -> InvokeCtx {
        let call = CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(60)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(8),
            trace: vec![],
        };
        InvokeCtx {
            call,
            tool_call_id: "call-1".into(),
            raw_args: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn rejects_bad_args() {
        let stt: DynSpeechToText = Arc::new(MockSpeechToText::new());
        let t = TranscribeTool::new(stt);
        let res = t.invoke(serde_json::json!({"path": "/tmp/x.wav"}), &ctx()).await;
        assert!(res.is_err(), "expected bad-args error");
    }

    #[tokio::test]
    async fn transcribes_existing_file() {
        // Write a tiny temp file so the mock's `tokio::fs::metadata`
        // call succeeds.
        let dir = std::env::temp_dir();
        let p = dir.join("atomr-stt-tool-test.wav");
        tokio::fs::write(&p, b"riff").await.unwrap();
        let stt: DynSpeechToText = Arc::new(MockSpeechToText::new().with_text("hello"));
        let t = TranscribeTool::new(stt);
        let v = t
            .invoke(
                serde_json::json!({"audio_path": p.to_str().unwrap()}),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(v["text"], "hello");
        assert_eq!(v["backend"], "mock");
    }
}
