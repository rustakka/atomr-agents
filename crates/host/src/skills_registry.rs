//! Skill selection + scaffold helpers (M4).

use std::path::Path;

use crate::error::{HostError, HostResult};
use crate::layout::AgentPaths;
use crate::loader::SkillDefinition;
use crate::markdown::MarkdownDoc;

#[derive(Debug, Clone)]
pub struct SkillValidationReport {
    pub skill_id: String,
    pub path: std::path::PathBuf,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl SkillValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Sort skills by substring-match against `user_message` then by
/// `(-priority, id)` — mirrors the pre-port Python semantics.
pub fn select_skills_for<'a>(skills: &'a [SkillDefinition], user_message: &str) -> Vec<&'a SkillDefinition> {
    let lower = user_message.to_lowercase();
    let mut hits: Vec<&SkillDefinition> = skills
        .iter()
        .filter(|sd| sd.keywords.iter().any(|kw| lower.contains(&kw.to_lowercase())))
        .collect();
    hits.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    hits
}

/// Scaffold a new SKILL.md under `<agent>/skills/<skill_id>/`.
pub fn scaffold_skill(
    paths: &AgentPaths,
    skill_id: &str,
    name: &str,
    priority: u8,
    keywords: &[String],
) -> HostResult<std::path::PathBuf> {
    let skill_dir = paths.skills_dir().join(skill_id);
    std::fs::create_dir_all(&skill_dir).map_err(|e| HostError::io(&skill_dir, e))?;
    let md_path = skill_dir.join("SKILL.md");
    if md_path.exists() {
        return Err(HostError::skill(skill_id, format!("{} already exists", md_path.display())));
    }
    let mut fm = format!("---\nname: {name}\npriority: {priority}\n");
    if !keywords.is_empty() {
        fm.push_str("keywords:\n");
        for kw in keywords {
            fm.push_str(&format!("  - {kw}\n"));
        }
    }
    fm.push_str("---\nDescribe this skill.\n");
    std::fs::write(&md_path, fm).map_err(|e| HostError::io(&md_path, e))?;
    Ok(md_path)
}

/// Walk `<agent>/skills/` and emit one report per skill directory.
pub fn validate_skills(paths: &AgentPaths) -> HostResult<Vec<SkillValidationReport>> {
    let skills_dir = paths.skills_dir();
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&skills_dir)
        .map_err(|e| HostError::io(&skills_dir, e))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        let md = path.join("SKILL.md");
        let mut report = SkillValidationReport {
            skill_id: name.clone(),
            path: md.clone(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        if !md.is_file() {
            report.errors.push(format!("missing SKILL.md at {}", md.display()));
            out.push(report);
            continue;
        }
        match MarkdownDoc::read(&md) {
            Ok(doc) => {
                if doc.frontmatter.is_empty() {
                    report.warnings.push("no YAML frontmatter".into());
                }
                if doc.body.trim().is_empty() {
                    report.warnings.push("empty body — no instruction fragment".into());
                }
            }
            Err(e) => report.errors.push(e.to_string()),
        }
        out.push(report);
    }
    Ok(out)
}

/// Helper used by tests: walk a directory checking that every contained
/// SKILL.md parses.
pub fn quick_check(skill_md: &Path) -> HostResult<MarkdownDoc> {
    MarkdownDoc::read(skill_md)
}
