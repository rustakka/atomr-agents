//! Test-only support shared across every tool's `#[cfg(test)]` block.

use std::sync::Arc;
use std::time::Duration;

use atomr_agents_core::{CallCtx, InvokeCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, Value};
use atomr_agents_deep_research_core::{ResearchRequest, ResearchResult};
use atomr_agents_web_search_core::MockWebSearch;
use parking_lot::Mutex;

use crate::handle::ResearchHandle;

pub fn ctx() -> InvokeCtx {
    InvokeCtx {
        call: CallCtx {
            agent_id: None,
            tokens: TokenBudget::new(1000),
            time: TimeBudget::new(Duration::from_secs(5)),
            money: MoneyBudget::from_usd(1.0),
            iterations: IterationBudget::new(5),
            trace: vec![],
        },
        tool_call_id: "test-1".into(),
        raw_args: Value::Null,
    }
}

pub fn handle_for(query: &str) -> ResearchHandle {
    let req = ResearchRequest::new(query);
    let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
    ResearchHandle::new(result, Arc::new(req), Arc::new(MockWebSearch::new()))
}
