//! `SpeechToText` impl for whisper.cpp.
//!
//! Without the `whisper-cpp` feature the runner returns
//! [`SttError::ModelLoad`] from `transcribe`, naming the missing
//! feature so callers know how to enable it. With the feature, it
//! holds a `parking_lot::Mutex<WhisperContext>` (whisper.cpp is
//! single-threaded per context) and dispatches the actual call via
//! `tokio::task::spawn_blocking`.

#[cfg(feature = "whisper-cpp")]
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_stt_core::{
    AudioInput, BackendKind, Capabilities, Result, SpeechToText, StreamOptions,
    StreamingSession, SttError, TranscribeOptions, Transcript, TransportKind,
};

use crate::caps::CAPS;
use crate::config::WhisperConfig;

#[cfg(feature = "whisper-cpp")]
use atomr_agents_stt_core::{Segment, Word};

pub struct WhisperRunner {
    config: WhisperConfig,
    #[cfg(feature = "whisper-cpp")]
    ctx: Arc<parking_lot::Mutex<whisper_rs::WhisperContext>>,
}

impl WhisperRunner {
    pub fn new(config: WhisperConfig) -> Result<Self> {
        #[cfg(feature = "whisper-cpp")]
        {
            let cpath = config
                .model_path
                .to_str()
                .ok_or_else(|| SttError::model_load("model_path is not valid UTF-8"))?;
            let mut params = whisper_rs::WhisperContextParameters::default();
            params.use_gpu = config.gpu;
            let ctx = whisper_rs::WhisperContext::new_with_params(cpath, params)
                .map_err(|e| SttError::model_load(format!("whisper init: {e}")))?;
            Ok(Self {
                config,
                ctx: Arc::new(parking_lot::Mutex::new(ctx)),
            })
        }
        #[cfg(not(feature = "whisper-cpp"))]
        {
            // Validate the path exists at construction so callers
            // get an early signal even without the feature.
            if !config.model_path.exists() {
                tracing::warn!(
                    path = ?config.model_path,
                    "WhisperRunner::new: model_path does not exist (the `whisper-cpp` feature is also disabled, so this won't load anything)",
                );
            }
            Ok(Self { config })
        }
    }
}

