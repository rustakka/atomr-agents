//! Static, cloneable description of an STT harness instance.
//!
//! [`SttHarnessSpec`] is to [`crate::SttHarness`] what
//! `atomr_agents_harness::HarnessSpec` is to its `Harness` — an
//! immutable config plus an [`SttHarnessSpec::into_harness`]
//! constructor that materializes a runnable, type-erased
//! [`crate::SttHarnessRef`].

use std::fmt;
use std::sync::Arc;

use atomr_agents_core::{HarnessId, TokenBudget};
use atomr_agents_stt_core::StreamOptions;
use atomr_agents_stt_diarize_sherpa::Diarizer;
use atomr_agents_stt_voice::VoiceMode;
use semver::Version;

use crate::audio_source::AudioSource;
use crate::boxed::BoxedSttHarness;
use crate::dispatch::SttHarnessRef;
use crate::loop_strategy::SttLoopStrategy;
use crate::termination::SttTermination;
use atomr_agents_stt_core::DynSpeechToText;

/// How the harness attributes speakers.
#[derive(Clone, Default)]
pub enum DiarizationPolicy {
    /// Do not diarize; turns carry no speaker.
    Off,
    /// Trust the backend's own speaker tags (for backends whose
    /// `Capabilities::diarization` is not `None`).
    #[default]
    Backend,
    /// Layer a local [`Diarizer`] over the audio: the harness retains
    /// the utterance PCM, runs the diarizer when the utterance
    /// commits, and stitches the spans onto the segment.
    Layered(Arc<dyn Diarizer>),
}

impl fmt::Debug for DiarizationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiarizationPolicy::Off => f.write_str("Off"),
            DiarizationPolicy::Backend => f.write_str("Backend"),
            DiarizationPolicy::Layered(_) => f.write_str("Layered(<diarizer>)"),
        }
    }
}

/// Tunable knobs for an STT harness run.
#[derive(Debug, Clone)]
pub struct SttHarnessConfig {
    /// Options passed to `SpeechToText::open_stream`. The harness fills
    /// in `format` from the audio source if it is `None`.
    pub stream_options: StreamOptions,
    /// `Live` surfaces partials as events; `TurnBased` buffers them.
    pub voice_mode: VoiceMode,
    /// Speaker-attribution policy.
    pub diarization: DiarizationPolicy,
}

impl Default for SttHarnessConfig {
    fn default() -> Self {
        Self {
            stream_options: StreamOptions::default(),
            voice_mode: VoiceMode::default(),
            diarization: DiarizationPolicy::default(),
        }
    }
}

/// Immutable spec for an STT harness.
#[derive(Debug, Clone)]
pub struct SttHarnessSpec {
    pub id: HarnessId,
    pub version: Version,
    pub eval_suite_id: Option<String>,
    /// Token-shaped budget proxy; see [`crate::SttHarnessState`].
    pub initial_budget: TokenBudget,
    pub config: SttHarnessConfig,
}

impl SttHarnessSpec {
    /// A spec with sensible defaults under the given id.
    pub fn new(id: impl Into<HarnessId>) -> Self {
        Self {
            id: id.into(),
            version: Version::new(0, 1, 0),
            eval_suite_id: None,
            initial_budget: TokenBudget::new(0),
            config: SttHarnessConfig::default(),
        }
    }

    /// Builder: set the diarization policy.
    pub fn with_diarization(mut self, policy: DiarizationPolicy) -> Self {
        self.config.diarization = policy;
        self
    }

    /// Builder: set the voice mode.
    pub fn with_voice_mode(mut self, mode: VoiceMode) -> Self {
        self.config.voice_mode = mode;
        self
    }

    /// Materialize a runnable, type-erased [`SttHarnessRef`] from this
    /// spec plus concrete runtime pieces. Both strategies are passed
    /// boxed so callers without the concrete generic types (Python
    /// loaders, registries) can construct one — mirrors
    /// `HarnessSpec::into_harness`.
    pub fn into_harness(
        self,
        backend: DynSpeechToText,
        audio: AudioSource,
        loop_strategy: Box<dyn SttLoopStrategy>,
        termination: Box<dyn SttTermination>,
    ) -> SttHarnessRef {
        let id = self.id.clone();
        let boxed = BoxedSttHarness::new(self, backend, audio, loop_strategy, termination);
        SttHarnessRef::new(id, Arc::new(boxed))
    }
}
