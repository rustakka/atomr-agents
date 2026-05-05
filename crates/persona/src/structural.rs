use async_trait::async_trait;
use atomr_agents_core::{AgentContext, Result, TokenBudget};
use std::collections::HashMap;

use crate::r#trait::{
    Persona, PersonaMetadata, PersonaStrategy, RenderedPersona, StyleSpec, TraitFragment,
};

/// Naive token estimator: ~4 chars per token.
fn est_tokens(s: &str) -> u32 {
    ((s.chars().count() + 3) / 4) as u32
}

// ---------- Static ----------

pub struct StaticPersonaStrategy {
    template: String,
    variables: HashMap<String, String>,
    metadata: PersonaMetadata,
}

impl StaticPersonaStrategy {
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            variables: HashMap::new(),
            metadata: PersonaMetadata { framework: Some("static".into()) },
        }
    }

    pub fn var(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.variables.insert(key.into(), val.into());
        self
    }

    fn rendered_identity(&self) -> String {
        let mut out = self.template.clone();
        for (k, v) in &self.variables {
            out = out.replace(&format!("{{{}}}", k), v);
        }
        out
    }
}

#[async_trait]
impl PersonaStrategy for StaticPersonaStrategy {
    async fn resolve(
        &self,
        _ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let identity = self.rendered_identity();
        let tokens = est_tokens(&identity);
        budget.consume(tokens.min(budget.remaining))?;
        Ok(RenderedPersona {
            identity,
            salient_traits: vec![],
            style: StyleSpec::default(),
            metadata: self.metadata.clone(),
            estimated_tokens: tokens,
        })
    }
}

// ---------- Big Five ----------

pub trait TraitRenderer: Send + Sync + 'static {
    fn render(&self, scores: BigFiveScores) -> (String, Vec<TraitFragment>);
}

#[derive(Debug, Clone, Copy)]
pub struct BigFiveScores {
    pub openness: f32,
    pub conscientiousness: f32,
    pub extraversion: f32,
    pub agreeableness: f32,
    pub neuroticism: f32,
}

pub struct DefaultBigFiveRenderer;

impl TraitRenderer for DefaultBigFiveRenderer {
    fn render(&self, s: BigFiveScores) -> (String, Vec<TraitFragment>) {
        let mut traits = Vec::new();
        let pairs = [
            ("openness", s.openness, "open to new ideas"),
            ("conscientiousness", s.conscientiousness, "diligent and methodical"),
            ("extraversion", s.extraversion, "outgoing and energetic"),
            ("agreeableness", s.agreeableness, "warm and cooperative"),
            ("neuroticism", s.neuroticism, "emotionally reactive"),
        ];
        for (label, weight, desc) in pairs {
            traits.push(TraitFragment {
                label: label.into(),
                weight,
                description: desc.into(),
            });
        }
        let identity = format!(
            "OCEAN profile: O={:.2} C={:.2} E={:.2} A={:.2} N={:.2}",
            s.openness, s.conscientiousness, s.extraversion, s.agreeableness, s.neuroticism
        );
        (identity, traits)
    }
}

pub struct BigFivePersonaStrategy {
    pub scores: BigFiveScores,
    pub rendering: Box<dyn TraitRenderer>,
}

impl BigFivePersonaStrategy {
    pub fn new(scores: BigFiveScores) -> Self {
        Self { scores, rendering: Box::new(DefaultBigFiveRenderer) }
    }
}

#[async_trait]
impl PersonaStrategy for BigFivePersonaStrategy {
    async fn resolve(
        &self,
        _ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let (identity, traits) = self.rendering.render(self.scores);
        let tokens = est_tokens(&identity)
            + traits.iter().map(|t| est_tokens(&t.description)).sum::<u32>();
        budget.consume(tokens.min(budget.remaining))?;
        Ok(RenderedPersona {
            identity,
            salient_traits: traits,
            style: StyleSpec::default(),
            metadata: PersonaMetadata { framework: Some("big-five".into()) },
            estimated_tokens: tokens,
        })
    }
}

// ---------- MBTI ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum MbtiType {
    INTJ, INTP, ENTJ, ENTP, INFJ, INFP, ENFJ, ENFP,
    ISTJ, ISFJ, ESTJ, ESFJ, ISTP, ISFP, ESTP, ESFP,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum CognitiveFunction {
    Ni, Ne, Si, Se, Ti, Te, Fi, Fe,
}

