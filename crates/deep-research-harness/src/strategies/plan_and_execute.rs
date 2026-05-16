//! Plan-and-execute strategy: planner emits ordered sub-questions
//! (steps), researcher runs each, critic runs after every step, and a
//! lingering gap re-invokes the planner mid-flow.

use async_trait::async_trait;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::{ClarifyOutcome, CritiqueOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    Execute,
    StepCritique,
    Write,
    Verify,
    Done,
}

/// Loop strategy: clarify → plan → for each step (execute → critique →
/// maybe re-plan) → write → verify.
///
/// Differs from [`IterativeDeepeningLoop`] (which critiques only at
/// round boundaries before a batch of research): critique here fires
/// **after every single step**, and a re-plan *replaces* remaining
/// sub-questions instead of appending to them.
///
/// [`IterativeDeepeningLoop`]: crate::strategies::IterativeDeepeningLoop
pub struct PlanAndExecuteLoop {
    state: parking_lot::Mutex<LoopState>,
}

struct LoopState {
    stage: Stage,
    next_idx: usize,
    rounds: u32,
}

impl Default for PlanAndExecuteLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanAndExecuteLoop {
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
impl DeepResearchLoopStrategy for PlanAndExecuteLoop {
    fn name(&self) -> &str {
        "plan-and-execute"
    }

    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
        // Snapshot state under the lock, drop guard before any await.
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
                {
                    let mut s = self.state.lock();
                    s.next_idx = 0;
                    s.stage = Stage::Execute;
                }
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::Execute => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                let idx = self.state.lock().next_idx;
                if idx >= plan.sub_questions.len() {
                    self.state.lock().stage = Stage::Write;
                    return Ok(DeepResearchStepOutcome::Continue {
                        label: "execute_done".into(),
                    });
                }
                let sub = plan.sub_questions[idx].clone();
                ctx.researcher.research(&sub, ctx.handle).await?;
                self.state.lock().stage = Stage::StepCritique;
                Ok(DeepResearchStepOutcome::Continue {
                    label: format!("researched:{}", sub.id),
                })
            }
            Stage::StepCritique => {
                let request = ctx.handle.request();
                let CritiqueOutcome { done, gaps, .. } = ctx.critic.critique(ctx.handle).await?;
                let max_depth = request.depth;

                let (cur_idx, cur_rounds) = {
                    let s = self.state.lock();
                    (s.next_idx, s.rounds)
                };

                // Re-plan condition: gaps present, critic not done, depth not
                // exhausted. Re-plan replaces remaining sub-questions: we
                // re-invoke the planner, which overwrites the plan whole via
                // `set_plan`; execution then resumes from index 0 of the new
                // plan. Otherwise advance to the next step.
                if !done && !gaps.is_empty() && cur_rounds < max_depth {
                    let mut s = self.state.lock();
                    s.rounds += 1;
                    s.next_idx = 0;
                    s.stage = Stage::Plan;
                    drop(s);
                    Ok(DeepResearchStepOutcome::Continue {
                        label: "replanning".into(),
                    })
                } else {
                    // Move on to next step.
                    let mut s = self.state.lock();
                    s.next_idx = cur_idx + 1;
                    s.stage = Stage::Execute;
                    drop(s);
                    Ok(DeepResearchStepOutcome::Continue {
                        label: "step_critiqued".into(),
                    })
                }
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
