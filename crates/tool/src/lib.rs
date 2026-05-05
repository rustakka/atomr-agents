//! Tools, tool sets, registry, and the tool-call parser.

mod descriptor;
mod handoff;
mod parser;
mod permission;
mod registry;
mod strategies;
mod toolset;
mod r#trait;
mod tool_return;

pub use descriptor::{ToolDescriptor, ToolSchema};
pub use handoff::HandoffTool;
pub use parser::{ParsedToolCall, Provider, ToolCallParser};
pub use permission::PermissionSpec;
pub use registry::ToolSetRegistry;
pub use strategies::{KeywordToolStrategy, StaticToolStrategy};
pub use tool_return::{RichTool, ToolControl, ToolReturn};
pub use toolset::{ToolSet, ToolSetMeta};
pub use r#trait::{DynTool, Tool};