#[derive(Debug, Clone, Copy)]
pub struct CognitiveStack {
    pub dominant: CognitiveFunction,
    pub auxiliary: CognitiveFunction,
    pub tertiary: CognitiveFunction,
    pub inferior: CognitiveFunction,
}

#[derive(Debug, Clone, Copy)]
pub enum ExpressionLevel {
    Subtle,
    Pronounced,
}

pub struct MbtiPersonaStrategy {
    pub mbti_type: MbtiType,
    pub cognitive_stack: CognitiveStack,
    pub expression: ExpressionLevel,
}

impl MbtiPersonaStrategy {
    pub fn new(t: MbtiType, stack: CognitiveStack, expression: ExpressionLevel) -> Self {
        Self { mbti_type: t, cognitive_stack: stack, expression }
    }
}

#[async_trait]
impl PersonaStrategy for MbtiPersonaStrategy {
    async fn resolve(
        &self,
        _ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let identity = format!(
            "MBTI {:?} (dom={:?}, aux={:?}, tert={:?}, inf={:?}, expr={:?})",
            self.mbti_type,
            self.cognitive_stack.dominant,
            self.cognitive_stack.auxiliary,
            self.cognitive_stack.tertiary,
            self.cognitive_stack.inferior,
            self.expression
        );
        let tokens = est_tokens(&identity);
        budget.consume(tokens.min(budget.remaining))?;
        Ok(RenderedPersona {
            identity,
            salient_traits: vec![],
            style: StyleSpec::default(),
            metadata: PersonaMetadata { framework: Some("mbti".into()) },
            estimated_tokens: tokens,
        })
    }
}

// ---------- Jungian Archetype ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    Sage, Caregiver, Explorer, Hero, Magician, Outlaw,
    Lover, Jester, Everyman, Innocent, Ruler, Creator,
}

pub enum ArchetypeExpression {
    Pure,
    Modern,
}

pub struct JungianArchetypeStrategy {
    pub primary: Archetype,
    pub shadow: Option<Archetype>,
    pub individuation: f32,
    pub expression: ArchetypeExpression,
}

impl JungianArchetypeStrategy {
    pub fn new(primary: Archetype) -> Self {
        Self {
            primary,
            shadow: None,
            individuation: 0.5,
            expression: ArchetypeExpression::Modern,
        }
    }

    pub fn with_shadow(mut self, shadow: Archetype) -> Self {
        self.shadow = Some(shadow);
        self
    }
}

#[async_trait]
impl PersonaStrategy for JungianArchetypeStrategy {
    async fn resolve(
        &self,
        _ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        let shadow_str =
            self.shadow.map(|s| format!(" with {s:?} shadow")).unwrap_or_default();
        let identity = format!(
            "Archetype: {:?}{} (individuation={:.2})",
            self.primary, shadow_str, self.individuation
        );
        let tokens = est_tokens(&identity);
        budget.consume(tokens.min(budget.remaining))?;
        Ok(RenderedPersona {
            identity,
            salient_traits: vec![],
            style: StyleSpec::default(),
            metadata: PersonaMetadata { framework: Some("jungian".into()) },
            estimated_tokens: tokens,
        })
    }
}

// ---------- Composite ----------

pub trait PersonaReconciler: Send + Sync + 'static {
    fn reconcile(&self, layers: Vec<(RenderedPersona, f32)>) -> Persona;
}

pub struct WeightedAverageReconciler;

impl PersonaReconciler for WeightedAverageReconciler {
    fn reconcile(&self, layers: Vec<(RenderedPersona, f32)>) -> Persona {
        if layers.is_empty() {
            return Persona::default();
        }
        let total: f32 = layers.iter().map(|(_, w)| *w).sum();
        // Concatenate identities weighted, keep all salient traits.
        let mut identity_parts = Vec::new();
        let mut traits = Vec::new();
        let mut metadata = PersonaMetadata { framework: Some("composite".into()) };
        for (p, w) in &layers {
            identity_parts.push(format!("[{:.0}%] {}", (w / total) * 100.0, p.identity));
            for t in &p.salient_traits {
                traits.push(TraitFragment {
                    label: t.label.clone(),
                    weight: t.weight * (w / total),
                    description: t.description.clone(),
                });
            }
            if metadata.framework.as_deref() == Some("composite") {
                if let Some(f) = &p.metadata.framework {
                    metadata.framework = Some(format!("composite({f})"));
                }
            }
        }
        Persona {
            identity: identity_parts.join(" + "),
            salient_traits: traits,
            style: StyleSpec::default(),
            metadata,
        }
    }
}

