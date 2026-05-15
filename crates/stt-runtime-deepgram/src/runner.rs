//! `SpeechToText` impl for Deepgram (REST batch + WS streaming).

use async_trait::async_trait;
use atomr_agents_stt_core::{
    AudioFormat, AudioInput, BackendKind, Capabilities, Result, Segment, SpeakerTag, SpeechToText,
    StreamOptions, StreamingSession, SttError, TranscribeOptions, Transcript, TransportKind, Word,
};
use atomr_agents_stt_remote_core::{build_http_client, classify_status, retry, ws};
use bytes::Bytes;
use reqwest::{header, Client};
use secrecy::ExposeSecret;

use crate::caps::CAPS;
use crate::config::DeepgramConfig;
use crate::stream::DeepgramStreamingSession;
use crate::wire::ListenResponse;

pub struct DeepgramRunner {
    config: DeepgramConfig,
    client: Client,
}

impl DeepgramRunner {
    pub fn new(config: DeepgramConfig) -> Result<Self> {
        let client = build_http_client(&config.timeouts)?;
        Ok(Self { config, client })
    }

    fn auth_header(&self) -> Result<String> {
        let secret = self.config.api_key.resolve()?;
        Ok(format!("Token {}", secret.expose_secret()))
    }

    fn build_query(&self, opts: &TranscribeOptions) -> Vec<(&'static str, String)> {
        let mut q = vec![(
            "model",
            opts.model
                .clone()
                .unwrap_or_else(|| self.config.default_model.clone()),
        )];
        if let Some(lang) = opts
            .language
            .clone()
            .or_else(|| self.config.default_language.clone())
        {
            q.push(("language", lang));
        } else {
            q.push(("detect_language", "true".into()));
        }
        if let Some(tier) = self.config.default_tier.clone() {
            q.push(("tier", tier));
        }
        if opts.diarize {
            q.push(("diarize", "true".into()));
        }
        if opts.punctuation {
            q.push(("punctuate", "true".into()));
        }
        if opts.profanity_filter {
            q.push(("profanity_filter", "true".into()));
        }
        // Always-on niceties.
        q.push(("smart_format", "true".into()));
        q.push(("utterances", "true".into()));
        q
    }
}

#[async_trait]
impl SpeechToText for DeepgramRunner {
    fn capabilities(&self) -> &'static Capabilities {
        &CAPS
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::Deepgram
    }

    fn transport_kind(&self) -> TransportKind {
        TransportKind::Hybrid
    }

    async fn transcribe(&self, input: AudioInput, opts: TranscribeOptions) -> Result<Transcript> {
        let url = self
            .config
            .rest_endpoint
            .join("listen")
            .map_err(|e| SttError::internal(format!("join URL: {e}")))?;
        let (bytes, content_type) = read_input(input).await?;
        let auth = self.auth_header()?;
        let q = self.build_query(&opts);
        let policy = self.config.retry.clone();
        let client = self.client.clone();

        let parsed = retry(&policy, move || {
            let client = client.clone();
            let url = url.clone();
            let auth = auth.clone();
            let q = q.clone();
            let bytes = bytes.clone();
            let content_type = content_type.clone();
            async move {
                let resp = client
                    .post(url.clone())
                    .query(&q)
                    .header(header::AUTHORIZATION, &auth)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(bytes.to_vec())
                    .send()
                    .await
                    .map_err(|e| SttError::transport(format!("deepgram POST: {e}")))?;
                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let retry_after = resp
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());
                    let body = resp.text().await.unwrap_or_default();
                    return Err(classify_status(status, retry_after, body));
                }
                let parsed: ListenResponse = resp
                    .json()
                    .await
                    .map_err(|e| SttError::transport(format!("deepgram parse: {e}")))?;
                Ok(parsed)
            }
        })
        .await?;
        Ok(into_transcript(
            parsed,
            self.config.default_model.clone(),
            opts.model,
        ))
    }

    async fn open_stream(&self, opts: StreamOptions) -> Result<Box<dyn StreamingSession>> {
        let mut url = self.config.ws_endpoint.clone();
        let model = opts
            .model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("model", &model);
            if let Some(lang) = opts
                .language
                .clone()
                .or_else(|| self.config.default_language.clone())
            {
                q.append_pair("language", &lang);
            } else {
                q.append_pair("detect_language", "true");
            }
            if opts.diarize {
                q.append_pair("diarize", "true");
            }
            // Sensible streaming defaults.
            q.append_pair("smart_format", "true");
            q.append_pair("interim_results", "true");
            q.append_pair("utterance_end_ms", "1000");
            q.append_pair("vad_events", "true");
            // Encoding hint.
            if let Some(f) = &opts.format {
                let (encoding, sample_rate) = encoding_hint(f);
                q.append_pair("encoding", encoding);
                if let Some(sr) = sample_rate {
                    q.append_pair("sample_rate", &sr.to_string());
                }
            }
        }
        let auth = self.auth_header()?;
        let stream = ws::connect(url.as_str(), &[("Authorization", auth.as_str())]).await?;
        Ok(Box::new(DeepgramStreamingSession::spawn(stream)))
    }
}

