use atomr_agents_core::ToolSetId;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::permission::PermissionSpec;
use crate::r#trait::DynTool;

/// Free-form metadata attached to a tool set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSetMeta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
}

/// Versioned bundle. Lives behind an `Arc` in the registry.
#[derive(Clone)]
pub struct ToolSet {
    pub id: ToolSetId,
    pub version: Version,
    pub tools: Vec<DynTool>,
    pub metadata: ToolSetMeta,
    pub dependencies: Vec<ToolSetId>,
    pub permissions: PermissionSpec,
}

impl ToolSet {
    pub fn new(id: impl Into<ToolSetId>, version: Version, tools: Vec<DynTool>) -> Self {
        Self {
            id: id.into(),
            version,
            tools,
            metadata: ToolSetMeta::default(),
            dependencies: vec![],
            permissions: PermissionSpec::default(),
        }
    }
}
