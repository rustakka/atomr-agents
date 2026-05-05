use atomr_agents_core::{Result, ToolSetId};
use serde::{Deserialize, Serialize};

/// Policy is inherited and *narrowed* downward. Each level can
/// remove grants but never add ones the parent didn't grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Policy {
    pub allowed_toolsets: Vec<ToolSetId>,
    pub max_tokens_per_call: Option<u32>,
    pub max_money_micro_usd_per_call: Option<u64>,
    pub allowed_models: Vec<String>,
}

impl Policy {
    /// Compose two policies by intersection (child narrows parent).
    pub fn narrow(parent: &Policy, child: &Policy) -> Policy {
        let allowed_toolsets = if child.allowed_toolsets.is_empty() {
            parent.allowed_toolsets.clone()
        } else {
            child
                .allowed_toolsets
                .iter()
                .filter(|c| parent.allowed_toolsets.iter().any(|p| p.as_str() == c.as_str()))
                .cloned()
                .collect()
        };
        let allowed_models = if child.allowed_models.is_empty() {
            parent.allowed_models.clone()
        } else {
            child
                .allowed_models
                .iter()
                .filter(|m| parent.allowed_models.contains(m))
                .cloned()
                .collect()
        };
        let max_tokens_per_call = match (parent.max_tokens_per_call, child.max_tokens_per_call) {
            (Some(p), Some(c)) => Some(p.min(c)),
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };
        let max_money_micro_usd_per_call = match (
            parent.max_money_micro_usd_per_call,
            child.max_money_micro_usd_per_call,
        ) {
            (Some(p), Some(c)) => Some(p.min(c)),
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };
        Policy {
            allowed_toolsets,
            allowed_models,
            max_tokens_per_call,
            max_money_micro_usd_per_call,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
}

pub trait PolicyStrategy: Send + Sync + 'static {
    fn evaluate(&self, policy: &Policy, requested_toolset: Option<&ToolSetId>) -> Result<PolicyDecision>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_intersects_grants() {
        let parent = Policy {
            allowed_toolsets: vec!["a".into(), "b".into(), "c".into()],
            max_tokens_per_call: Some(1000),
            ..Default::default()
        };
        let child = Policy {
            allowed_toolsets: vec!["b".into(), "d".into()],
            max_tokens_per_call: Some(500),
            ..Default::default()
        };
        let resolved = Policy::narrow(&parent, &child);
        let names: Vec<&str> = resolved.allowed_toolsets.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, vec!["b"]);
        assert_eq!(resolved.max_tokens_per_call, Some(500));
    }
}
