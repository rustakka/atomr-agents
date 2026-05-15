//! LangGraph open_deep_research-style strategy: supervisor critic +
//! `think_tool` style decisions, iterative deepening, researcher
//! emits compressed findings.

use async_trait::async_trait;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::ClarifyOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    Supervisor,
    Research,
    Write,
    Verify,
    Done,
}

/// Loop strategy: clarify → plan → repeated (supervisor → research) →
/// write → verify. The supervisor doubles as the critic; sub-questions
/// can be appended dynamically when the supervisor identifies new gaps.
pub struct IterativeDeepeningLoop {
    state: parking_lot::Mutex<LoopState>,
}

struct LoopState {
    stage: Stage,
    rounds: u32,
    next_idx: usize,
}

impl Default for IterativeDeepeningLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl IterativeDeepeningLoop {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(LoopState {
                stage: Stage::Clarify,
                rounds: 0,
                next_idx: 0,
            }),
        }
    }
}

#[async_trait]
impl DeepResearchLoopStrategy for IterativeDeepeningLoop {
    fn name(&self) -> &str {
        "iterative-deepening"
    }

    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
        let stage = self.state.lock().stage;
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
                self.state.lock().stage = Stage::Supervisor;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::Supervisor => {
                // The supervisor IS the critic — and the
                // `next_idx` cursor is the "think_tool" output.
                let request = ctx.handle.request();
                let outcome = ctx.critic.critique(ctx.handle).await?;
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                let cur_idx = self.state.lock().next_idx;
                if outcome.done && cur_idx >= plan.sub_questions.len() {
                    self.state.lock().stage = Stage::Write;
                    return Ok(DeepResearchStepOutcome::Continue {
                        label: "supervisor_done".into(),
                    });
                }
                // If we hit the depth cap, finish anyway.
                let rounds = self.state.lock().rounds;
                if rounds >= request.depth {
                    self.state.lock().stage = Stage::Write;
                    return Ok(DeepResearchStepOutcome::Continue {
                        label: "depth_cap".into(),
                    });
                }
                // Append new sub-questions for any gaps before researching.
                let mut s = self.state.lock();
                s.rounds += 1;
                drop(s);
                let mut idx_offset = ctx
                    .handle
                    .snapshot()
                    .plan
                    .map(|p| p.sub_questions.len())
                    .unwrap_or(0);
                for g in outcome.gaps.iter().take(request.breadth.max(1) as usize) {
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
                self.state.lock().stage = Stage::Research;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "supervisor".into(),
                })
            }
            Stage::Research => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                let idx = self.state.lock().next_idx;
                if idx >= plan.sub_questions.len() {
                    self.state.lock().stage = Stage::Supervisor;
                    return Ok(DeepResearchStepOutcome::Continue {
                        label: "research_done".into(),
                    });
                }
                let sub = plan.sub_questions[idx].clone();
                ctx.researcher.research(&sub, ctx.handle).await?;
                self.state.lock().next_idx = idx + 1;
                Ok(DeepResearchStepOutcome::Continue {
                    label: format!("researched:{}", sub.id),
                })
            }
            Stage::Write => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                ctx.writer.write(&plan, ctx.handle).await?;
                self.state.lock().stage = Stage::Verify;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "wrote".into(),
                })
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
