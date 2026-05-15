//! STT harness — the agentic streaming speech-to-text pipeline.
//!
//! Exposes `atomr_agents._native.stt_harness`:
//!
//! - `SttHarnessSpec` — id + version + diarization / voice-mode policy.
//! - `SttHarness` — built from a spec, a `SpeechToText` backend (from
//!   the `stt` submodule), and an `AudioInput`. `run()` is async and
//!   returns an `SttConversation`; `events()` yields an
//!   `SttEventStream`.
//! - `SttConversation` / `SttTurn` — the diarized transcript record,
//!   with `to_turn_input()` bridging to the agentic turn shape.
//! - `SttEventStream` — async `recv()` over live `SttHarnessEvent`s.

use std::sync::Arc;

use atomr_agents_stt_diarize_sherpa::MockDiarizer;
use atomr_agents_stt_harness::{
    AudioSource, BoxedSttHarness, ConversationStore, DiarizationPolicy, InMemoryConversationStore,
    SpeakerMap, SpeakerRef, StreamEndTermination, StreamingLoop, SttConversation, SttEventStream,
    SttHarnessConfig, SttHarnessSpec, SttTurn, TurnState,
};
use atomr_agents_stt_voice::VoiceMode;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use tokio::sync::Mutex as AsyncMutex;

use crate::conv::{json_to_py, parse_version};

fn diarization_label(policy: &DiarizationPolicy) -> &'static str {
    match policy {
        DiarizationPolicy::Off => "off",
        DiarizationPolicy::Backend => "backend",
        DiarizationPolicy::Layered(_) => "layered",
    }
}

fn voice_mode_label(mode: VoiceMode) -> &'static str {
    match mode {
        VoiceMode::Live => "live",
        VoiceMode::TurnBased { .. } => "turn_based",
    }
}

fn message_role_str(role: atomr_agents_core::MessageRole) -> &'static str {
    use atomr_agents_core::MessageRole;
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

// ----- SttHarnessSpec -----------------------------------------------------

#[pyclass(name = "SttHarnessSpec", module = "atomr_agents._native.stt_harness")]
#[derive(Clone)]
pub struct PySttHarnessSpec {
    pub(crate) inner: SttHarnessSpec,
}

#[pymethods]
impl PySttHarnessSpec {
    /// `diarization`: `"off"`, `"backend"`, or `"layered_mock"` (a
    /// deterministic 2-speaker mock diarizer). `voice_mode`: `"live"`
    /// or `"turn_based"`.
    #[new]
    #[pyo3(signature = (id, version="0.1.0", diarization="backend", voice_mode="turn_based"))]
    fn new(id: String, version: &str, diarization: &str, voice_mode: &str) -> PyResult<Self> {
        let v = parse_version(version)?;
        let diar = match diarization {
            "off" => DiarizationPolicy::Off,
            "backend" => DiarizationPolicy::Backend,
            "layered_mock" => DiarizationPolicy::Layered(Arc::new(MockDiarizer::default())),
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown diarization policy {other:?}; expected off|backend|layered_mock"
                )))
            }
        };
        let vm = match voice_mode {
            "live" => VoiceMode::Live,
            "turn_based" => VoiceMode::TurnBased { silence_ms: 800 },
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown voice mode {other:?}; expected live|turn_based"
                )))
            }
        };
        let mut spec = SttHarnessSpec::new(id);
        spec.version = v;
        spec.config = SttHarnessConfig {
            stream_options: Default::default(),
            voice_mode: vm,
            diarization: diar,
        };
        Ok(Self { inner: spec })
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id.as_str()
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.version.to_string()
    }

    #[getter]
    fn diarization(&self) -> &'static str {
        diarization_label(&self.inner.config.diarization)
    }

    #[getter]
    fn voice_mode(&self) -> &'static str {
        voice_mode_label(self.inner.config.voice_mode)
    }

    fn __repr__(&self) -> String {
        format!(
            "SttHarnessSpec(id={:?}, version={:?}, diarization={:?}, voice_mode={:?})",
            self.inner.id.as_str(),
            self.inner.version.to_string(),
            self.diarization(),
            self.voice_mode(),
        )
    }
}

