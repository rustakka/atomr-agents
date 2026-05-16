//! Lightweight Markdown reader for SOUL/RULES/MEMORY/USER/SKILL.md.
//!
//! The host doesn't depend on a Markdown engine — the only structure
//! it cares about is an optional YAML frontmatter delimited by `---`.
//! Bodies stay as plain text and the consumer extracts bullets / facts
//! at higher levels.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownDoc {
    /// Path the doc was loaded from, if any.
    #[serde(default)]
    pub source_path: Option<std::path::PathBuf>,
    /// Parsed YAML frontmatter as a flat string-keyed map. The value is
    /// preserved as JSON so non-string types (lists, nested maps, ints)
    /// round-trip cleanly between YAML and the Python facade.
    #[serde(default)]
    pub frontmatter: BTreeMap<String, serde_json::Value>,
    /// Body content after the frontmatter delimiter (or the whole file
    /// when no frontmatter is present).
    pub body: String,
}

impl MarkdownDoc {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.frontmatter.is_empty() && self.body.trim().is_empty()
    }

    pub fn read(path: &Path) -> HostResult<Self> {
        if !path.is_file() {
            return Ok(Self::empty());
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| HostError::io(path, e))?;
        let mut doc = Self::parse_str(&text, Some(path))?;
        doc.source_path = Some(path.to_path_buf());
        Ok(doc)
    }

    pub fn parse_str(text: &str, path: Option<&Path>) -> HostResult<Self> {
        if let Some(rest) = text.strip_prefix("---\n") {
            if let Some(end_idx) = rest.find("\n---\n") {
                let yaml_part = &rest[..end_idx];
                let body = rest[end_idx + 5..].to_string();
                let fm: serde_yaml::Value = serde_yaml::from_str(yaml_part).map_err(|e| {
                    HostError::yaml(path.unwrap_or(Path::new("<inline>")).to_path_buf(), e)
                })?;
                let map = match fm {
                    serde_yaml::Value::Mapping(m) => m,
                    serde_yaml::Value::Null => Default::default(),
                    _ => {
                        return Err(HostError::markdown(
                            path.unwrap_or(Path::new("<inline>")).to_path_buf(),
                            "frontmatter must be a YAML mapping",
                        ))
                    }
                };
                let mut fm_map = BTreeMap::new();
                for (k, v) in map {
                    let ks = match k {
                        serde_yaml::Value::String(s) => s,
                        other => match serde_yaml::to_string(&other) {
                            Ok(s) => s.trim().to_string(),
                            Err(_) => continue,
                        },
                    };
                    fm_map.insert(ks, crate::config::yaml_to_json(v));
                }
                return Ok(Self { source_path: path.map(|p| p.to_path_buf()), frontmatter: fm_map, body });
            }
            // Opening `---` with no closer: treat as plain text.
        }
        Ok(Self { source_path: path.map(|p| p.to_path_buf()), frontmatter: Default::default(), body: text.to_string() })
    }
}

/// Bullet prefixes recognized by [`split_bullets`].
pub(crate) const BULLET_PREFIXES: &[&str] = &["- ", "* ", "+ "];

/// Extract bulleted lines from a RULES/MEMORY/USER body.
///
/// Only Markdown list items (`- `, `* `, `+ `) become entries.
/// Headings, prose paragraphs, and blank lines are skipped so authors
/// can leave explanatory copy in the file without it leaking into the
/// instruction prefix.
pub fn split_bullets(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        for prefix in BULLET_PREFIXES {
            if let Some(rest) = stripped.strip_prefix(prefix) {
                let rule = rest.trim().to_string();
                if !rule.is_empty() {
                    out.push(rule);
                }
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nidentity: Alpha\nstyle:\n  tone: terse\n---\nbody line\nanother\n";
        let doc = MarkdownDoc::parse_str(text, None).unwrap();
        assert_eq!(doc.frontmatter.get("identity").and_then(|v| v.as_str()), Some("Alpha"));
        assert!(doc.body.contains("body line"));
    }

    #[test]
    fn handles_no_frontmatter() {
        let doc = MarkdownDoc::parse_str("just body\n", None).unwrap();
        assert!(doc.frontmatter.is_empty());
        assert_eq!(doc.body, "just body\n");
    }

    #[test]
    fn split_bullets_skips_prose_and_headings() {
        let body = "# Title\n\nSome prose.\n\n- one\n* two\n+ three\n\nmore prose\n";
        assert_eq!(split_bullets(body), vec!["one", "two", "three"]);
    }
}
