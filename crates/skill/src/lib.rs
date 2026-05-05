//! Skills: bundled instruction-fragment + tool overlay + optional
//! sub-agents + optional memory namespace.

use async_trait::async_trait;
use atomr_agents_core::{AgentContext, MemoryNamespace, Result, SkillId, TokenBudget, ToolId};
use atomr_agents_strategy::{SkillRef, SkillStrategy};
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: SkillId,
    pub name: String,
    pub instruction_fragment: String,
    #[serde(default)]
    pub tool_overlay: Vec<ToolId>,
    #[serde(default)]
    pub memory_namespace: Option<MemoryNamespace>,
    /// Keywords that trigger `KeywordSkillStrategy`.
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
}

fn default_priority() -> u8 {
    5
}

#[derive(Clone)]
pub struct SkillSet {
    pub id: String,
    pub version: Version,
    pub skills: Vec<Skill>,
}

impl SkillSet {
    pub fn new(id: impl Into<String>, version: Version, skills: Vec<Skill>) -> Self {
        Self {
            id: id.into(),
            version,
            skills,
        }
    }
}

/// Always picks the same fixed list of skills.
pub struct StaticSkillStrategy {
    skills: Vec<Skill>,
}

impl StaticSkillStrategy {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl SkillStrategy for StaticSkillStrategy {
    async fn applicable(&self, _ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<Vec<SkillRef>> {
        Ok(self
            .skills
            .iter()
            .map(|s| SkillRef {
                id: s.id.clone(),
                name: s.name.clone(),
                priority: s.priority,
            })
            .collect())
    }
}

/// Returns the skills whose `keywords` overlap with the user turn.
pub struct KeywordSkillStrategy {
    skills: Vec<Skill>,
}

impl KeywordSkillStrategy {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl SkillStrategy for KeywordSkillStrategy {
    async fn applicable(&self, ctx: &AgentContext, _budget: &mut TokenBudget) -> Result<Vec<SkillRef>> {
        let needle = ctx.turn.user.to_lowercase();
        let mut out: Vec<SkillRef> = self
            .skills
            .iter()
            .filter(|s| s.keywords.iter().any(|k| needle.contains(&k.to_lowercase())))
            .map(|s| SkillRef {
                id: s.id.clone(),
                name: s.name.clone(),
                priority: s.priority,
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.priority));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{AgentId, TurnInput};

    fn ctx(text: &str) -> AgentContext {
        AgentContext::for_agent(
            AgentId::from("a-1"),
            TurnInput {
                user: text.into(),
                history: vec![],
            },
        )
    }

    #[tokio::test]
    async fn keyword_picks_matching_skills() {
        let s1 = Skill {
            id: SkillId::from("rag"),
            name: "RAG".into(),
            instruction_fragment: "use the index".into(),
            tool_overlay: vec![],
            memory_namespace: None,
            keywords: vec!["search".into(), "lookup".into()],
            priority: 7,
        };
        let s2 = Skill {
            id: SkillId::from("math"),
            name: "Math".into(),
            instruction_fragment: "use the calculator".into(),
            tool_overlay: vec![],
            memory_namespace: None,
            keywords: vec!["compute".into()],
            priority: 3,
        };
        let strat = KeywordSkillStrategy::new(vec![s1, s2]);
        let mut b = TokenBudget::new(1000);
        let out = strat
            .applicable(&ctx("please search for x"), &mut b)
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "RAG");
    }
}
