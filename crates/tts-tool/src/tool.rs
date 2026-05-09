//! `SpeakTool` — render text to a WAV file via any `TextToSpeech`
//! backend. The tool returns `{path, duration_secs, voice}`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result, ToolId, Value};
use atomr_agents_stt_core::AudioFormat;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use atomr_agents_tts_audio::encode::pcm_to_wav_bytes;
use atomr_agents_tts_core::{
    AudioOutput, DynTextToSpeech, SynthesisRequest, VoiceRef,
};
use serde::Deserialize;

pub struct SpeakTool {
    descriptor: ToolDescriptor,
    tts: DynTextToSpeech,
    output_dir: PathBuf,
}

impl SpeakTool {
    pub fn new(tts: DynTextToSpeech) -> Self {
        Self::with_output_dir(tts, std::env::temp_dir())
    }

    pub fn with_output_dir(tts: DynTextToSpeech, output_dir: PathBuf) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from("tts.speak"),
            name: "speak_text".into(),
            description: "Synthesise spoken audio for the given text and write it to a WAV file. \
                Returns {path, duration_secs, voice, backend}."
                .into(),
            schema: ToolSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to vocalise. Backends may impose per-request length limits."
                    },
                    "voice": {
                        "type": ["string", "null"],
                        "description": "Voice ID (interpreted by the backend, e.g. 'alloy', 'rachel')."
                    }
                },
                "required": ["text"]
            })),
        };
        Self {
            descriptor,
            tts,
            output_dir,
        }
    }

    pub fn from_arc(tts: DynTextToSpeech) -> Arc<dyn Tool> {
        Arc::new(Self::new(tts))
    }
}

#[derive(Debug, Deserialize)]
struct Args {
    text: String,
    #[serde(default)]
    voice: Option<String>,
}

#[async_trait]
impl Tool for SpeakTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("speak_text: bad args: {e}")))?;
        let voice = parsed
            .voice
            .clone()
            .map(|id| VoiceRef::Library { id })
            .unwrap_or(VoiceRef::Library { id: "default".to_string() });
        let req = SynthesisRequest::tts(parsed.text.clone(), voice);
        let out: AudioOutput = self
            .tts
            .synthesize(req)
            .await
            .map_err(|e| AgentError::Tool(format!("speak_text: {e}")))?;
        let bytes = render_wav(&out)?;
        let suffix = uuid_like();
        let path = self.output_dir.join(format!("speak-{suffix}.wav"));
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| AgentError::Tool(format!("speak_text: write: {e}")))?;

        Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "duration_secs": out.duration_secs,
            "voice": out.voice_id_used,
            "model": out.model_id,
            "backend": format!("{:?}", out.backend),
            "characters_processed": out.characters_processed,
            "format": format_to_string(&out.format),
        }))
    }
}

fn render_wav(out: &AudioOutput) -> Result<Vec<u8>> {
    if !out.audio.samples.is_empty() {
        return pcm_to_wav_bytes(&out.audio)
            .map(|b| b.to_vec())
            .map_err(|e| AgentError::Tool(format!("speak_text: wav encode: {e}")));
    }
    // Container-bytes path (MP3 / Opus etc.). Write the container as-is
    // — the file suffix lies in name only; the contents are valid in
    // their native format. Callers that need true WAV should configure
    // the backend to emit PCM.
    if let Some(b) = &out.container_bytes {
        return Ok(b.to_vec());
    }
    Ok(Vec::new())
}

fn format_to_string(f: &AudioFormat) -> String {
    match f {
        AudioFormat::Pcm { sample_rate, channels, .. } => {
            format!("pcm_{sample_rate}_{channels}")
        }
        AudioFormat::Wav => "wav".into(),
        AudioFormat::Mp3 => "mp3".into(),
        AudioFormat::Flac => "flac".into(),
        AudioFormat::Ogg => "ogg".into(),
        AudioFormat::Opus => "opus".into(),
        AudioFormat::Mulaw { sample_rate } => format!("ulaw_{sample_rate}"),
        AudioFormat::Mp4 => "mp4".into(),
        AudioFormat::Webm => "webm".into(),
        AudioFormat::Aac => "aac".into(),
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{
        CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
    };
    use atomr_agents_tts_core::MockTextToSpeech;
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
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let t = SpeakTool::new(tts);
        let res = t.invoke(serde_json::json!({"foo": "bar"}), &ctx()).await;
        assert!(res.is_err(), "expected bad-args error");
    }

    #[tokio::test]
    async fn speaks_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let tts: DynTextToSpeech = Arc::new(MockTextToSpeech::default());
        let t = SpeakTool::with_output_dir(tts, dir.path().to_path_buf());
        let v = t
            .invoke(
                serde_json::json!({"text": "hello world", "voice": "default"}),
                &ctx(),
            )
            .await
            .unwrap();
        let path = v["path"].as_str().unwrap();
        let meta = tokio::fs::metadata(path).await.unwrap();
        assert!(meta.len() > 0);
        assert_eq!(v["voice"], "default");
    }
}
