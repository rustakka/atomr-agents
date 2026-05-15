//! NVIDIA AI-Q-style strategy: clarify → plan → search → write → verify.

use async_trait::async_trait;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::{ClarifyOutcome, CritiqueOutcome};

/// Stage of the AI-Q pipeline this strategy is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    Research,
    Write,
    Critique,
    Verify,
    Done,
}

/// Loop strategy that walks the canonical
/// clarify → plan → research → write → critique → verify pipeline.
pub struct ClarifyPlanSearchVerifyLoop {
    state: parking_lot::Mutex<LoopState>,
}

struct LoopState {
    stage: Stage,
    refinement_rounds: u32,
    /// Sub-question id last seen as `Answered` — used to drive forward
    /// progress on subsequent iterations.
    next_sub_question_idx: usize,
}

impl Default for ClarifyPlanSearchVerifyLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl ClarifyPlanSearchVerifyLoop {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(LoopState {
                stage: Stage::Clarify,
                refinement_rounds: 0,
                next_sub_question_idx: 0,
            }),
        }
    }
}

#[async_trait]
impl DeepResearchLoopStrategy for ClarifyPlanSearchVerifyLoop {
    fn name(&self) -> &str {
        "clarify-plan-search-verify"
    }

    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
        // Snapshot loop state under the lock, then drop the guard
        // before any await.
        let stage = {
            let s = self.state.lock();
            s.stage
        };

        match stage {
            Stage::Clarify => {
                let request = ctx.handle.request();
                let outcome = ctx.clarifier.clarify(&request, ctx.handle).await?;
                match outcome {
                    ClarifyOutcome::Ready => {
                        self.state.lock().stage = Stage::Plan;
                        Ok(DeepResearchStepOutcome::Continue {
                            label: "clarified".into(),
                        })
                    }
                    ClarifyOutcome::NeedAnswers { questions } => {
                        for q in questions {
                            ctx.handle
                                .record_clarification(q, "(awaiting human input)".into());
                        }
                        Ok(DeepResearchStepOutcome::Done {
                            label: "awaiting_clarifications".into(),
                        })
                    }
                }
            }
            Stage::Plan => {
                let request = ctx.handle.request();
                let plan = ctx.planner.plan(&request, ctx.handle).await?;
                ctx.handle.set_plan(plan);
                self.state.lock().stage = Stage::Research;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::Research => {
                let snap = ctx.handle.snapshot();
                let plan = snap.plan.clone().unwrap_or_default();
                let idx = self.state.lock().next_sub_question_idx;
                if idx >= plan.sub_questions.len() {
                    self.state.lock().stage = Stage::Write;
                    return Ok(DeepResearchStepOutcome::Continue {
                        label: "research_done".into(),
                    });
                }
                let sub = plan.sub_questions[idx].clone();
                ctx.researcher.research(&sub, ctx.handle).await?;
                {
                    let mut s = self.state.lock();
                    s.next_sub_question_idx = idx + 1;
                }
                Ok(DeepResearchStepOutcome::Continue {
                    label: format!("researched:{}", sub.id),
                })
            }
            Stage::Write => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                ctx.writer.write(&plan, ctx.handle).await?;
                self.state.lock().stage = Stage::Critique;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "wrote".into(),
                })
            }
            Stage::Critique => {
                let request = ctx.handle.request();
                let CritiqueOutcome { done, gaps, .. } = ctx.critic.critique(ctx.handle).await?;
                let max_depth = request.depth;
                let mut s = self.state.lock();
                if done || s.refinement_rounds >= max_depth {
                    s.stage = Stage::Verify;
                    Ok(DeepResearchStepOutcome::Continue {
                        label: "critique_done".into(),
                    })
                } else {
                    s.refinement_rounds += 1;
                    s.next_sub_question_idx = 0;
                    s.stage = Stage::Research;
                    drop(s);
                    // Append follow-up sub-questions for each gap.
                    let mut idx_offset = ctx
                        .handle
                        .snapshot()
                        .plan
                        .map(|p| p.sub_questions.len())
                        .unwrap_or(0);
                    for g in gaps.iter().take(request.breadth.max(1) as usize) {
                        idx_offset += 1;
                        let sq = atomr_agents_deep_research_core::SubQuestion {
                            id: format!("sq-{}", idx_offset),
                            text: format!("Resolve gap: {g}"),
                            rationale: None,
                            section: Some("Findings".into()),
                            status: atomr_agents_deep_research_core::SubQuestionStatus::Pending,
                        };
                        ctx.handle.append_sub_question(sq);
                    }
                    Ok(DeepResearchStepOutcome::Continue {
                        label: "refining".into(),
                    })
                }
            }
            Stage::Verify => {
                ctx.verifier.verify(ctx.handle).await?;
                self.state.lock().stage = Stage::Done;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "verified".into(),
                })
            }
            Stage::Done => Ok(DeepResearchStepOutcome::Done { label: "done".into() }),
        }
    }
}
