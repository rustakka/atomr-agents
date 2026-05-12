//! PyO3 bindings for the speech-to-text capability.
//!
//! Step-1 surface: `Capabilities` (round-tripped to a Python dict
//! via `serde_json` → `json.loads`), `AudioInput` factory functions,
//! `Transcript`, `SpeechToText`, `StreamingSession` (async iterator
//! pattern mirroring `PyEventStream` in `observability.rs`), and a
//! `mock_speech_to_text()` constructor that exercises the full FFI
//! shape without any backend.
//!
//! Backend constructors (`stt_openai`, `stt_deepgram`, …) are added
//! in step 12 after the runtime crates land.

use std::path::PathBuf;
use std::sync::Arc;

use atomr_agents_stt_core::{
    AudioFormat, AudioInput, Capabilities, DynSpeechToText, MockSpeechToText, PcmBuffer,
    StreamEvent, StreamOptions, StreamingSession, TranscribeOptions, Transcript,
};
use bytes::Bytes;
use futures::StreamExt;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::Mutex as AsyncMutex;

use atomr_agents_stt_tool::{voice_input_skill as build_voice_input_skill, TranscribeTool};
use atomr_agents_tool::Tool;

use crate::conv::{json_to_py, py_to_json};
use crate::skill::PySkill;
use crate::tool::PyToolDescriptor;

// ----- Capabilities ---------------------------------------------------------

#[pyclass(name = "Capabilities", module = "atomr_agents._native.stt")]
#[derive(Clone)]
pub struct PyCapabilities {
    pub(crate) inner: Capabilities,
}

#[pymethods]
impl PyCapabilities {
    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    #[getter]
    fn batch(&self) -> bool {
        self.inner.batch
    }
    #[getter]
    fn streaming_push(&self) -> bool {
        self.inner.streaming_push
    }
    #[getter]
    fn realtime_microphone(&self) -> bool {
        self.inner.realtime_microphone
    }
    #[getter]
    fn diarization(&self) -> &'static str {
        match self.inner.diarization {
            atomr_agents_stt_core::DiarizationSupport::None => "none",
            atomr_agents_stt_core::DiarizationSupport::SpeakerCount => "speaker_count",
            atomr_agents_stt_core::DiarizationSupport::NamedSpeakers => "named_speakers",
        }
    }
    #[getter]
    fn word_timestamps(&self) -> bool {
        self.inner.word_timestamps
    }
    #[getter]
    fn language_detection(&self) -> bool {
        self.inner.language_detection
    }
    #[getter]
    fn requires_network(&self) -> bool {
        self.inner.requires_network
    }
    #[getter]
    fn partial_results(&self) -> bool {
        self.inner.partial_results
    }
    #[getter]
    fn vad_endpointing(&self) -> bool {
        self.inner.vad_endpointing
    }
    #[getter]
    fn cost_per_audio_min_usd(&self) -> Option<f32> {
        self.inner.cost_per_audio_min_usd
    }
    #[getter]
    fn max_audio_secs(&self) -> Option<u32> {
        self.inner.max_audio_secs
    }
    #[getter]
    fn min_chunk_ms(&self) -> Option<u32> {
        self.inner.min_chunk_ms
    }

    fn __repr__(&self) -> String {
        format!(
            "Capabilities(batch={}, streaming={}, mic={}, diarization={:?})",
            self.inner.batch,
            self.inner.streaming_push,
            self.inner.realtime_microphone,
            self.inner.diarization,
        )
    }
}

// ----- AudioInput -----------------------------------------------------------

#[pyclass(name = "AudioInput", module = "atomr_agents._native.stt")]
#[derive(Clone)]
pub struct PyAudioInput {
    pub(crate) inner: AudioInput,
}

fn parse_format(s: &str) -> PyResult<AudioFormat> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "wav" => AudioFormat::Wav,
        "mp3" => AudioFormat::Mp3,
        "flac" => AudioFormat::Flac,
        "ogg" => AudioFormat::Ogg,
        "opus" => AudioFormat::Opus,
        "webm" => AudioFormat::Webm,
        "mp4" | "m4a" => AudioFormat::Mp4,
        "aac" => AudioFormat::Aac,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown audio format {other:?}"
            )));
        }
    })
}

