use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, OrgId, TeamId};
use crate::value::Value;

/// Stored unit. The strategy decides what to put in `payload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub kind: MemoryKind,
    pub namespace: MemoryNamespace,
    pub payload: Value,
    pub timestamp_ms: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Retrieval unit. Memory strategies emit chunks; the assembler packs
/// them into the prompt under the shared budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    pub source_id: String,
    pub text: String,
    pub score: f32,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Working,
    Scratchpad,
}

/// Where a memory item lives in the org/team/agent hierarchy.
/// Reads cascade outward (agent → team → org); writes are gated by
/// policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryNamespace {
    Agent(AgentId),
    Team(TeamId),
    Org(OrgId),
}
