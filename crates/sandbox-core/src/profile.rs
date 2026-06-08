//! Toolchain profiles and the languages they can execute.
//!
//! The profile drives two things: which rootFS image the backend boots,
//! and the minimum [`ResourceBudget`](crate::ResourceBudget). Profiles that
//! contain a Rust toolchain force a higher floor (see `budget.rs`).

use serde::{Deserialize, Serialize};

use crate::error::{Result, SandboxError};

/// A language an agent can ask the sandbox to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    Bash,
    Js,
    Rust,
}

/// RootFS / toolchain profile. The orchestrator keeps pre-warmed snapshot
/// pools per profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    PythonOnly,
    NpmOnly,
    RustOnly,
    PythonAndNpm,
    FullStack,
}

impl SandboxProfile {
    /// Whether code in `lang` can run under this profile. `Bash` is always
    /// available (every rootFS ships a shell).
    pub fn supports(self, lang: Language) -> bool {
        use Language::*;
        use SandboxProfile::*;
        if lang == Bash {
            return true;
        }
        match self {
            PythonOnly => lang == Python,
            NpmOnly => lang == Js,
            RustOnly => lang == Rust,
            PythonAndNpm => matches!(lang, Python | Js),
            FullStack => true,
        }
    }

    /// `true` for profiles carrying a Rust toolchain — these enforce a
    /// higher [`ResourceBudget`](crate::ResourceBudget) floor.
    pub fn is_rust(self) -> bool {
        matches!(self, SandboxProfile::RustOnly | SandboxProfile::FullStack)
    }

    /// Fail-fast guard with a single canonical error string, shared by the
    /// harness pre-check and every backend's `exec` path (so the rule and its
    /// message live in exactly one place).
    pub fn ensure_supports(self, lang: Language) -> Result<()> {
        if self.supports(lang) {
            Ok(())
        } else {
            Err(SandboxError::BudgetViolation(format!(
                "profile {self:?} cannot run language {lang:?}"
            )))
        }
    }

    /// Logical image tag, resolved to a concrete kernel + rootFS (Firecracker)
    /// or container image (Docker) by the backend's image resolver.
    pub fn image_tag(self) -> &'static str {
        use SandboxProfile::*;
        match self {
            PythonOnly => "sandbox/python-only",
            NpmOnly => "sandbox/npm-only",
            RustOnly => "sandbox/rust-only",
            PythonAndNpm => "sandbox/python-and-npm",
            FullStack => "sandbox/full-stack",
        }
    }

    /// Smallest profile that can run `lang`, used when a caller supplies a
    /// language but no explicit profile.
    pub fn for_language(lang: Language) -> Self {
        match lang {
            Language::Python => SandboxProfile::PythonOnly,
            Language::Js => SandboxProfile::NpmOnly,
            Language::Rust => SandboxProfile::RustOnly,
            // Bash needs no toolchain; FullStack is the safe superset.
            Language::Bash => SandboxProfile::FullStack,
        }
    }
}
