//! Context assembler.
//!
//! Strategies produce `ContextFragment`s with priorities and token
//! estimates. The assembler orders by priority and packs greedily
//! into the remaining `TokenBudget` — high-priority fragments
//! survive; low-priority ones are dropped first under pressure.

use atomr_agents_core::{Result, TokenBudget};

/// One fragment contributed by a strategy.
#[derive(Debug, Clone)]
pub struct ContextFragment {
    pub source: &'static str,
    pub priority: u8,
    pub estimated_tokens: u32,
    pub text: String,
}

/// Output of `ContextAssembler::assemble`.
#[derive(Debug, Clone, Default)]
pub struct RenderedContext {
    pub fragments: Vec<ContextFragment>,
    pub total_tokens: u32,
}

impl RenderedContext {
    pub fn join(&self, sep: &str) -> String {
        self.fragments
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>()
            .join(sep)
    }
}

pub struct ContextAssembler;

impl ContextAssembler {
    /// Pack the fragments into the remaining budget, highest priority
    /// first. Returns the fragments that fit, in the order they were
    /// originally given (priority is used for *eviction*, not
    /// reordering).
    pub fn assemble(
        mut fragments: Vec<ContextFragment>,
        budget: &mut TokenBudget,
    ) -> Result<RenderedContext> {
        // Stable indexed sort: keep original positions for tie-break,
        // but evict lowest priority first when over budget.
        let mut indexed: Vec<(usize, ContextFragment)> = fragments.drain(..).enumerate().collect();

        let total: u64 = indexed.iter().map(|(_, f)| f.estimated_tokens as u64).sum();
        if total <= budget.remaining as u64 {
            // Everything fits; restore original order.
            let mut out: Vec<ContextFragment> = indexed.into_iter().map(|(_, f)| f).collect();
            let total_tokens = out.iter().map(|f| f.estimated_tokens).sum();
            budget.consume(total_tokens)?;
            return Ok(RenderedContext {
                fragments: std::mem::take(&mut out),
                total_tokens,
            });
        }

        // Otherwise, evict lowest-priority fragments until we fit.
        // Priority: higher number = more important.
        indexed.sort_by(|a, b| b.1.priority.cmp(&a.1.priority).then_with(|| a.0.cmp(&b.0)));
        let mut kept: Vec<(usize, ContextFragment)> = Vec::new();
        let mut acc: u64 = 0;
        for entry in indexed {
            let cost = entry.1.estimated_tokens as u64;
            if acc + cost <= budget.remaining as u64 {
                acc += cost;
                kept.push(entry);
            }
        }
        kept.sort_by_key(|(i, _)| *i);
        let out: Vec<ContextFragment> = kept.into_iter().map(|(_, f)| f).collect();
        let total_tokens = out.iter().map(|f| f.estimated_tokens).sum();
        budget.consume(total_tokens)?;
        Ok(RenderedContext {
            fragments: out,
            total_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(source: &'static str, prio: u8, tokens: u32) -> ContextFragment {
        ContextFragment {
            source,
            priority: prio,
            estimated_tokens: tokens,
            text: source.to_string(),
        }
    }

    #[test]
    fn assemble_fits_under_budget() {
        let mut b = TokenBudget::new(100);
        let r =
            ContextAssembler::assemble(vec![frag("a", 5, 30), frag("b", 5, 30), frag("c", 5, 30)], &mut b)
                .unwrap();
        assert_eq!(r.fragments.len(), 3);
        assert_eq!(r.total_tokens, 90);
        assert_eq!(b.remaining, 10);
    }

    #[test]
    fn assemble_evicts_lowest_priority_first() {
        let mut b = TokenBudget::new(60);
        let r = ContextAssembler::assemble(
            vec![frag("low", 1, 30), frag("hi", 9, 30), frag("med", 5, 30)],
            &mut b,
        )
        .unwrap();
        let kept: Vec<&str> = r.fragments.iter().map(|f| f.source).collect();
        assert_eq!(kept, vec!["hi", "med"]);
    }
}
