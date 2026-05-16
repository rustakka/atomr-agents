//! Linear draft-then-refine strategy: research once, then loop on
//! write-and-critique until the critic is satisfied or depth exhausted.

use async_trait::async_trait;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::{ClarifyOutcome, CritiqueOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    ResearchAll,
    Write,
    Critique,
    Verify,
    Done,
}

/// Loop strategy: clarify → plan → sequentially research every
/// sub-question → write → critique → (refine via write again until
/// critic done || rounds >= depth) → verify.
///
/// Never loops back to research — refinement is purely a writer
/// concern. The `Writer` trait already receives `&ResearchHandle`, so
/// a refining writer can read `handle.snapshot()` to see prior drafts
/// and the latest critique transcript entry.
pub struct LinearWriteCritiqueLoop {
    state: parking_lot::Mutex<LoopState>,
}

struct LoopState {
    stage: Stage,
    next_idx: usize,
    rounds: u32,
}

impl Default for LinearWriteCritiqueLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl LinearWriteCritiqueLoop {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(LoopState {
                stage: Stage::Clarify,
                next_idx: 0,
                rounds: 0,
            }),
        }
    }
}

#[async_trait]
impl DeepResearchLoopStrategy for LinearWriteCritiqueLoop {
    fn name(&self) -> &str {
        "linear-write-critique"
    }

    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
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
                self.state.lock().stage = Stage::ResearchAll;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::ResearchAll => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                let idx = self.state.lock().next_idx;
                if idx >= plan.sub_questions.len() {
                    self.state.lock().stage = Stage::Write;
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
                self.state.lock().stage = Stage::Critique;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "wrote".into(),
                })
            }
            Stage::Critique => {
                let request = ctx.handle.request();
                let CritiqueOutcome { done, .. } = ctx.critic.critique(ctx.handle).await?;
                let max_depth = request.depth;
                let mut s = self.state.lock();
                if done || s.rounds >= max_depth {
                    s.stage = Stage::Verify;
                    Ok(DeepResearchStepOutcome::Continue {
                        label: "critique_done".into(),
                    })
                } else {
                    s.rounds += 1;
                    s.stage = Stage::Write;
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
