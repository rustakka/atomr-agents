//! Domain events emitted by the deep-research harness.

use atomr_agents_deep_research_core::{Citation, NodeStep, Plan, SubQuestion};
use serde::Serialize;
use tokio::sync::broadcast;

/// One domain event for the deep-research pipeline.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeepResearchEvent {
    Started {
        strategy: String,
        query: String,
    },
    ClarificationsRecorded {
        count: usize,
    },
    PlanComposed {
        plan: Plan,
    },
    SubQuestionStarted {
        sub_question: SubQuestion,
    },
    SubQuestionDone {
        sub_question_id: String,
        hits: u32,
    },
    SearchHitRecorded {
        provider: String,
        url: String,
        title: String,
    },
    DraftSectionAppended {
        heading: String,
        body_chars: usize,
    },
    CitationAppended {
        citation: Citation,
    },
    CritiqueRecorded {
        summary: String,
        gaps: Vec<String>,
    },
    VerificationComplete {
        verified: u32,
        flagged: u32,
    },
    TranscriptStep {
        step: NodeStep,
    },
    Finalized {
        sections: usize,
        citations: usize,
        sub_questions_answered: u32,
    },
    Failed {
        reason: String,
    },
}

/// Subscriber handle.
pub struct DeepResearchEventStream {
    rx: broadcast::Receiver<DeepResearchEvent>,
}

impl DeepResearchEventStream {
    pub(crate) fn new(rx: broadcast::Receiver<DeepResearchEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event. `None` once the channel closes.
    pub async fn recv(&mut self) -> Option<DeepResearchEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
