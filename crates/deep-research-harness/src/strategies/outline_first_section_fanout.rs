//! Section-centric strategy: outline-first, then fan out one task per
//! outline section. Each task sequentially researches its section's
//! sub-questions; a final writer pass composes the report.

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures::future::join_all;

use crate::error::Result;
use crate::loop_strategy::{DeepResearchLoopStrategy, DeepResearchStepCtx, DeepResearchStepOutcome};
use crate::roles::ClarifyOutcome;

const UNCATEGORIZED: &str = "Uncategorized";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Clarify,
    Plan,
    FanOutBySection,
    Write,
    Verify,
    Done,
}

/// Loop strategy: clarify → plan → group sub-questions by section,
/// fan out one task per section (capped by `breadth`), each task
/// sequentially runs its section's sub-questions → single writer pass
/// → verify.
///
/// Sub-questions with `section: None` go into a synthetic
/// `"Uncategorized"` bucket.
pub struct OutlineFirstSectionFanoutLoop {
    state: parking_lot::Mutex<Stage>,
}

impl Default for OutlineFirstSectionFanoutLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlineFirstSectionFanoutLoop {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(Stage::Clarify),
        }
    }
}

#[async_trait]
impl DeepResearchLoopStrategy for OutlineFirstSectionFanoutLoop {
    fn name(&self) -> &str {
        "outline-first-section-fanout"
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
                *self.state.lock() = Stage::FanOutBySection;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "planned".into(),
                })
            }
            Stage::FanOutBySection => {
                let request = ctx.handle.request();
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();

                // Group sub-questions by section. Use a BTreeMap so the
                // ordering of outer tasks is deterministic regardless of
                // insertion order.
                let mut buckets: BTreeMap<String, Vec<atomr_agents_deep_research_core::SubQuestion>> =
                    BTreeMap::new();
                for sub in &plan.sub_questions {
                    let key = sub.section.clone().unwrap_or_else(|| UNCATEGORIZED.to_string());
                    buckets.entry(key).or_default().push(sub.clone());
                }

                let breadth = request.breadth.max(1) as usize;
                let mut sections: Vec<(String, Vec<atomr_agents_deep_research_core::SubQuestion>)> =
                    buckets.into_iter().collect();
                sections.truncate(breadth.min(sections.len().max(1)));

                // Spawn one task per (kept) section. Each task runs its
                // section's sub-questions sequentially.
                let mut tasks = Vec::new();
                for (_section_name, subs) in sections {
                    let handle = ctx.handle.clone();
                    let researcher = ctx.researcher;
                    tasks.push(async move {
                        for sub in subs {
                            researcher.research(&sub, &handle).await?;
                        }
                        Ok::<(), crate::error::DeepResearchError>(())
                    });
                }
                let results = join_all(tasks).await;
                for r in results {
                    r?;
                }
                *self.state.lock() = Stage::Write;
                Ok(DeepResearchStepOutcome::Continue {
                    label: "fan_out_by_section_done".into(),
                })
            }
            Stage::Write => {
                let plan = ctx.handle.snapshot().plan.unwrap_or_default();
                ctx.writer.write(&plan, ctx.handle).await?;
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
