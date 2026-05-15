//! Anthropic-style strategy: lead orchestrator + parallel sub-agents.

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::ClarifyOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    FanOut,
    Write,
    Verify,
    Done,
}

/// Loop strategy: clarify → plan → fan out researcher per sub-question
/// (capped by `request.breadth`) → write → verify.
pub struct MultiAgentParallelLoop {
    state: parking_lot::Mutex<Stage>,
}

impl Default for MultiAgentParallelLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiAgentParallelLoop {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(Stage::Clarify),
        }
    }
}

#[async_trait]
impl DeepResearchLoopStrategy for MultiAgentParallelLoop {
    fn name(&self) -> &str {
        "multi-agent-parallel"
    }

    async fn step(&self, ctx: &mut DeepResearchStepCtx<'_>) -> Result<DeepResearchStepOutcome> {
        let stage = *self.state.lock();
        match stage {
            Stage::Clarify => {
                let request = ctx.handle.request();
                let outcome = ctx.clarifier.clarify(&request, ctx.handle).await?;
                match outcome {
                    ClarifyOutcome::Ready => {
                        *self.state.lock() = Stage::Plan;
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
                *self.state.lock() = Stage::FanOut;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::FanOut => {
                let request = ctx.handle.request();
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                let breadth = request.breadth.max(1) as usize;
                // Fan out — futures share the same handle and researcher.
                let mut futures_vec = Vec::new();
                for sub in plan.sub_questions.iter().take(breadth) {
                    let sub = sub.clone();
                    let handle = ctx.handle.clone();
                    let researcher = ctx.researcher;
                    futures_vec.push(async move { researcher.research(&sub, &handle).await });
                }
                let results = join_all(futures_vec).await;
                for r in results {
                    r?;
                }
                *self.state.lock() = Stage::Write;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "fan_out_done".into(),
                })
            }
            Stage::Write => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                ctx.writer.write(&plan, ctx.handle).await?;
                // One critic pass for telemetry / coverage hints.
                let _ = ctx.critic.critique(ctx.handle).await?;
                *self.state.lock() = Stage::Verify;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "wrote".into(),
                })
            }
            Stage::Verify => {
                ctx.verifier.verify(ctx.handle).await?;
                *self.state.lock() = Stage::Done;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "verified".into(),
                })
            }
            Stage::Done => Ok(DeepResearchStepOutcome::Done { label: "done".into() }),
        }
    }
}
