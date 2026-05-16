//! M9 — Skill curator + CurationStrategy.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};
use crate::events::EventLog;
use crate::layout::AgentPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillProposal {
    pub agent_id: String,
    pub skill_id: String,
    pub name: String,
    pub body: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub tool_overlay: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub success_rate: Option<f64>,
}

fn default_priority() -> u8 {
    5
}

impl SkillProposal {
    pub fn to_markdown(&self) -> String {
        let mut s = String::from("---\n");
        s.push_str(&format!("name: {}\n", self.name));
        s.push_str(&format!("priority: {}\n", self.priority));
        if !self.keywords.is_empty() {
            s.push_str("keywords:\n");
            for kw in &self.keywords {
                s.push_str(&format!("  - {kw}\n"));
            }
        }
        if !self.tool_overlay.is_empty() {
            s.push_str("tool_overlay:\n");
            for t in &self.tool_overlay {
                s.push_str(&format!("  - {t}\n"));
            }
        }
        if let Some(r) = &self.rationale {
            s.push_str(&format!("rationale: {r}\n"));
        }
        s.push_str("---\n");
        s.push_str(&self.body);
        if !self.body.ends_with('\n') {
            s.push('\n');
        }
        s
    }
}

pub enum CurationOutcome {
    Promoted { path: PathBuf },
    Proposed { path: PathBuf },
    Rejected { reason: String },
}

pub struct CurationCtx {
    pub paths: AgentPaths,
    pub events: Option<EventLog>,
}

pub trait CurationStrategy: Send + Sync {
    fn handle(&self, proposal: SkillProposal, ctx: &CurationCtx) -> HostResult<CurationOutcome>;
}

#[derive(Debug, Clone)]
pub struct AutoPromoteCurationStrategy {
    pub min_success_rate: Option<f64>,
    pub history_limit: usize,
}

impl Default for AutoPromoteCurationStrategy {
    fn default() -> Self {
        Self { min_success_rate: None, history_limit: 20 }
    }
}

impl CurationStrategy for AutoPromoteCurationStrategy {
    fn handle(&self, proposal: SkillProposal, ctx: &CurationCtx) -> HostResult<CurationOutcome> {
        if let (Some(threshold), Some(rate)) = (self.min_success_rate, proposal.success_rate) {
            if rate < threshold {
                return Ok(CurationOutcome::Rejected {
                    reason: format!("success_rate {rate:.2} < threshold {threshold:.2}"),
                });
            }
        }
        let path = promote_proposal(&ctx.paths, &proposal, self.history_limit)?;
        if let Some(ev) = &ctx.events {
            let _ = ev.emit(
                "SkillPromoted",
                Some(proposal.agent_id.clone()),
                serde_json::json!({"skill_id": proposal.skill_id, "path": path.display().to_string()}),
            );
        }
        Ok(CurationOutcome::Promoted { path })
    }
}

#[derive(Debug, Clone, Default)]
pub struct HumanApprovalCurationStrategy;

impl CurationStrategy for HumanApprovalCurationStrategy {
    fn handle(&self, proposal: SkillProposal, ctx: &CurationCtx) -> HostResult<CurationOutcome> {
        let path = write_proposal(&ctx.paths, &proposal)?;
        if let Some(ev) = &ctx.events {
            let _ = ev.emit(
                "SkillProposed",
                Some(proposal.agent_id.clone()),
                serde_json::json!({"skill_id": proposal.skill_id, "path": path.display().to_string()}),
            );
        }
        Ok(CurationOutcome::Proposed { path })
    }
}

// ---------- core operations -------------------------------------------------

pub fn promote_proposal(
    paths: &AgentPaths,
    proposal: &SkillProposal,
    history_limit: usize,
) -> HostResult<PathBuf> {
    let target_dir = paths.skills_dir().join(&proposal.skill_id);
    std::fs::create_dir_all(&target_dir).map_err(|e| HostError::io(&target_dir, e))?;
    let target = target_dir.join("SKILL.md");
    if target.is_file() {
        let history = target_dir.join(".history");
        std::fs::create_dir_all(&history).map_err(|e| HostError::io(&history, e))?;
        let ts = now_ms();
        let snapshot = history.join(format!("{ts}.md"));
        std::fs::copy(&target, &snapshot).map_err(|e| HostError::io(&snapshot, e))?;
        prune_history(&history, history_limit)?;
    }
    std::fs::write(&target, proposal.to_markdown()).map_err(|e| HostError::io(&target, e))?;
    Ok(target)
}

pub fn write_proposal(paths: &AgentPaths, proposal: &SkillProposal) -> HostResult<PathBuf> {
    let proposed_dir = paths.skills_dir().join(".proposed").join(&proposal.skill_id);
    std::fs::create_dir_all(&proposed_dir).map_err(|e| HostError::io(&proposed_dir, e))?;
    let target = proposed_dir.join("SKILL.md");
    std::fs::write(&target, proposal.to_markdown()).map_err(|e| HostError::io(&target, e))?;
    Ok(target)
}

pub fn reject_proposal(paths: &AgentPaths, skill_id: &str) -> HostResult<bool> {
    let proposed_dir = paths.skills_dir().join(".proposed").join(skill_id);
    if !proposed_dir.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&proposed_dir).map_err(|e| HostError::io(&proposed_dir, e))?;
    Ok(true)
}

pub fn revert_skill(paths: &AgentPaths, skill_id: &str) -> HostResult<Option<PathBuf>> {
    let history = paths.skills_dir().join(skill_id).join(".history");
    if !history.is_dir() {
        return Ok(None);
    }
    let mut entries: Vec<_> = std::fs::read_dir(&history)
        .map_err(|e| HostError::io(&history, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let last = match entries.pop() {
        Some(e) => e.path(),
        None => return Ok(None),
    };
    let target = paths.skills_dir().join(skill_id).join("SKILL.md");
    std::fs::copy(&last, &target).map_err(|e| HostError::io(&target, e))?;
    Ok(Some(target))
}

pub fn list_proposals(paths: &AgentPaths) -> HostResult<Vec<String>> {
    let dir = paths.skills_dir().join(".proposed");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| HostError::io(&dir, e))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Ok(name) = entry.file_name().into_string() {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn list_history(paths: &AgentPaths, skill_id: &str) -> HostResult<Vec<PathBuf>> {
    let history = paths.skills_dir().join(skill_id).join(".history");
    if !history.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&history)
        .map_err(|e| HostError::io(&history, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    Ok(entries)
}

fn prune_history(history: &std::path::Path, keep: usize) -> HostResult<()> {
    let mut entries: Vec<_> = std::fs::read_dir(history)
        .map_err(|e| HostError::io(history, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    while entries.len() > keep {
        if let Some(p) = entries.first().cloned() {
            let _ = std::fs::remove_file(&p);
            entries.remove(0);
        } else {
            break;
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