/// `audio_file(path)` — wrap a path on disk as an `AudioInput`.
#[pyfunction]
pub fn audio_file(path: PathBuf) -> PyAudioInput {
    PyAudioInput {
        inner: AudioInput::File(path),
    }
}

/// `audio_bytes(data, format)` — wrap an in-memory buffer.
/// `format` is one of `"wav"`, `"mp3"`, `"flac"`, `"ogg"`, `"opus"`,
/// `"webm"`, `"mp4"`/`"m4a"`, `"aac"`.
#[pyfunction]
pub fn audio_bytes(data: &Bound<'_, PyBytes>, format: &str) -> PyResult<PyAudioInput> {
    let fmt = parse_format(format)?;
    Ok(PyAudioInput {
        inner: AudioInput::Bytes {
            data: Bytes::copy_from_slice(data.as_bytes()),
            format: fmt,
        },
    })
}

/// `audio_pcm(samples, sample_rate, channels)` — wrap an
/// already-decoded f32 PCM buffer (mono or interleaved).
#[pyfunction]
pub fn audio_pcm(samples: Vec<f32>, sample_rate: u32, channels: u16) -> PyAudioInput {
    PyAudioInput {
        inner: AudioInput::Pcm(PcmBuffer::new(samples, sample_rate, channels)),
    }
}

// ----- Transcript -----------------------------------------------------------

#[pyclass(name = "Transcript", module = "atomr_agents._native.stt")]
#[derive(Clone)]
pub struct PyTranscript {
    pub(crate) inner: Transcript,
}

#[pymethods]
impl PyTranscript {
    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }
    #[getter]
    fn language(&self) -> Option<String> {
        self.inner.language.clone()
    }
    #[getter]
    fn duration_secs(&self) -> f32 {
        self.inner.duration_secs
    }
    #[getter]
    fn backend(&self) -> String {
        self.inner.backend.as_str().to_string()
    }
    #[getter]
    fn model_id(&self) -> Option<String> {
        self.inner.model_id.clone()
    }
    #[getter]
    fn cost_usd(&self) -> Option<f32> {
        self.inner.cost_usd
    }

    #[getter]
    fn segments(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner.segments)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        let preview: String = self.inner.text.chars().take(40).collect();
        format!(
            "Transcript(backend={:?}, lang={:?}, secs={:.2}, text={:?})",
            self.inner.backend.as_str(),
            self.inner.language,
            self.inner.duration_secs,
            preview,
        )
    }
}

// ----- StreamEvent ----------------------------------------------------------

#[pyclass(name = "StreamEvent", module = "atomr_agents._native.stt")]
#[derive(Clone)]
pub struct PyStreamEvent {
    pub(crate) inner: StreamEvent,
}

#[pymethods]
impl PyStreamEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            StreamEvent::Partial { .. } => "partial",
            StreamEvent::Final { .. } => "final",
            StreamEvent::SpeakerTurn { .. } => "speaker_turn",
            StreamEvent::UtteranceEnd { .. } => "utterance_end",
            StreamEvent::Metadata(_) => "metadata",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            StreamEvent::Partial { text, .. } => format!("StreamEvent.Partial({text:?})"),
            StreamEvent::Final { segment } => {
                format!("StreamEvent.Final({:?})", segment.text)
            }
            StreamEvent::SpeakerTurn { speaker, at_ms } => {
                format!("StreamEvent.SpeakerTurn(id={}, at_ms={})", speaker.id, at_ms)
            }
            StreamEvent::UtteranceEnd { at_ms } => {
                format!("StreamEvent.UtteranceEnd(at_ms={at_ms})")
            }
            StreamEvent::Metadata(_) => "StreamEvent.Metadata(...)".to_string(),
        }
    }
}

// ----- StreamingSession + async iterator -----------------------------------

/// Wraps a Rust `StreamingSession` behind an async `Mutex` so the
/// Python class can hand out async methods that don't need `&mut`
/// synchronization across the FFI boundary.
///
/// The inner `Option` allows `VoiceSession.open` to consume the
/// underlying box and move it into the voice-session task; calling
/// any further method on a consumed `StreamingSession` returns a
/// runtime error.
#[pyclass(name = "StreamingSession", module = "atomr_agents._native.stt")]
pub struct PyStreamingSession {
    pub(crate) inner: Arc<AsyncMutex<Option<Box<dyn StreamingSession>>>>,
}