// ----- SttTurn ------------------------------------------------------------

#[pyclass(name = "SttTurn", module = "atomr_agents._native.stt_harness")]
#[derive(Clone)]
pub struct PySttTurn {
    pub(crate) inner: SttTurn,
}

#[pymethods]
impl PySttTurn {
    #[getter]
    fn index(&self) -> u64 {
        self.inner.index
    }
    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }
    #[getter]
    fn start_ms(&self) -> u32 {
        self.inner.start_ms
    }
    #[getter]
    fn end_ms(&self) -> u32 {
        self.inner.end_ms
    }
    #[getter]
    fn confidence(&self) -> Option<f32> {
        self.inner.confidence
    }
    #[getter]
    fn state(&self) -> &'static str {
        match self.inner.state {
            TurnState::Partial => "partial",
            TurnState::Final => "final",
        }
    }
    #[getter]
    fn speaker_id(&self) -> Option<u8> {
        self.inner.speaker_id()
    }
    #[getter]
    fn speaker_kind(&self) -> &'static str {
        match &self.inner.speaker {
            SpeakerRef::Diarized { .. } => "diarized",
            SpeakerRef::Role { .. } => "role",
            SpeakerRef::Unknown => "unknown",
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "SttTurn(index={}, speaker_id={:?}, text={:?})",
            self.inner.index,
            self.inner.speaker_id(),
            self.inner.text,
        )
    }
}

// ----- SttConversation ----------------------------------------------------

#[pyclass(name = "SttConversation", module = "atomr_agents._native.stt_harness")]
#[derive(Clone)]
pub struct PySttConversation {
    pub(crate) inner: SttConversation,
}

#[pymethods]
impl PySttConversation {
    #[getter]
    fn id(&self) -> &str {
        &self.inner.id
    }
    #[getter]
    fn language(&self) -> Option<String> {
        self.inner.language.clone()
    }
    #[getter]
    fn backend(&self) -> Option<String> {
        self.inner.backend.as_ref().map(|b| b.as_str().to_string())
    }
    #[getter]
    fn model_id(&self) -> Option<String> {
        self.inner.model_id.clone()
    }
    #[getter]
    fn total_audio_secs(&self) -> f32 {
        self.inner.total_audio_secs
    }
    #[getter]
    fn turns(&self) -> Vec<PySttTurn> {
        self.inner
            .turns
            .iter()
            .cloned()
            .map(|t| PySttTurn { inner: t })
            .collect()
    }
    #[getter]
    fn speaker_labels(&self, py: Python<'_>) -> PyResult<PyObject> {
        let d = PyDict::new_bound(py);
        for (id, label) in &self.inner.speaker_labels {
            d.set_item(*id, label)?;
        }
        Ok(d.into())
    }

    /// Distinct diarized speaker ids appearing in the conversation.
    fn speaker_ids(&self) -> Vec<u8> {
        self.inner.speaker_ids()
    }

    /// The effective display label for a speaker id: per-conversation
    /// override, then the `speaker_{id}` fallback.
    fn effective_label(&self, speaker_id: u8) -> String {
        self.inner.effective_label(speaker_id)
    }

    /// Rename a speaker in-place; every turn by that speaker picks the
    /// new label up via [`effective_label`].
    fn rename_speaker(&mut self, speaker_id: u8, label: String) {
        self.inner.rename_speaker(speaker_id, label);
    }

    /// Bridge to the agentic turn shape: the last turn becomes `user`,
    /// the rest become `history`. Returns `None` when there are no
    /// committed turns.
    fn to_turn_input(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        match self.inner.to_turn_input(&SpeakerMap::default()) {
            None => Ok(None),
            Some(ti) => {
                let d = PyDict::new_bound(py);
                d.set_item("user", ti.user)?;
                let history = PyList::empty_bound(py);
                for message in ti.history {
                    let m = PyDict::new_bound(py);
                    m.set_item("role", message_role_str(message.role))?;
                    m.set_item("content", message.content)?;
                    history.append(m)?;
                }
                d.set_item("history", history)?;
                Ok(Some(d.into()))
            }
        }
    }

