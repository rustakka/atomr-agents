//! M8 — MCP bridge. Stub transport: holds the configured tool list
//! and dispatches `Call` via an optional in-process mock handler.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{HostError, HostResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPServerConfig {
    pub id: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub tools: Vec<MCPToolSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub schema: serde_json::Value,
}

pub type MockHandler = Arc<
    dyn Fn(&str, &serde_json::Value) -> HostResult<serde_json::Value> + Send + Sync + 'static,
>;

#[derive(Clone)]
pub struct McpBridge {
    config: MCPServerConfig,
    mock: Arc<Mutex<Option<MockHandler>>>,
}

impl std::fmt::Debug for McpBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpBridge").field("config", &self.config).finish()
    }
}

impl McpBridge {
    pub fn new(config: MCPServerConfig) -> Self {
        Self { config, mock: Arc::new(Mutex::new(None)) }
    }

    pub fn config(&self) -> &MCPServerConfig {
        &self.config
    }

    pub fn set_mock(&self, handler: MockHandler) {
        *self.mock.lock() = Some(handler);
    }

    pub fn tools(&self) -> &[MCPToolSpec] {
        &self.config.tools
    }

    pub async fn call(&self, name: &str, args: &serde_json::Value) -> HostResult<serde_json::Value> {
        let handler = self.mock.lock().clone();
        match handler {
            Some(h) => h(name, args),
            None => Err(HostError::Mcp(format!(
                "M8 ships only a stub transport — no mock handler installed for `{}` (call={})",
                self.config.id, name
            ))),
        }
    }
}

pub fn load_mcp_servers(mcp_dir: &Path) -> HostResult<Vec<MCPServerConfig>> {
    if !mcp_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(mcp_dir).map_err(|e| HostError::io(mcp_dir, e))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "yaml" | "yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| HostError::io(&path, e))?;
        let cfg: MCPServerConfig = serde_yaml::from_str(&text).map_err(|e| HostError::yaml(&path, e))?;
        out.push(cfg);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub fn scaffold_mcp_server(
    mcp_dir: &Path,
    id: &str,
    command: Vec<String>,
) -> HostResult<std::path::PathBuf> {
    std::fs::create_dir_all(mcp_dir).map_err(|e| HostError::io(mcp_dir, e))?;
    let cfg = MCPServerConfig { id: id.to_string(), command, env: Default::default(), tools: Vec::new() };
    let body = serde_yaml::to_string(&cfg).map_err(|e| HostError::yaml(mcp_dir.join(format!("{id}.yaml")), e))?;
    let path = mcp_dir.join(format!("{id}.yaml"));
    std::fs::write(&path, body).map_err(|e| HostError::io(&path, e))?;
    Ok(path)
}
