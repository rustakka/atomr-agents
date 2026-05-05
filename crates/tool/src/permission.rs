use atomr_agents_core::ToolSetId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionSpec {
    /// Tool sets this spec depends on transitively.
    pub depends_on: Vec<ToolSetId>,
    /// If true, requires explicit grant from the next level up.
    #[serde(default)]
    pub requires_explicit_grant: bool,
}
