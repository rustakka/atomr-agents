//! Immutable spec for a meetings harness instance.

use atomr_agents_core::{HarnessId, TokenBudget};
use semver::Version;

use crate::analysis::RunMode;

/// Tunable knobs for a meetings run.
#[derive(Debug, Clone)]
pub struct MeetingsHarnessConfig {
    /// Run mode — batch (default) or live.
    pub mode: RunMode,
    /// Max iterations of the extraction loop before forced
    /// termination. Belt-and-braces with [`IterationCapTermination`].
    pub max_iterations: u32,
    /// Optional override of the extractor's system prompt. When
    /// supplied, it is passed through to extractors that consume one.
    pub system_prompt_override: Option<String>,
    /// Auto-trigger configuration. Defaults to manual-only.
    pub auto_trigger: AutoTriggerCfg,
}

impl Default for MeetingsHarnessConfig {
    fn default() -> Self {
        Self {
            mode: RunMode::default(),
            max_iterations: 32,
            system_prompt_override: None,
            auto_trigger: AutoTriggerCfg::default(),
        }
    }
}

/// Whether the harness should auto-start when the upstream STT harness
/// signals readiness. Manual is the default; opt in by setting
/// [`AutoTriggerCfg::enabled`] = `true` and choosing a mode.
#[derive(Debug, Clone, Default)]
pub struct AutoTriggerCfg {
    /// When `true`, callers wired into the STT broadcast may kick off
    /// the meetings run automatically on `Started`.
    pub enabled: bool,
}

/// Immutable spec for a meetings harness.
#[derive(Debug, Clone)]
pub struct MeetingsHarnessSpec {
    pub id: HarnessId,
    pub version: Version,
    /// Required: the LLM model id used by an agentic extractor. The
    /// crate's default rule-based extractor leaves it unused, but it is
    /// recorded on the resulting [`crate::MeetingAnalysis`].
    pub model_id: String,
    /// Token-shaped budget proxy.
    pub initial_budget: TokenBudget,
    pub config: MeetingsHarnessConfig,
}

impl MeetingsHarnessSpec {
    /// A spec with sensible defaults under the given id and model id.
    /// The model id has no default — the caller must supply one.
    pub fn new(id: impl Into<HarnessId>, model_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: Version::new(0, 1, 0),
            model_id: model_id.into(),
            initial_budget: TokenBudget::new(0),
            config: MeetingsHarnessConfig::default(),
        }
    }

    /// Builder: set run mode.
    pub fn with_mode(mut self, mode: RunMode) -> Self {
        self.config.mode = mode;
        self
    }

    /// Builder: set the iteration cap.
    pub fn with_max_iterations(mut self, cap: u32) -> Self {
        self.config.max_iterations = cap;
        self
    }

    /// Builder: opt into auto-trigger.
    pub fn with_auto_trigger(mut self, enabled: bool) -> Self {
        self.config.auto_trigger.enabled = enabled;
        self
    }

    /// Builder: override the extractor's system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt_override = Some(prompt.into());
        self
    }
}
