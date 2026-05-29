use std::collections::HashMap;

use async_trait::async_trait;
use atomr_agents_core::CallCtx;

use crate::clearance::{ClearanceContext, NeedToKnow};
use crate::error::SecurityError;

/// An opaque, LLM-visible reference to a credential. The model may pass
/// a handle around (it is just an id); it can never read the underlying
/// secret — only the [`CapabilityBroker`] can, and only after a
/// clearance check.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityHandle(pub String);

impl CapabilityHandle {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A use-and-drop credential. Deliberately:
/// * NOT `Serialize`/`Deserialize` — it can never be written into a
///   prompt, checkpoint, or telemetry record;
/// * `Debug` is redacted;
/// * `Drop` overwrites the backing bytes.
///
/// Read the value exactly when you need it via [`ScopedSecret::expose`].
pub struct ScopedSecret {
    value: String,
}

impl ScopedSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self { value: value.into() }
    }

    /// Borrow the secret value for immediate use. Do not clone or log it.
    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Debug for ScopedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScopedSecret(***redacted***)")
    }
}

impl Drop for ScopedSecret {
    fn drop(&mut self) {
        // Best-effort scrub so the secret does not linger in freed memory.
        // SAFETY: we only overwrite bytes we own.
        let bytes = unsafe { self.value.as_bytes_mut() };
        for b in bytes.iter_mut() {
            *b = 0;
        }
    }
}

/// Resolves [`CapabilityHandle`]s to [`ScopedSecret`]s, enforcing
/// need-to-know against the caller's [`ClearanceContext`] (attached to
/// the [`CallCtx`] extension map). Agents never hold credentials; they
/// hold handles, and the broker is the only path to the secret.
#[async_trait]
pub trait CapabilityBroker: Send + Sync + 'static {
    async fn resolve(&self, handle: &CapabilityHandle, ctx: &CallCtx) -> Result<ScopedSecret, SecurityError>;
}

struct Entry {
    need: NeedToKnow,
    secret: String,
}

/// An in-memory broker for tests / single-process hosts. Each handle is
/// registered with a [`NeedToKnow`] gate and its secret value.
#[derive(Default)]
pub struct StaticCapabilityBroker {
    entries: HashMap<String, Entry>,
}

impl StaticCapabilityBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handle, its access gate, and its secret value.
    pub fn register(
        mut self,
        handle: impl Into<String>,
        need: NeedToKnow,
        secret: impl Into<String>,
    ) -> Self {
        self.entries.insert(
            handle.into(),
            Entry {
                need,
                secret: secret.into(),
            },
        );
        self
    }
}

#[async_trait]
impl CapabilityBroker for StaticCapabilityBroker {
    async fn resolve(&self, handle: &CapabilityHandle, ctx: &CallCtx) -> Result<ScopedSecret, SecurityError> {
        let entry = self
            .entries
            .get(handle.as_str())
            .ok_or_else(|| SecurityError::UnknownCapability(handle.as_str().to_string()))?;

        // Fail-closed: no clearance context attached -> deny.
        let clearance = ctx.ext::<ClearanceContext>().ok_or_else(|| {
            SecurityError::AccessDenied(format!(
                "no clearance context present for capability '{}'",
                handle.as_str()
            ))
        })?;

        if !clearance.permits(&entry.need) {
            return Err(SecurityError::AccessDenied(format!(
                "subject '{}' lacks need-to-know for capability '{}'",
                clearance.subject,
                handle.as_str()
            )));
        }

        Ok(ScopedSecret::new(entry.secret.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clearance::ClearanceLevel;
    use atomr_agents_core::{IterationBudget, MoneyBudget, TimeBudget, TokenBudget};

    fn ctx_with(clearance: Option<ClearanceContext>) -> CallCtx {
        let mut ctx = CallCtx::new(
            None,
            TokenBudget::new(100),
            TimeBudget::new(std::time::Duration::from_secs(1)),
            MoneyBudget::from_usd(1.0),
            IterationBudget::new(1),
            vec![],
        );
        if let Some(c) = clearance {
            ctx.insert_ext(c);
        }
        ctx
    }

    #[tokio::test]
    async fn resolves_only_with_sufficient_clearance() {
        let broker = StaticCapabilityBroker::new().register(
            "broker-api-key",
            NeedToKnow::level(ClearanceLevel::Restricted).with_compartment("desk:credit"),
            "sk-secret-123",
        );
        let handle = CapabilityHandle::new("broker-api-key");

        // Sufficient clearance -> resolves.
        let good = ClearanceContext::new("alice", ClearanceLevel::Restricted).with_compartment("desk:credit");
        let secret = broker.resolve(&handle, &ctx_with(Some(good))).await.unwrap();
        assert_eq!(secret.expose(), "sk-secret-123");

        // No clearance context -> fail closed.
        assert!(broker.resolve(&handle, &ctx_with(None)).await.is_err());

        // Insufficient compartment -> denied.
        let weak = ClearanceContext::new("bob", ClearanceLevel::Mnpi);
        assert!(broker.resolve(&handle, &ctx_with(Some(weak))).await.is_err());

        // Unknown handle.
        assert!(broker
            .resolve(&CapabilityHandle::new("nope"), &ctx_with(None))
            .await
            .is_err());
    }
}