pub struct CompositePersonaStrategy {
    layers: Vec<(Box<dyn PersonaStrategy>, f32)>,
    reconciler: Box<dyn PersonaReconciler>,
}

impl CompositePersonaStrategy {
    pub fn new(
        layers: Vec<(Box<dyn PersonaStrategy>, f32)>,
        reconciler: Box<dyn PersonaReconciler>,
    ) -> Self {
        Self { layers, reconciler }
    }

    pub fn weighted_average(layers: Vec<(Box<dyn PersonaStrategy>, f32)>) -> Self {
        Self { layers, reconciler: Box::new(WeightedAverageReconciler) }
    }
}

#[async_trait]
impl PersonaStrategy for CompositePersonaStrategy {
    async fn resolve(
        &self,
        ctx: &AgentContext,
        budget: &mut TokenBudget,
    ) -> Result<RenderedPersona> {
        // Each layer gets a slice of the budget proportional to its
        // weight. (Simplification: we equal-split for now.)
        let n = self.layers.len() as u32;
        if n == 0 {
            return Ok(RenderedPersona::default());
        }
        let mut subs = budget.split(n);
        let mut rendered: Vec<(RenderedPersona, f32)> = Vec::with_capacity(self.layers.len());
        let mut consumed_total = 0u32;
        for ((strat, weight), sub) in self.layers.iter().zip(subs.iter_mut()) {
            let initial = sub.remaining;
            let r = strat.resolve(ctx, sub).await?;
            consumed_total += initial.saturating_sub(sub.remaining);
            rendered.push((r, *weight));
        }
        // Sync the parent budget with what was actually used.
        budget.consume(consumed_total.min(budget.remaining)).ok();
        let merged = self.reconciler.reconcile(rendered);
        let tokens = est_tokens(&merged.identity)
            + merged.salient_traits.iter().map(|t| est_tokens(&t.description)).sum::<u32>();
        Ok(RenderedPersona {
            identity: merged.identity,
            salient_traits: merged.salient_traits,
            style: merged.style,
            metadata: merged.metadata,
            estimated_tokens: tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{AgentId, TurnInput};

    fn ctx() -> AgentContext {
        AgentContext::for_agent(
            AgentId::from("a-1"),
            TurnInput { user: "hi".into(), history: vec![] },
        )
    }

    #[tokio::test]
    async fn static_strategy_substitutes_variables() {
        let s = StaticPersonaStrategy::new("You are a {role}.").var("role", "researcher");
        let mut b = TokenBudget::new(1000);
        let r = s.resolve(&ctx(), &mut b).await.unwrap();
        assert_eq!(r.identity, "You are a researcher.");
        assert!(r.estimated_tokens > 0);
    }

    #[tokio::test]
    async fn big_five_renders_all_traits() {
        let s = BigFivePersonaStrategy::new(BigFiveScores {
            openness: 0.9,
            conscientiousness: 0.8,
            extraversion: 0.2,
            agreeableness: 0.7,
            neuroticism: 0.1,
        });
        let mut b = TokenBudget::new(2000);
        let r = s.resolve(&ctx(), &mut b).await.unwrap();
        assert_eq!(r.salient_traits.len(), 5);
    }

    #[tokio::test]
    async fn composite_merges_layers() {
        let big = Box::new(BigFivePersonaStrategy::new(BigFiveScores {
            openness: 0.8,
            conscientiousness: 0.7,
            extraversion: 0.4,
            agreeableness: 0.5,
            neuroticism: 0.3,
        }));
        let arch = Box::new(JungianArchetypeStrategy::new(Archetype::Sage));
        let composite =
            CompositePersonaStrategy::weighted_average(vec![(big, 0.6), (arch, 0.4)]);
        let mut b = TokenBudget::new(2000);
        let r = composite.resolve(&ctx(), &mut b).await.unwrap();
        assert!(r.identity.contains("Archetype"));
        assert!(r.identity.contains("OCEAN"));
    }
}