fn encoding_hint(f: &AudioFormat) -> (&'static str, Option<u32>) {
    match f {
        AudioFormat::Pcm {
            sample_rate, sample, ..
        } => (
            match sample {
                atomr_agents_stt_core::SampleType::I16 => "linear16",
                atomr_agents_stt_core::SampleType::I32 => "linear16",
                atomr_agents_stt_core::SampleType::F32 => "linear16",
            },
            Some(*sample_rate),
        ),
        AudioFormat::Wav => ("linear16", Some(16_000)),
        AudioFormat::Mp3 => ("mp3", None),
        AudioFormat::Flac => ("flac", None),
        AudioFormat::Ogg => ("ogg-opus", None),
        AudioFormat::Opus => ("opus", None),
        AudioFormat::Mulaw { sample_rate } => ("mulaw", Some(*sample_rate)),
        _ => ("linear16", Some(16_000)),
    }
}

async fn read_input(input: AudioInput) -> Result<(Bytes, String)> {
    match input {
        AudioInput::File(p) => {
            let data = tokio::fs::read(&p).await?;
            let ct = match p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .as_deref()
            {
                Some("mp3") => "audio/mpeg",
                Some("flac") => "audio/flac",
                Some("ogg") => "audio/ogg",
                Some("opus") => "audio/opus",
                _ => "audio/wav",
            };
            Ok((Bytes::from(data), ct.to_string()))
        }
        AudioInput::Bytes { data, format } => Ok((data, format.mime().to_string())),
        AudioInput::Pcm(_) => Err(SttError::UnsupportedFormat(
            "PCM input requires WAV-encoding via stt-audio::wav before deepgram batch".into(),
        )),
    }
}

fn into_transcript(r: ListenResponse, cfg_model: String, opt_model: Option<String>) -> Transcript {
    let language = r
        .results
        .as_ref()
        .and_then(|res| res.channels.first())
        .and_then(|c| c.detected_language.clone());

    let duration = r.metadata.as_ref().and_then(|m| m.duration).unwrap_or(0.0);
    let model = opt_model.unwrap_or(cfg_model);

    let mut text = String::new();
    let mut segments: Vec<Segment> = Vec::new();

    if let Some(res) = r.results {
        if let Some(alt) = res.channels.first().and_then(|c| c.alternatives.first()) {
            text = alt.transcript.clone();
        }
        if let Some(utts) = res.utterances {
            for u in utts {
                let words: Vec<Word> = u
                    .words
                    .iter()
                    .map(|w| Word {
                        text: w.punctuated_word.clone().unwrap_or_else(|| w.word.clone()),
                        start_ms: (w.start * 1000.0) as u32,
                        end_ms: (w.end * 1000.0) as u32,
                        confidence: w.confidence,
                    })
                    .collect();
                segments.push(Segment {
                    text: u.transcript,
                    start_ms: (u.start * 1000.0) as u32,
                    end_ms: (u.end * 1000.0) as u32,
                    words,
                    speaker: u.speaker.map(|id| SpeakerTag { id, label: None }),
                    confidence: u.confidence,
                });
            }
        }
    }

    Transcript {
        text,
        language,
        segments,
        duration_secs: duration,
        backend: BackendKind::Deepgram,
        model_id: Some(model),
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_advertise_streaming_and_diarization() {
        assert!(CAPS.batch);
        assert!(CAPS.streaming_push);
        assert!(matches!(
            CAPS.diarization,
            atomr_agents_stt_core::DiarizationSupport::SpeakerCount
        ));
    }
}
