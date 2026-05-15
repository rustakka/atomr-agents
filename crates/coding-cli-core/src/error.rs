//! Errors produced by vendor adapters.

use thiserror::Error;

/// A vendor-side stream parser failed to interpret a line.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("malformed json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("unsupported event shape: {0}")]
    Unsupported(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid value for {field}: {message}")]
    InvalidValue { field: &'static str, message: String },
}

/// A vendor adapter failed to project atomr concepts onto its on-disk
/// config (`CLAUDE.md`, `.cursor/rules/*`, `AGENTS.md`, MCP, ...).
#[derive(Debug, Error)]
pub enum MapperError {
    #[error("filesystem error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("vendor does not support concept {0}")]
    Unsupported(&'static str),
}

impl MapperError {
    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
