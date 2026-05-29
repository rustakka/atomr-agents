use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Value};

use crate::error::SecurityError;

/// A pre-trade / pre-action boundary checked at the `Tool` seam before a
/// guarded tool runs. hedgehog supplies the concrete mandate logic
/// (position limits, restricted lists, notional caps); the framework
/// only defines the contract and drives it from [`crate::WalledTool`].
#[async_trait]
pub trait Mandate: Send + Sync + 'static {
    /// Inspect the about-to-run tool call. Return `Err` to block it.
    async fn check(&self, args: &Value, ctx: &InvokeCtx) -> Result<(), SecurityError>;
}

/// A mandate that allows everything — useful as a default / in tests.
pub struct AllowAll;

#[async_trait]
impl Mandate for AllowAll {
    async fn check(&self, _args: &Value, _ctx: &InvokeCtx) -> Result<(), SecurityError> {
        Ok(())
    }
}

/// A mandate that blocks everything with a fixed reason — handy for
/// fail-closed defaults and tests.
pub struct DenyAll(pub &'static str);

#[async_trait]
impl Mandate for DenyAll {
    async fn check(&self, _args: &Value, _ctx: &InvokeCtx) -> Result<(), SecurityError> {
        Err(SecurityError::MandateViolation(self.0.to_string()))
    }
}
