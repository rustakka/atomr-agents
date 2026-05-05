//! Output parsers + auto-repair + streaming partial-JSON.
//!
//! `Parser<T>` is the single trait. Stock impls cover JSON,
//! schema-validated objects, enum, comma-separated list, XML, and
//! YAML. Two repair wrappers — `OutputFixingParser` and
//! `RetryWithErrorParser` — re-invoke a model when the inner parse
//! fails. `StreamingPartialJsonParser` emits partial JSON values as
//! tokens arrive.

mod auto_repair;
mod basic;
mod streaming;

pub use auto_repair::{OutputFixingParser, RepairModel, RetryWithErrorParser};
pub use basic::{
    CommaListParser, EnumParser, JsonParser, JsonSchemaParser, SchemaParser, XmlParser, YamlParser,
};
pub use streaming::StreamingPartialJsonParser;

use async_trait::async_trait;
use atomr_agents_core::Result;

#[async_trait]
pub trait Parser<T>: Send + Sync + 'static {
    async fn parse(&self, raw: &str) -> Result<T>;
    fn format_instructions(&self) -> String;
}