impl PyStreamingSession {
    pub(crate) fn new(session: Box<dyn StreamingSession>) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(session))),
        }
    }

    /// Consume the underlying session, leaving `None` behind.
    /// Returns `Err` if the session was already consumed.
    pub(crate) async fn take(&self) -> PyResult<Box<dyn StreamingSession>> {
        let mut g = self.inner.lock().await;
        g.take().ok_or_else(|| {
            PyRuntimeError::new_err("StreamingSession already consumed (e.g. by VoiceSession.open)")
        })
    }
}

fn consumed_err() -> pyo3::PyErr {
    PyRuntimeError::new_err("StreamingSession was consumed (e.g. by VoiceSession.open)")
}

#[pymethods]
impl PyStreamingSession {
    fn capabilities(&self) -> PyResult<PyCapabilities> {
        let inner = self.inner.clone();
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let caps = rt
            .block_on(async move {
                let guard = inner.lock().await;
                guard.as_ref().map(|s| s.capabilities().clone())
            })
            .ok_or_else(consumed_err)?;
        Ok(PyCapabilities { inner: caps })
    }

    fn push_audio<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'py, PyBytes>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let bytes = Bytes::copy_from_slice(data.as_bytes());
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let s = guard.as_mut().ok_or_else(consumed_err)?;
            s.push_audio(bytes)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn finish<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let s = guard.as_mut().ok_or_else(consumed_err)?;
            s.finish()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            if let Some(s) = guard.as_mut() {
                s.close()
                    .await
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            }
            Ok(())
        })
    }

    /// Open an async iterator over events. Each `__anext__` pulls one
    /// `StreamEvent` from the underlying session's stream.
    fn events(&self) -> PyStreamEventIter {
        PyStreamEventIter {
            session: self.inner.clone(),
        }
    }
}

#[pyclass(name = "StreamEventIter", module = "atomr_agents._native.stt")]
pub struct PyStreamEventIter {
    session: Arc<AsyncMutex<Option<Box<dyn StreamingSession>>>>,
}

#[pymethods]
impl PyStreamEventIter {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = slf.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = session.lock().await;
            let s = guard.as_mut().ok_or_else(consumed_err)?;
            let mut stream = s.events();
            match stream.next().await {
                Some(Ok(ev)) => {
                    drop(stream);
                    Python::with_gil(|py| Py::new(py, PyStreamEvent { inner: ev }))
                }
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

// ----- SpeechToText (instance) ---------------------------------------------

#[pyclass(name = "SpeechToText", module = "atomr_agents._native.stt")]
#[derive(Clone)]
pub struct PySpeechToText {
    pub(crate) inner: DynSpeechToText,
}

#[pymethods]
impl PySpeechToText {
    fn capabilities(&self) -> PyCapabilities {
        PyCapabilities {
            inner: self.inner.capabilities().clone(),
        }
    }

    fn backend_kind(&self) -> String {
        self.inner.backend_kind().as_str().to_string()
    }

    fn transport_kind(&self) -> String {
        self.inner.transport_kind().as_str().to_string()
    }

    /// `transcribe(input, language=None, model=None, diarize=False, ...)`.
    #[pyo3(signature = (
        input, *, language=None, model=None, diarize=false,
        punctuation=true, profanity_filter=false, keywords=None,
        initial_prompt=None, extra=None
    ))]
    fn transcribe<'py>(
        &self,
        py: Python<'py>,
        input: PyAudioInput,
        language: Option<String>,
        model: Option<String>,
        diarize: bool,
        punctuation: bool,
        profanity_filter: bool,
        keywords: Option<Vec<String>>,
        initial_prompt: Option<String>,
        extra: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let opts = TranscribeOptions {
            language,
            model,
            diarize,
            punctuation,
            profanity_filter,
            keywords: keywords.unwrap_or_default(),
            initial_prompt,
            extra: extra.map(|e| py_to_json(py, e)).transpose()?,
        };
        let stt = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let t = stt
                .transcribe(input.inner, opts)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyTranscript { inner: t }))
        })
    }

    #[pyo3(signature = (
        *, format=None, language=None, diarize=false, model=None, extra=None
    ))]
    fn open_stream<'py>(
        &self,
        py: Python<'py>,
        format: Option<&str>,
        language: Option<String>,
        diarize: bool,
        model: Option<String>,
        extra: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let opts = StreamOptions {
            format: format.map(parse_format).transpose()?,
            language,
            diarize,
            model,
            extra: extra.map(|e| py_to_json(py, e)).transpose()?,
        };
        let stt = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let session = stt
                .open_stream(opts)
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyStreamingSession::new(session)))
        })
    }
}