    /// The full conversation as a plain Python value (JSON-shaped).
    fn to_json(&self, py: Python<'_>) -> PyResult<PyObject> {
        let value = serde_json::to_value(&self.inner)
            .map_err(|e| PyValueError::new_err(format!("serialize conversation: {e}")))?;
        json_to_py(py, &value)
    }

    fn __repr__(&self) -> String {
        format!(
            "SttConversation(id={:?}, turns={})",
            self.inner.id,
            self.inner.turns.len(),
        )
    }
}

// ----- SttEventStream -----------------------------------------------------

#[pyclass(name = "SttEventStream", module = "atomr_agents._native.stt_harness")]
pub struct PySttEventStream {
    inner: Arc<AsyncMutex<SttEventStream>>,
}

#[pymethods]
impl PySttEventStream {
    /// Await the next `SttHarnessEvent` as a dict, or `None` once the
    /// harness has finished.
    fn recv<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let next = {
                let mut guard = stream.lock().await;
                guard.recv().await
            };
            Python::with_gil(|py| match next {
                None => Ok(py.None()),
                Some(event) => {
                    let value = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                    json_to_py(py, &value)
                }
            })
        })
    }

    fn __repr__(&self) -> String {
        "SttEventStream(handle)".into()
    }
}

// ----- SttHarness ---------------------------------------------------------

#[pyclass(name = "SttHarness", module = "atomr_agents._native.stt_harness")]
pub struct PySttHarness {
    inner: Arc<BoxedSttHarness>,
}

#[pymethods]
impl PySttHarness {
    /// Build a harness from a spec, a `SpeechToText` backend (from the
    /// `stt` submodule), and an `AudioInput`. The default streaming
    /// loop + stream-end termination are used.
    #[new]
    fn new(
        spec: PySttHarnessSpec,
        backend: crate::stt::PySpeechToText,
        audio: crate::stt::PyAudioInput,
    ) -> Self {
        let voice_mode = spec.inner.config.voice_mode;
        let loop_strategy = Box::new(StreamingLoop::new(voice_mode));
        let termination = Box::new(StreamEndTermination);
        let source = AudioSource::from(audio.inner);
        let boxed = BoxedSttHarness::new(spec.inner, backend.inner, source, loop_strategy, termination);
        Self {
            inner: Arc::new(boxed),
        }
    }

    /// Subscribe to the live `SttHarnessEvent` stream. Call before
    /// `run()` so no events are missed.
    fn events(&self) -> PySttEventStream {
        PySttEventStream {
            inner: Arc::new(AsyncMutex::new(self.inner.events())),
        }
    }

    /// Drive the pipeline to completion. Async — returns an
    /// `SttConversation`.
    fn run<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let conversation = inner.run().await.map_err(crate::errors::map)?;
            Ok(PySttConversation { inner: conversation })
        })
    }

    fn __repr__(&self) -> String {
        format!("SttHarness(id={:?})", self.inner.spec.id.as_str())
    }
}

// ----- module registration ------------------------------------------------

/// `atomr_agents._native.stt_harness.in_memory_store_demo()` —
/// round-trips a conversation through an [`InMemoryConversationStore`],
/// proving the persistence surface is wired. Returns the stored id.
#[pyfunction]
fn store_roundtrip(conversation: PySttConversation) -> PyResult<String> {
    // A synchronous convenience using a throwaway runtime — the store
    // API is async but this helper keeps the Python smoke test simple.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let id = conversation.inner.id.clone();
    rt.block_on(async move {
        let store = InMemoryConversationStore::new();
        store
            .put(&conversation.inner)
            .await
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        store
            .get(&id)
            .await
            .map_err(|e| PyValueError::new_err(e.to_string()))?
            .ok_or_else(|| PyValueError::new_err("conversation vanished from store"))?;
        Ok::<String, PyErr>(id)
    })
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "stt_harness")?;
    m.add_class::<PySttHarnessSpec>()?;
    m.add_class::<PySttTurn>()?;
    m.add_class::<PySttConversation>()?;
    m.add_class::<PySttEventStream>()?;
    m.add_class::<PySttHarness>()?;
    m.add_function(wrap_pyfunction!(store_roundtrip, &m)?)?;
    parent.add_submodule(&m)?;
    Ok(())
}
