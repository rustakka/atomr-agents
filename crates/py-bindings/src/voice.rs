//! PyO3 bindings for the higher-level voice-session abstraction.

use std::sync::Arc;

use atomr_agents_stt_voice::{VoiceEvent, VoiceMode, VoiceSession};
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use tokio::sync::Mutex as AsyncMutex;

use crate::conv::json_to_py;
use crate::stt::PyStreamingSession;

#[pyclass(name = "VoiceMode", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyVoiceMode {
    pub(crate) inner: VoiceMode,
}

#[pymethods]
impl PyVoiceMode {
    #[staticmethod]
    fn live() -> Self {
        Self {
            inner: VoiceMode::Live,
        }
    }

    #[staticmethod]
    fn turn_based(silence_ms: u32) -> Self {
        Self {
            inner: VoiceMode::TurnBased { silence_ms },
        }
    }

    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            VoiceMode::Live => "live",
            VoiceMode::TurnBased { .. } => "turn_based",
        }
    }

    #[getter]
    fn silence_ms(&self) -> Option<u32> {
        match self.inner {
            VoiceMode::TurnBased { silence_ms } => Some(silence_ms),
            VoiceMode::Live => None,
        }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            VoiceMode::Live => "VoiceMode.Live".into(),
            VoiceMode::TurnBased { silence_ms } => {
                format!("VoiceMode.TurnBased(silence_ms={silence_ms})")
            }
        }
    }
}

#[pyclass(name = "VoiceEvent", module = "atomr_agents._native.voice")]
#[derive(Clone)]
pub struct PyVoiceEvent {
    pub(crate) inner: VoiceEvent,
}

#[pymethods]
impl PyVoiceEvent {
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            VoiceEvent::PartialTranscript(_) => "partial_transcript",
            VoiceEvent::UserTurn { .. } => "user_turn",
            VoiceEvent::SpeakerChange(_) => "speaker_change",
            VoiceEvent::SilenceDetected { .. } => "silence_detected",
            VoiceEvent::InterimWord(_) => "interim_word",
        }
    }

    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let v = serde_json::to_value(&self.inner)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        json_to_py(py, &v)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            VoiceEvent::PartialTranscript(t) => format!("VoiceEvent.PartialTranscript({t:?})"),
            VoiceEvent::UserTurn { text, .. } => format!("VoiceEvent.UserTurn({text:?})"),
            VoiceEvent::SpeakerChange(s) => format!("VoiceEvent.SpeakerChange(id={})", s.id),
            VoiceEvent::SilenceDetected { duration_ms } => {
                format!("VoiceEvent.SilenceDetected({duration_ms}ms)")
            }
            VoiceEvent::InterimWord(w) => format!("VoiceEvent.InterimWord({:?})", w.text),
        }
    }
}

#[pyclass(name = "VoiceSession", module = "atomr_agents._native.voice")]
pub struct PyVoiceSession {
    pub(crate) inner: Arc<AsyncMutex<VoiceSession>>,
}

#[pymethods]
impl PyVoiceSession {
    /// `VoiceSession.open(streaming_session, mode)` — wrap an active
    /// `StreamingSession` returned by `SpeechToText.open_stream`.
    /// The streaming session is **consumed** by this call; further
    /// method calls on it return a runtime error.
    #[staticmethod]
    fn open(stream: &Bound<'_, PyStreamingSession>, mode: &PyVoiceMode) -> PyResult<Self> {
        let py_stream = stream.borrow();
        let mode = mode.inner;
        // Run on the shared pyo3-async tokio runtime so VoiceSession's
        // constructor (which calls `tokio::spawn`) has a reactor in
        // context.
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let session = rt.block_on(async move {
            let raw = py_stream.take().await?;
            Ok::<VoiceSession, pyo3::PyErr>(VoiceSession::open(raw, mode, None))
        })?;
        Ok(Self {
            inner: Arc::new(AsyncMutex::new(session)),
        })
    }

    /// Open an async iterator over `VoiceEvent`s.
    fn events(&self) -> PyVoiceEventIter {
        PyVoiceEventIter {
            session: self.inner.clone(),
        }
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = inner.lock().await;
            g.close()
                .await
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }

    fn mode(&self) -> PyResult<PyVoiceMode> {
        let inner = self.inner.clone();
        let rt = pyo3_async_runtimes::tokio::get_runtime();
        let m = rt.block_on(async move {
            let g = inner.lock().await;
            g.mode()
        });
        Ok(PyVoiceMode { inner: m })
    }
}

#[pyclass(name = "VoiceEventIter", module = "atomr_agents._native.voice")]
pub struct PyVoiceEventIter {
    session: Arc<AsyncMutex<VoiceSession>>,
}

#[pymethods]
impl PyVoiceEventIter {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session = slf.session.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut g = session.lock().await;
            match g.recv().await {
                Some(Ok(ev)) => Python::with_gil(|py| Py::new(py, PyVoiceEvent { inner: ev })),
                Some(Err(e)) => Err(PyRuntimeError::new_err(e.to_string())),
                None => Err(PyStopAsyncIteration::new_err("")),
            }
        })
    }
}

pub fn register(py: Python<'_>, parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(py, "voice")?;
    m.add_class::<PyVoiceMode>()?;
    m.add_class::<PyVoiceEvent>()?;
    m.add_class::<PyVoiceSession>()?;
    m.add_class::<PyVoiceEventIter>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