// ----- Free constructors ----------------------------------------------------

/// `mock_speech_to_text(text=None, language="en")` — deterministic
/// in-process backend used by tests.
#[pyfunction]
#[pyo3(signature = (text=None, language=None))]
pub fn mock_speech_to_text(text: Option<String>, language: Option<String>) -> PySpeechToText {
    let mut m = MockSpeechToText::new();
    if let Some(t) = text {
        m = m.with_text(t);
    }
    if let Some(l) = language {
        m = m.with_language(l);
    }
    PySpeechToText {
        inner: Arc::new(m),
    }
}

// ----- Backend constructors -------------------------------------------------

fn secret_ref_from(api_key: &str) -> atomr_agents_stt_remote_core::SecretRef {
    if let Some(env_name) = api_key.strip_prefix("env:") {
        atomr_agents_stt_remote_core::SecretRef::env(env_name)
    } else if let Some(file_path) = api_key.strip_prefix("file:") {
        atomr_agents_stt_remote_core::SecretRef::file(std::path::PathBuf::from(file_path))
    } else {
        atomr_agents_stt_remote_core::SecretRef::literal(api_key)
    }
}

/// `stt_openai(api_key, *, model=None, organization=None, language=None)`.
/// `api_key` accepts a literal key, `"env:VARNAME"`, or `"file:/path"`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, organization=None, language=None))]
pub fn stt_openai(
    api_key: &str,
    model: Option<String>,
    organization: Option<String>,
    language: Option<String>,
) -> PyResult<PySpeechToText> {
    let mut cfg = atomr_agents_stt_runtime_openai::OpenAiSttConfig::from_env();
    cfg.api_key = secret_ref_from(api_key);
    if let Some(m) = model {
        cfg.default_model = m;
    }
    cfg.organization = organization;
    cfg.default_language = language;
    let runner = atomr_agents_stt_runtime_openai::OpenAiSttRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PySpeechToText {
        inner: Arc::new(runner),
    })
}

/// `stt_deepgram(api_key, *, model=None, language=None)`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, language=None))]
pub fn stt_deepgram(
    api_key: &str,
    model: Option<String>,
    language: Option<String>,
) -> PyResult<PySpeechToText> {
    let mut cfg = atomr_agents_stt_runtime_deepgram::DeepgramConfig::from_env();
    cfg.api_key = secret_ref_from(api_key);
    if let Some(m) = model {
        cfg.default_model = m;
    }
    cfg.default_language = language;
    let runner = atomr_agents_stt_runtime_deepgram::DeepgramRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PySpeechToText {
        inner: Arc::new(runner),
    })
}

/// `stt_assemblyai(api_key, *, model=None, language=None, speaker_labels=False)`.
#[pyfunction]
#[pyo3(signature = (api_key, *, model=None, language=None, speaker_labels=false))]
pub fn stt_assemblyai(
    api_key: &str,
    model: Option<String>,
    language: Option<String>,
    speaker_labels: bool,
) -> PyResult<PySpeechToText> {
    let mut cfg = atomr_agents_stt_runtime_assemblyai::AssemblyAiConfig::from_env();
    cfg.api_key = secret_ref_from(api_key);
    if let Some(m) = model {
        cfg.default_model = m;
    }
    cfg.default_language = language;
    cfg.default_speaker_labels = speaker_labels;
    let runner = atomr_agents_stt_runtime_assemblyai::AssemblyAiRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PySpeechToText {
        inner: Arc::new(runner),
    })
}

