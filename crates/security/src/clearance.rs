use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Hierarchical sensitivity level. Ordered ascending: a subject cleared
/// at a higher level satisfies any lower-or-equal requirement.
///
/// `Mnpi` (material non-public information) is the top wall — the
/// hedge-fund use case that motivates this crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceLevel {
    #[default]
    Public,
    Internal,
    Confidential,
    Restricted,
    Mnpi,
}

/// A named information compartment (need-to-know wall), e.g.
/// `"deal:acme"`, `"desk:credit"`, `"issuer:XYZ"`. Membership is
/// all-or-nothing: a subject either holds the compartment or does not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Compartment(pub String);

impl Compartment {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Into<String>> From<S> for Compartment {
    fn from(s: S) -> Self {
        Compartment(s.into())
    }
}

/// What a guarded operation requires of a subject before it may run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedToKnow {
    /// Minimum clearance level (defaults to `Public`).
    #[serde(default)]
    pub min_level: ClearanceLevel,
    /// Compartments the subject must hold (ALL of them).
    #[serde(default)]
    pub compartments: BTreeSet<Compartment>,
}

impl NeedToKnow {
    /// Require a clearance level with no compartment restriction.
    pub fn level(min_level: ClearanceLevel) -> Self {
        Self {
            min_level,
            compartments: BTreeSet::new(),
        }
    }

    /// Add a required compartment.
    pub fn with_compartment(mut self, c: impl Into<Compartment>) -> Self {
        self.compartments.insert(c.into());
        self
    }
}

/// The caller's clearance, attached to [`atomr_agents_core::CallCtx`] as
/// a typed extension and read at the `Tool` seam by [`crate::WalledTool`]
/// and at query time by retriever entitlement filters.
///
/// This is a framework-local type; `atomr-orgs` projects its
/// system-of-record clearance onto it. It is never serialized into a
/// prompt or checkpoint (it travels only via the non-serialized
/// `CallCtx` extension map).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearanceContext {
    /// The subject (person/role) this clearance belongs to.
    pub subject: String,
    /// Granted clearance level.
    pub level: ClearanceLevel,
    /// Compartments the subject holds.
    pub compartments: BTreeSet<Compartment>,
}

impl ClearanceContext {
    pub fn new(subject: impl Into<String>, level: ClearanceLevel) -> Self {
        Self {
            subject: subject.into(),
            level,
            compartments: BTreeSet::new(),
        }
    }

    /// Grant a compartment, returning `self` for chaining.
    pub fn with_compartment(mut self, c: impl Into<Compartment>) -> Self {
        self.compartments.insert(c.into());
        self
    }

    /// Whether this clearance satisfies a `NeedToKnow` (level high
    /// enough AND every required compartment held).
    pub fn permits(&self, need: &NeedToKnow) -> bool {
        self.level >= need.min_level && need.compartments.iter().all(|c| self.compartments.contains(c))
    }

    /// Whether the subject holds a given compartment (used by retriever
    /// entitlement filtering).
    pub fn holds(&self, c: &Compartment) -> bool {
        self.compartments.contains(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering() {
        assert!(ClearanceLevel::Mnpi > ClearanceLevel::Public);
        assert!(ClearanceLevel::Restricted > ClearanceLevel::Internal);
    }

    #[test]
    fn permits_requires_level_and_all_compartments() {
        let ctx = ClearanceContext::new("alice", ClearanceLevel::Restricted)
            .with_compartment("deal:acme")
            .with_compartment("desk:credit");

        // Level high enough, both compartments held.
        let ok = NeedToKnow::level(ClearanceLevel::Confidential).with_compartment("deal:acme");
        assert!(ctx.permits(&ok));

        // Missing compartment -> denied.
        let missing = NeedToKnow::level(ClearanceLevel::Public).with_compartment("deal:other");
        assert!(!ctx.permits(&missing));

        // Level too low -> denied.
        let too_high = NeedToKnow::level(ClearanceLevel::Mnpi);
        assert!(!ctx.permits(&too_high));
    }
}
