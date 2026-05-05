use async_trait::async_trait;
use atomr_agents_core::{AgentContext, Result, TokenBudget};

use crate::r#trait::{Persona, RenderedPersona};

/// A static persona is the same every turn; an *emphasis* strategy
/// projects different facets based on what's happening.
#[async_trait]
pub trait PersonaEmphasisStrategy: Send + Sync + 'static {
    async fn emphasize(
        &self,
        full: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona>;
}

fn estimated_tokens(p: &Persona) -> u32 {
    let chars = p.identity.chars().count()
        + p.salient_traits
            .iter()
            .map(|t| t.description.chars().count())
            .sum::<usize>();
    ((chars + 3) / 4) as u32
}

// ---------- StaticEmphasis ----------

pub struct StaticEmphasis;

#[async_trait]
impl PersonaEmphasisStrategy for StaticEmphasis {
    async fn emphasize(
        &self,
        full: &Persona,
        _ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let tokens = estimated_tokens(full);
        budget.consume(tokens.min(budget.remaining))?;
        Ok(RenderedPersona {
            identity: full.identity.clone(),
            salient_traits: full.salient_traits.clone(),
            style: full.style.clone(),
            metadata: full.metadata.clone(),
            estimated_tokens: tokens,
        })
    }
}

// ---------- AudienceAdaptive ----------

pub struct AudienceAdaptive;

#[async_trait]
impl PersonaEmphasisStrategy for AudienceAdaptive {
    async fn emphasize(
        &self,
        full: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        // Heuristic: long history → "expert audience" emphasis;
        // short or empty → "newcomer" emphasis.
        let expert = ctx.turn.history.len() >= 4;
        let mut p = full.clone();
        if expert {
            p.salient_traits.retain(|t| t.weight >= 0.5);
        } else {
            // Newcomers hear the warmest traits foregrounded.
            p.salient_traits.sort_by(|a, b| {
                b.weight
                    .partial_cmp(&a.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            p.salient_traits.truncate(3);
        }
        StaticEmphasis.emphasize(&p, ctx, budget).await
    }
}

// ---------- TaskAdaptive ----------

pub struct TaskAdaptive;

#[async_trait]
impl PersonaEmphasisStrategy for TaskAdaptive {
    async fn emphasize(
        &self,
        full: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let user_lower = ctx.turn.user.to_lowercase();
        let label = if user_lower.contains("debug") || user_lower.contains("error") {
            "analytical"
        } else if user_lower.contains("brainstorm") || user_lower.contains("idea") {
            "generative"
        } else {
            "balanced"
        };
        let mut p = full.clone();
        p.identity = format!("{} (mode: {})", p.identity, label);
        StaticEmphasis.emphasize(&p, ctx, budget).await
    }
}

// ---------- MoodState ----------

pub struct MoodState {
    /// 0.0 = relaxed; 1.0 = pressured. Mutated by the agent loop via
    /// `update_mood` (for now exposed as `pub`).
    pub pressure: f32,
}

impl MoodState {
    pub fn new() -> Self {
        Self { pressure: 0.0 }
    }
}

impl Default for MoodState {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersonaEmphasisStrategy for MoodState {
    async fn emphasize(
        &self,
        full: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let mood_word = if self.pressure > 0.7 {
            "focused"
        } else if self.pressure > 0.3 {
            "alert"
        } else {
            "playful"
        };
        let mut p = full.clone();
        p.identity = format!("{} (mood: {})", p.identity, mood_word);
        StaticEmphasis.emphasize(&p, ctx, budget).await
    }
}

// ---------- GoalConditioned ----------

pub struct GoalConditioned {
    /// Free-form sub-goal label set externally by the harness.
    pub current_goal: String,
}

#[async_trait]
impl PersonaEmphasisStrategy for GoalConditioned {
    async fn emphasize(
        &self,
        full: &Persona,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let mut p = full.clone();
        p.identity = format!("{} (goal: {})", p.identity, self.current_goal);
        StaticEmphasis.emphasize(&p, ctx, budget).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#trait::{Persona, StyleSpec, TraitFragment};
    use atomr_agents_core::{AgentId, Message, MessageRole, TurnInput};

    fn ctx_with_history(n: usize, q: &str) -> AgentContext {
        let history = (0..n)
            .map(|i| Message {
                role: MessageRole::User,
                content: format!("turn {i}"),
            })
            .collect();
        AgentContext::for_agent(
            AgentId::from("a"),
            TurnInput {
                user: q.into(),
                history,
            },
        )
    }

    fn persona() -> Persona {
        Persona {
            identity: "engineer".into(),
            salient_traits: vec![
                TraitFragment {
                    label: "warmth".into(),
                    weight: 0.8,
                    description: "warm".into(),
                },
                TraitFragment {
                    label: "rigor".into(),
                    weight: 0.6,
                    description: "rigorous".into(),
                },
                TraitFragment {
                    label: "playfulness".into(),
                    weight: 0.3,
                    description: "playful".into(),
                },
                TraitFragment {
                    label: "irrelevant".into(),
                    weight: 0.1,
                    description: "not now".into(),
                },
            ],
            style: StyleSpec::default(),
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn audience_adaptive_drops_low_weight_for_experts() {
        let p = persona();
        let mut b = TokenBudget::new(1000);
        let r = AudienceAdaptive
            .emphasize(&p, &ctx_with_history(5, "x"), &mut b)
            .await
            .unwrap();
        assert!(r.salient_traits.iter().all(|t| t.weight >= 0.5));
    }

    #[tokio::test]
    async fn task_adaptive_picks_mode_from_keywords() {
        let p = persona();
        let mut b = TokenBudget::new(1000);
        let r = TaskAdaptive
            .emphasize(&p, &ctx_with_history(0, "debug this please"), &mut b)
            .await
            .unwrap();
        assert!(r.identity.contains("analytical"));
    }
}