/// `stt_whisper(model_path, *, language=None, n_threads=None, gpu=False, beam_size=1)`.
/// Without the `whisper-cpp` Cargo feature, calling `transcribe`
/// returns a typed `ModelLoad` error pointing at the missing
/// feature. The constructor still validates inputs.
#[pyfunction]
#[pyo3(signature = (
    model_path, *, language=None, n_threads=None, gpu=false, beam_size=1
))]
pub fn stt_whisper(
    model_path: PathBuf,
    language: Option<String>,
    n_threads: Option<u16>,
    gpu: bool,
    beam_size: u16,
) -> PyResult<PySpeechToText> {
    let mut cfg = atomr_agents_stt_runtime_whisper::WhisperConfig::new(model_path);
    if let Some(n) = n_threads {
        cfg.n_threads = n;
    }
    cfg.gpu = gpu;
    cfg.default_language = language;
    cfg.beam_size = beam_size;
    let runner = atomr_agents_stt_runtime_whisper::WhisperRunner::new(cfg)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(PySpeechToText {
        inner: Arc::new(runner),
    })
}

// ----- Tool + Skill factories ----------------------------------------------

/// `transcribe_tool(stt_handle, options=None) -> ToolDescriptor`.
///
/// Wraps an `stt-tool::TranscribeTool` and returns its `ToolDescriptor`
/// so the result can flow through the rest of the Python tool surface
/// (e.g. `static_tool_strategy([transcribe_tool(stt)])`). The returned
/// descriptor is "descriptor-only" — wiring up an executable Tool that
/// the harness can actually call belongs in step 14 once the
/// guest-tool surface accepts native-built `DynTool`s.
///
/// `options` is reserved for future per-invocation defaults (language,
/// diarize, …). It is currently ignored; pass `None`.
#[pyfunction]
#[pyo3(signature = (stt_handle, options=None))]
pub fn transcribe_tool(
    stt_handle: PySpeechToText,
    options: Option<&Bound<'_, PyAny>>,
) -> PyToolDescriptor {
    let _ = options; // accepted for forward compat; see doc comment
    let tool = TranscribeTool::new(stt_handle.inner);
    PyToolDescriptor {
        inner: Tool::descriptor(&tool).clone(),
    }
}

/// `voice_input_skill(stt_handle) -> Skill`.
///
/// Returns the packaged Skill produced by the Rust
/// `stt-tool::voice_input_skill` helper — instruction fragment +
/// `transcribe_audio` tool overlay. The companion `TranscribeTool`
/// itself is descriptor-only at the Python surface for now; the Skill
/// only needs the `ToolId` to declare its overlay, which is what this
/// binding exposes.
#[pyfunction]
pub fn voice_input_skill(stt_handle: PySpeechToText) -> PySkill {
    let (skill, _tool) = build_voice_input_skill(stt_handle.inner);
    PySkill { inner: skill }
}

// ----- Module registration --------------------------------------------------

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "stt")?;
    m.add_class::<PyCapabilities>()?;
    m.add_class::<PyAudioInput>()?;
    m.add_class::<PyTranscript>()?;
    m.add_class::<PyStreamEvent>()?;
    m.add_class::<PyStreamingSession>()?;
    m.add_class::<PyStreamEventIter>()?;
    m.add_class::<PySpeechToText>()?;
    m.add_function(wrap_pyfunction!(audio_file, &m)?)?;
    m.add_function(wrap_pyfunction!(audio_bytes, &m)?)?;
    m.add_function(wrap_pyfunction!(audio_pcm, &m)?)?;
    m.add_function(wrap_pyfunction!(mock_speech_to_text, &m)?)?;
    m.add_function(wrap_pyfunction!(stt_openai, &m)?)?;
    m.add_function(wrap_pyfunction!(stt_deepgram, &m)?)?;
    m.add_function(wrap_pyfunction!(stt_assemblyai, &m)?)?;
    m.add_function(wrap_pyfunction!(stt_whisper, &m)?)?;
    m.add_function(wrap_pyfunction!(transcribe_tool, &m)?)?;
    m.add_function(wrap_pyfunction!(voice_input_skill, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
