//! Loop-strategy implementations (one per topology).

mod clarify_plan_search_verify;
mod iterative_deepening;
mod linear_write_critique;
mod multi_agent_parallel;
mod outline_first_section_fanout;
mod plan_and_execute;

pub use clarify_plan_search_verify::ClarifyPlanSearchVerifyLoop;
pub use iterative_deepening::IterativeDeepeningLoop;
pub use linear_write_critique::LinearWriteCritiqueLoop;
pub use multi_agent_parallel::MultiAgentParallelLoop;
pub use outline_first_section_fanout::OutlineFirstSectionFanoutLoop;
pub use plan_and_execute::PlanAndExecuteLoop;
