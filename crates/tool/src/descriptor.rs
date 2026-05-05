use atomr_agents_core::{ToolId, Value};
use serde::{Deserialize, Serialize};

/// Descriptor advertised to the model. The schema is JSON-Schema
/// shaped; per-provider renderers in this crate adapt it to OpenAI /
/// Anthropic / Gemini call formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub id: ToolId,
    pub name: String,
    pub description: String,
    pub schema: ToolSchema,
}

/// JSON-Schema fragment for the tool's arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema(pub Value);

impl ToolSchema {
    pub fn empty_object() -> Self {
        Self(serde_json::json!({"type": "object", "properties": {}}))
    }
}
