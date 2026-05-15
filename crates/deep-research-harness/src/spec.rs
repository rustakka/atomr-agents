//! Immutable spec for a deep-research harness instance.

use atomr_agents_core::{HarnessId, TokenBudget};
use semver::Version;

/// Tunable knobs for a run.
#[derive(Debug, Clone)]
pub struct DeepResearchConfig {
    /// Iteration cap as a belt-and-braces guard against runaway loops.
    pub max_iterations: u32,
    /// Optional system-prompt override surfaced to LLM-driven roles.
    pub system_prompt_override: Option<String>,
}

impl Default for DeepResearchConfig {
    fn default() -> Self {
        Self {
            max_iterations: 32,
            system_prompt_override: None,
        }
    }
}

/// Immutable spec for a deep-research harness.
#[derive(Debug, Clone)]
pub struct DeepResearchHarnessSpec {
    pub id: HarnessId,
    pub version: Version,
    /// Model id used by LLM-driven roles. The deterministic defaults
    /// shipped with the crate ignore it but record it on the result.
    pub model_id: Option<String>,
    pub initial_budget: TokenBudget,
    pub config: DeepResearchConfig,
}

impl DeepResearchHarnessSpec {
    pub fn new(id: impl Into<HarnessId>) -> Self {
        Self {
            id: id.into(),
            version: Version::new(0, 1, 0),
            model_id: None,
            initial_budget: TokenBudget::new(0),
            config: DeepResearchConfig::default(),
        }
    }

    pub fn with_model_id(mut self, id: impl Into<String>) -> Self {
        self.model_id = Some(id.into());
        self
    }

    pub fn with_max_iterations(mut self, cap: u32) -> Self {
        self.config.max_iterations = cap;
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt_override = Some(prompt.into());
        self
    }
}
