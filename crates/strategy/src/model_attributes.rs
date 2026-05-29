//! Machine-readable provider/model compliance attestation (FR-20).
//!
//! Gating which LLM an MNPI compartment may use must be *derived* from
//! per-model data-residency / contractual (BAA) / approval attributes —
//! not maintained as a brittle hardcoded string allowlist. A
//! [`ModelRegistry`] holds [`ModelAttributes`]; a declarative
//! [`ModelPredicate`] expresses a compartment→model rule (e.g. "region ==
//! EU AND tags ⊇ {mnpi-approved}"); [`ModelRegistry::allowlist`] derives
//! the concrete `allowed_models` list, and the attribute set backing a
//! decision is auditable.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

/// Data-residency / deployment region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Eu,
    Us,
    Uk,
    ApacJp,
    Other(String),
}

/// Contractual coverage attested for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// Business Associate Agreement.
    Baa,
    /// Data Processing Agreement.
    Dpa,
    Other(String),
}

/// Machine-readable attributes for one model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAttributes {
    pub provider: String,
    pub model_id: String,
    pub region: Region,
    /// Where data is processed/stored (often == region).
    pub residency: Region,
    pub contractual: Vec<Coverage>,
    /// Free-form tags, e.g. `"mnpi-approved"`.
    pub tags: BTreeSet<String>,
    /// Monotonic version of this attribute record (auditable).
    #[serde(default)]
    pub version: u32,
}

impl ModelAttributes {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>, region: Region) -> Self {
        let region2 = region.clone();
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
            residency: region2,
            region,
            contractual: Vec::new(),
            tags: BTreeSet::new(),
            version: 1,
        }
    }
    pub fn with_coverage(mut self, c: Coverage) -> Self {
        self.contractual.push(c);
        self
    }
    pub fn with_tag(mut self, t: impl Into<String>) -> Self {
        self.tags.insert(t.into());
        self
    }
    pub fn has_tag(&self, t: &str) -> bool {
        self.tags.contains(t)
    }
    pub fn has_coverage(&self, c: &Coverage) -> bool {
        self.contractual.contains(c)
    }
}

/// Declarative predicate over [`ModelAttributes`]. Serializable so a
/// compartment→model policy is itself an auditable artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPredicate {
    RegionIs(Region),
    ResidencyIs(Region),
    HasTag(String),
    HasCoverage(Coverage),
    ProviderIs(String),
    All(Vec<ModelPredicate>),
    Any(Vec<ModelPredicate>),
    Not(Box<ModelPredicate>),
}

impl ModelPredicate {
    pub fn matches(&self, m: &ModelAttributes) -> bool {
        match self {
            ModelPredicate::RegionIs(r) => &m.region == r,
            ModelPredicate::ResidencyIs(r) => &m.residency == r,
            ModelPredicate::HasTag(t) => m.has_tag(t),
            ModelPredicate::HasCoverage(c) => m.has_coverage(c),
            ModelPredicate::ProviderIs(p) => &m.provider == p,
            ModelPredicate::All(ps) => ps.iter().all(|p| p.matches(m)),
            ModelPredicate::Any(ps) => ps.iter().any(|p| p.matches(m)),
            ModelPredicate::Not(p) => !p.matches(m),
        }
    }
}

/// Registry of model attributes, queryable and versioned.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    by_id: HashMap<String, ModelAttributes>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, attrs: ModelAttributes) {
        self.by_id.insert(attrs.model_id.clone(), attrs);
    }

    pub fn with(mut self, attrs: ModelAttributes) -> Self {
        self.register(attrs);
        self
    }

    pub fn attributes(&self, model_id: &str) -> Option<&ModelAttributes> {
        self.by_id.get(model_id)
    }

    /// Derive the allowed-models list for a predicate. A model with no
    /// registered attributes is excluded (fail-closed). Result is sorted
    /// for determinism.
    pub fn allowlist(&self, predicate: &ModelPredicate) -> Vec<String> {
        let mut v: Vec<String> = self
            .by_id
            .values()
            .filter(|m| predicate.matches(m))
            .map(|m| m.model_id.clone())
            .collect();
        v.sort();
        v
    }

    /// The attribute records that satisfied a predicate — the audit
    /// evidence for a policy decision.
    pub fn evidence(&self, predicate: &ModelPredicate) -> Vec<ModelAttributes> {
        let mut v: Vec<ModelAttributes> = self
            .by_id
            .values()
            .filter(|m| predicate.matches(m))
            .cloned()
            .collect();
        v.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ModelRegistry {
        ModelRegistry::new()
            .with(
                ModelAttributes::new("anthropic", "claude-eu", Region::Eu)
                    .with_coverage(Coverage::Baa)
                    .with_tag("mnpi-approved"),
            )
            .with(ModelAttributes::new("openai", "gpt-us", Region::Us).with_tag("mnpi-approved"))
            .with(ModelAttributes::new(
                "anthropic",
                "claude-eu-unapproved",
                Region::Eu,
            ))
    }

    #[test]
    fn mnpi_eu_predicate_derives_allowlist() {
        let reg = registry();
        let pred = ModelPredicate::All(vec![
            ModelPredicate::RegionIs(Region::Eu),
            ModelPredicate::HasTag("mnpi-approved".into()),
        ]);
        assert_eq!(reg.allowlist(&pred), vec!["claude-eu".to_string()]);
        // gpt-us excluded (wrong region); claude-eu-unapproved excluded (no tag)
        assert_eq!(reg.evidence(&pred).len(), 1);
    }

    #[test]
    fn unregistered_model_is_excluded() {
        let reg = registry();
        let pred = ModelPredicate::HasCoverage(Coverage::Baa);
        let allow = reg.allowlist(&pred);
        assert_eq!(allow, vec!["claude-eu".to_string()]);
        assert!(reg.attributes("nonexistent").is_none());
    }

    #[test]
    fn predicate_serializes() {
        let pred = ModelPredicate::Not(Box::new(ModelPredicate::ProviderIs("openai".into())));
        let json = serde_json::to_string(&pred).unwrap();
        let back: ModelPredicate = serde_json::from_str(&json).unwrap();
        assert_eq!(pred, back);
    }
}
