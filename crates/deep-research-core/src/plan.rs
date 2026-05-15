//! The plan produced by the Planner role.

use serde::{Deserialize, Serialize};

/// Status of a single sub-question.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubQuestionStatus {
    #[default]
    Pending,
    InProgress,
    Answered,
    Unresolved,
}

/// One sub-question of the overall research plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubQuestion {
    /// Stable id used by transcript / citations / coverage signals.
    pub id: String,
    pub text: String,
    /// Free-text rationale for *why* this sub-question matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Outline section this sub-question is assigned to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default)]
    pub status: SubQuestionStatus,
}

impl SubQuestion {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            rationale: None,
            section: None,
            status: SubQuestionStatus::Pending,
        }
    }
}

/// The research plan — outline plus a flat list of sub-questions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Plan {
    /// Top-level outline as an ordered list of section titles.
    #[serde(default)]
    pub outline: Vec<String>,
    /// Sub-questions to answer to fulfill the outline.
    #[serde(default)]
    pub sub_questions: Vec<SubQuestion>,
    /// Free-text rationale for the overall plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Plan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a sub-question by id.
    pub fn sub_question(&self, id: &str) -> Option<&SubQuestion> {
        self.sub_questions.iter().find(|s| s.id == id)
    }

    pub fn sub_question_mut(&mut self, id: &str) -> Option<&mut SubQuestion> {
        self.sub_questions.iter_mut().find(|s| s.id == id)
    }
}
