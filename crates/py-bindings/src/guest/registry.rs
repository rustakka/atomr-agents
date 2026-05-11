//! Process-wide guest registries.
//!
//! Generic factories store any `PyObject`; tool factories additionally
//! carry their descriptor so the Rust adapter advertises the right
//! schema when handed off to a `ToolSet`.

use std::sync::Arc;

use atomr_agents_tool::ToolDescriptor;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use pyo3::prelude::*;

/// Per-tool registry entry: descriptor + the Python target.
#[derive(Clone)]
pub(crate) struct ToolEntry {
    pub(crate) descriptor: ToolDescriptor,
    pub(crate) target: Arc<PyObject>,
}

/// Process-wide generic registry. Keyed on `(kind, key)`. The `kind`
/// for strategy-flavored entries is `strategy:{instruction|tool|memory|skill}`.
pub(crate) static GUESTS: Lazy<DashMap<(String, String), Arc<PyObject>>> = Lazy::new(DashMap::new);

/// Process-wide tool registry. Carries the descriptor.
pub(crate) static TOOLS: Lazy<DashMap<String, ToolEntry>> = Lazy::new(DashMap::new);