#[async_trait]
impl SpeechToText for WhisperRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::WhisperLocal
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::LocalModel
    }

    async fn transcribe(
        &self,
        input: AudioInput,
        opts: TranscribeOptions,
    ) -> Result<Transcript> {
        if opts.diarize {
            return Err(SttError::UnsupportedCapability("diarization"));
        }
        #[cfg(not(feature = "whisper-cpp"))]
        {
            let _ = (input, opts, &self.config);
            return Err(SttError::model_load(
                "atomr-agents-stt-runtime-whisper built without the `whisper-cpp` feature; \
                 rebuild with `--features whisper-cpp` (or `cuda`/`metal`/`coreml`) to enable.",
            ));
        }
        #[cfg(feature = "whisper-cpp")]
        {
            let pcm = decode_to_mono_16k(input).await?;
            let lang = opts
                .language
                .clone()
                .or_else(|| self.config.default_language.clone());
            let beam = self.config.beam_size as i32;
            let n_threads = self.config.n_threads as i32;
            let prompt = opts.initial_prompt.clone();
            let ctx = self.ctx.clone();
            let model_path = self.config.model_path.clone();

            let result = tokio::task::spawn_blocking(move || -> Result<Transcript> {
                let mut state = ctx
                    .lock()
                    .create_state()
                    .map_err(|e| SttError::internal(format!("whisper state: {e}")))?;
                let mut params = whisper_rs::FullParams::new(
                    whisper_rs::SamplingStrategy::BeamSearch {
                        beam_size: beam,
                        patience: -1.0,
                    },
                );
                params.set_n_threads(n_threads);
                params.set_token_timestamps(true);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_special(false);
                params.set_print_timestamps(false);
                if let Some(l) = &lang {
                    params.set_language(Some(l));
                } else {
                    params.set_language(Some("auto"));
                }
                if let Some(p) = &prompt {
                    params.set_initial_prompt(p);
                }

                state
                    .full(params, &pcm.samples)
                    .map_err(|e| SttError::internal(format!("whisper full: {e}")))?;

                let n_segments = state
                    .full_n_segments()
                    .map_err(|e| SttError::internal(format!("whisper segments: {e}")))?;

                let mut segments: Vec<Segment> = Vec::with_capacity(n_segments as usize);
                let mut all_text = String::new();
                for i in 0..n_segments {
                    let text = state
                        .full_get_segment_text(i)
                        .map_err(|e| SttError::internal(format!("seg text: {e}")))?;
                    let t0 = state.full_get_segment_t0(i).unwrap_or(0);
                    let t1 = state.full_get_segment_t1(i).unwrap_or(0);
                    // whisper t0/t1 are in 10ms ticks.
                    let start_ms = (t0.max(0) as u32) * 10;
                    let end_ms = (t1.max(0) as u32) * 10;
                    let mut words: Vec<Word> = Vec::new();
                    if let Ok(n_tokens) = state.full_n_tokens(i) {
                        for j in 0..n_tokens {
                            if let (Ok(t), Ok(td)) = (
                                state.full_get_token_text(i, j),
                                state.full_get_token_data(i, j),
                            ) {
                                if t.starts_with('[') && t.ends_with(']') {
                                    continue;
                                }
                                words.push(Word {
                                    text: t,
                                    start_ms: (td.t0.max(0) as u32) * 10,
                                    end_ms: (td.t1.max(0) as u32) * 10,
                                    confidence: Some(td.p),
                                });
                            }
                        }
                    }
                    all_text.push_str(&text);
                    segments.push(Segment {
                        text,
                        start_ms,
                        end_ms,
                        words,
                        speaker: None,
                        confidence: None,
                    });
                }
                let duration_secs = pcm.duration_secs();
                Ok(Transcript {
                    text: all_text,
                    language: lang,
                    segments,
                    duration_secs,
                    backend: BackendKind::WhisperLocal,
                    model_id: Some(
                        model_path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("whisper")
                            .to_string(),
                    ),
                    cost_usd: Some(0.0),
                })
            })
            .await
            .map_err(|e| SttError::internal(format!("spawn_blocking: {e}")))??;
            Ok(result)
        }
    }

    async fn open_stream(
        &self,
        _opts: StreamOptions,
    ) -> Result<Box<dyn StreamingSession>> {
        Err(SttError::UnsupportedCapability("streaming_push"))
    }
}

#[cfg(feature = "whisper-cpp")]
async fn decode_to_mono_16k(
    input: AudioInput,
) -> Result<atomr_agents_stt_core::PcmBuffer> {
    let pcm = match input {
        AudioInput::Pcm(p) => p,
        other => {
            tokio::task::spawn_blocking(move || atomr_agents_stt_audio::decode::decode_to_pcm(other))
                .await
                .map_err(|e| SttError::internal(format!("decode spawn: {e}")))??
        }
    };
    let mono = atomr_agents_stt_audio::decode::to_mono(&pcm);
    let resampled = atomr_agents_stt_audio::resample::resample_mono(&mono, 16_000)?;
    Ok(resampled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_advertise_local_only() {
        assert!(CAPS.batch);
        assert!(!CAPS.streaming_push);
        assert!(!CAPS.requires_network);
    }

    #[cfg(not(feature = "whisper-cpp"))]
    #[tokio::test]
    async fn without_feature_returns_typed_model_load_error() {
        let r = WhisperRunner::new(WhisperConfig::new("/nonexistent/whisper.gguf")).unwrap();
        let err = r
            .transcribe(
                AudioInput::Pcm(atomr_agents_stt_core::PcmBuffer::new(vec![], 16_000, 1)),
                TranscribeOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SttError::ModelLoad(_)), "got {err:?}");
    }

    #[cfg(feature = "whisper-cpp")]
    #[tokio::test]
    async fn with_feature_missing_model_path_surfaces_typed_error() {
        // With `whisper-cpp` the constructor itself tries to load the
        // model and fails fast with `SttError::ModelLoad`.
        match WhisperRunner::new(WhisperConfig::new("/nonexistent/whisper.gguf")) {
            Ok(_) => panic!("expected ModelLoad error from missing model path"),
            Err(SttError::ModelLoad(_)) => {}
            Err(other) => panic!("expected ModelLoad, got {other:?}"),
        }
    }
}
