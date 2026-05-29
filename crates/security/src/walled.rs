use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{InvokeCtx, Result, Value};
use atomr_agents_tool::{Tool, ToolDescriptor};

use crate::clearance::{ClearanceContext, NeedToKnow};
use crate::error::SecurityError;
use crate::mandate::Mandate;

/// Generic information-wall middleware. Wraps ANY [`Tool`] without
/// touching its constructor and, before delegating:
///
/// 1. reads the caller's [`ClearanceContext`] from the [`InvokeCtx`]
///    extension map and **fails closed** (typed `AccessDenied`) if it is
///    absent or insufficient for the configured [`NeedToKnow`];
/// 2. runs an optional [`Mandate`] pre-trade check.
///
/// Only if both pass does the inner tool run. This is the single,
/// reusable wall — a tool cannot "forget the wrapper" the way per-tool
/// clearance baked into a constructor can.
pub struct WalledTool<T: Tool> {
    inner: T,
    need: NeedToKnow,
    mandate: Option<Arc<dyn Mandate>>,
}

impl<T: Tool> WalledTool<T> {
    /// Wrap `inner`, requiring `need` of every caller.
    pub fn new(inner: T, need: NeedToKnow) -> Self {
        Self {
            inner,
            need,
            mandate: None,
        }
    }

    /// Attach a mandate / pre-trade boundary check.
    pub fn with_mandate(mut self, mandate: Arc<dyn Mandate>) -> Self {
        self.mandate = Some(mandate);
        self
    }
}

#[async_trait]
impl<T: Tool> Tool for WalledTool<T> {
    fn descriptor(&self) -> &ToolDescriptor {
        self.inner.descriptor()
    }

    async fn invoke(&self, args: Value, ctx: &InvokeCtx) -> Result<Value> {
        // Fail-closed clearance check.
        let clearance = ctx.ext::<ClearanceContext>().ok_or_else(|| {
            SecurityError::AccessDenied(format!(
                "tool '{}' requires a clearance context, none attached",
                self.inner.descriptor().name
            ))
        })?;

        if !clearance.permits(&self.need) {
            return Err(SecurityError::AccessDenied(format!(
                "subject '{}' is not cleared for tool '{}'",
                clearance.subject,
                self.inner.descriptor().name
            ))
            .into());
        }

        // Mandate / pre-trade boundary.
        if let Some(mandate) = &self.mandate {
            mandate.check(&args, ctx).await?;
        }

        self.inner.invoke(args, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clearance::ClearanceLevel;
    use crate::mandate::DenyAll;
    use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, ToolId};
    use atomr_agents_tool::ToolSchema;

    struct EchoTool {
        desc: ToolDescriptor,
    }
    impl EchoTool {
        fn new() -> Self {
            Self {
                desc: ToolDescriptor {
                    id: ToolId::from("echo"),
                    name: "echo".into(),
                    description: "echoes".into(),
                    schema: ToolSchema::empty_object(),
                },
            }
        }
    }
    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.desc
        }
        async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> Result<Value> {
            Ok(args)
        }
    }

    fn invoke_ctx(clearance: Option<ClearanceContext>) -> InvokeCtx {
        let mut call = CallCtx::new(
            None,
            TokenBudget::new(100),
            TimeBudget::new(std::time::Duration::from_secs(1)),
            MoneyBudget::from_usd(1.0),
            IterationBudget::new(1),
            vec![],
        );
        if let Some(c) = clearance {
            call.insert_ext(c);
        }
        InvokeCtx {
            call,
            tool_call_id: "t1".into(),
            raw_args: Value::Null,
        }
    }

    #[tokio::test]
    async fn denies_without_clearance_context() {
        let tool = WalledTool::new(EchoTool::new(), NeedToKnow::level(ClearanceLevel::Internal));
        let res = tool.invoke(Value::Null, &invoke_ctx(None)).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn allows_with_sufficient_clearance() {
        let tool = WalledTool::new(
            EchoTool::new(),
            NeedToKnow::level(ClearanceLevel::Internal).with_compartment("desk:credit"),
        );
        let clearance =
            ClearanceContext::new("alice", ClearanceLevel::Restricted).with_compartment("desk:credit");
        let res = tool
            .invoke(serde_json::json!({"x": 1}), &invoke_ctx(Some(clearance)))
            .await
            .unwrap();
        assert_eq!(res, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn mandate_can_block_even_when_cleared() {
        let tool = WalledTool::new(EchoTool::new(), NeedToKnow::default())
            .with_mandate(Arc::new(DenyAll("restricted list")));
        let clearance = ClearanceContext::new("alice", ClearanceLevel::Mnpi);
        let res = tool.invoke(Value::Null, &invoke_ctx(Some(clearance))).await;
        assert!(res.is_err());
    }
}
