//! Uniform research request — the input every strategy accepts.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::scope::ResearchScope;

/// Markdown body alias used by the report output.
pub type Markdown = String;

/// One turn of a user/assistant clarification exchange. When the
/// caller already has the answers they may pre-populate
/// [`ResearchRequest::clarifications`] so the `Clarifier` role short-
/// circuits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationTurn {
    pub question: String,
    pub answer: String,
}

/// Human-in-the-loop policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitlPolicy {
    /// The Clarifier role auto-derives answers from the request scope.
    #[default]
    AutoClarify,
    /// Stop once after composing clarifying questions and wait for the
    /// caller to supply answers before re-running.
    AskOnce,
    /// Stop at every planner/critic loop and require human approval.
    AskEveryRound,
    /// Never ask the user; never stop.
    Off,
}

/// Desired report format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputFormat {
    /// GitHub-flavored markdown with numbered citation markers.
    Markdown {
        /// Optional path/name of a template the writer should follow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    /// Plain text body; no markdown.
    Plain,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::Markdown { template: None }
    }
}

/// Per-role LLM model id overrides. Strategies pick the model named for
/// each role via [`LlmOverrides::role_model`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmOverrides {
    /// Default model id when a role isn't specifically overridden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Role → model id (e.g. `"researcher" -> "claude-sonnet-4-6"`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_role: BTreeMap<String, String>,
}

impl LlmOverrides {
    pub fn role_model(&self, role: &str) -> Option<&str> {
        self.per_role
            .get(role)
            .or(self.default_model.as_ref())
            .map(|s| s.as_str())
    }
}

/// Uniform input across every research topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRequest {
    /// Free-text question the user wants answered.
    pub query: String,

    /// Optional pre-supplied clarification turns. Empty on first call.
    #[serde(default)]
    pub clarifications: Vec<ClarificationTurn>,

    /// Sources, allowed domains, and attachments.
    #[serde(default)]
    pub scope: ResearchScope,

    /// Max planner/critic refinement rounds. Strategies cap their
    /// inner loops at this number.
    #[serde(default = "default_depth")]
    pub depth: u32,

    /// Max parallel sub-questions per round.
    #[serde(default = "default_breadth")]
    pub breadth: u32,

    /// Optional time budget for the whole run.
    #[serde(default, with = "duration_secs_opt", skip_serializing_if = "Option::is_none")]
    pub time_budget: Option<Duration>,

    /// Optional token cap (string instead of `TokenBudget` so this
    /// crate stays free of an `atomr-agents-core` dependency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,

    /// If non-empty, only tools whose name appears here may be invoked
    /// by an LLM-driven role.
    #[serde(default)]
    pub tools_allowlist: Vec<String>,

    /// Desired report format.
    #[serde(default)]
    pub output_format: OutputFormat,

    /// Per-role model overrides.
    #[serde(default)]
    pub llm_overrides: LlmOverrides,

    /// Human-in-the-loop policy.
    #[serde(default)]
    pub human_in_the_loop: HitlPolicy,
}

impl ResearchRequest {
    /// Build a default request for the given query.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            clarifications: Vec::new(),
            scope: ResearchScope::default(),
            depth: default_depth(),
            breadth: default_breadth(),
            time_budget: None,
            token_budget: None,
            tools_allowlist: Vec::new(),
            output_format: OutputFormat::default(),
            llm_overrides: LlmOverrides::default(),
            human_in_the_loop: HitlPolicy::default(),
        }
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_breadth(mut self, breadth: u32) -> Self {
        self.breadth = breadth;
        self
    }

    pub fn with_scope(mut self, scope: ResearchScope) -> Self {
        self.scope = scope;
        self
    }
}

fn default_depth() -> u32 {
    2
}
fn default_breadth() -> u32 {
    3
}

mod duration_secs_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&d.as_secs()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let v: Option<u64> = Option::deserialize(d)?;
        Ok(v.map(Duration::from_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_json() {
        let mut r = ResearchRequest::new("compare actor frameworks");
        r.depth = 3;
        r.breadth = 5;
        r.clarifications.push(ClarificationTurn {
            question: "what languages?".into(),
            answer: "rust".into(),
        });
        r.llm_overrides
            .per_role
            .insert("researcher".into(), "claude-sonnet-4-6".into());
        r.llm_overrides.default_model = Some("claude-opus-4-7".into());
        let j = serde_json::to_string(&r).unwrap();
        let back: ResearchRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(back.query, r.query);
        assert_eq!(back.depth, 3);
        assert_eq!(back.breadth, 5);
        assert_eq!(back.clarifications.len(), 1);
        assert_eq!(
            back.llm_overrides.role_model("researcher"),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(back.llm_overrides.role_model("writer"), Some("claude-opus-4-7"));
    }

    #[test]
    fn default_hitl_is_auto_clarify() {
        assert_eq!(HitlPolicy::default(), HitlPolicy::AutoClarify);
    }
}
