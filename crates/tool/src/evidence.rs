//! Required evidence-trace on tool returns (FR-22).
//!
//! Autonomous trading requires every order to be explainable to its
//! evidence. An [`EvidenceTrace`] ties a tool's decision to the inputs
//! that justified it ([`EvidenceRef`]s — doc hashes, checkpoint pointers,
//! measure ids) plus a rationale. The trace travels in the
//! [`ToolReturn::ContentAndArtifact`] artifact channel under a reserved
//! key, so it auto-persists into the recording layer's `ToolCallRecord`
//! and surfaces via telemetry — no enum change needed.
//!
//! [`ExplainedTool`] is reusable middleware: with
//! [`ExplainabilityPolicy::Require`], a money-moving tool whose return
//! lacks a trace is rejected with a typed [`MissingEvidence`].

use async_trait::async_trait;
use atomr_agents_core::{AgentError, InvokeCtx, Result, Value};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::descriptor::ToolDescriptor;
use crate::tool_return::{RichTool, ToolReturn};

/// Reserved artifact key under which the evidence trace is stored.
pub const EVIDENCE_KEY: &str = "__evidence_trace__";

/// A reference to a piece of evidence behind a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRef {
    /// Hash of a sourced document (resolvable in the retrieval store).
    DocHash(String),
    /// Pointer to a checkpoint `(workflow:run:step)`.
    CheckpointRef(String),
    /// A computed measure / metric id.
    MeasureId(String),
}

/// The evidence + rationale behind a tool's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceTrace {
    pub inputs: Vec<EvidenceRef>,
    pub rationale: String,
}

impl EvidenceTrace {
    pub fn new(rationale: impl Into<String>, inputs: Vec<EvidenceRef>) -> Self {
        Self {
            inputs,
            rationale: rationale.into(),
        }
    }

    /// Attach this trace to a [`ToolReturn`], moving plain `Content` into
    /// a `ContentAndArtifact` whose artifact carries the trace. `Command`
    /// returns are passed through unchanged (no content to annotate).
    pub fn attach(self, ret: ToolReturn) -> ToolReturn {
        let trace_val = serde_json::to_value(&self).unwrap_or(Value::Null);
        match ret {
            ToolReturn::Content(content) => ToolReturn::ContentAndArtifact {
                content,
                artifact: serde_json::json!({ EVIDENCE_KEY: trace_val }),
            },
            ToolReturn::ContentAndArtifact { content, artifact } => {
                let mut obj = match artifact {
                    Value::Object(m) => m,
                    other => {
                        let mut m = serde_json::Map::new();
                        if !other.is_null() {
                            m.insert("artifact".into(), other);
                        }
                        m
                    }
                };
                obj.insert(EVIDENCE_KEY.into(), trace_val);
                ToolReturn::ContentAndArtifact {
                    content,
                    artifact: Value::Object(obj),
                }
            }
            cmd @ ToolReturn::Command(_) => cmd,
        }
    }

    /// Extract a trace from a [`ToolReturn`], if present.
    pub fn extract(ret: &ToolReturn) -> Option<EvidenceTrace> {
        if let ToolReturn::ContentAndArtifact { artifact, .. } = ret {
            artifact
                .get(EVIDENCE_KEY)
                .and_then(|v| serde_json::from_value::<EvidenceTrace>(v.clone()).ok())
        } else {
            None
        }
    }
}

/// Per-tool/desk explainability requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainabilityPolicy {
    /// Reject a return lacking an evidence trace.
    Require,
    /// Allow, but the absence is observable to the caller.
    Warn,
    /// No requirement.
    Off,
}

/// Raised when [`ExplainabilityPolicy::Require`] is violated.
#[derive(Debug, Error)]
#[error("missing evidence trace for tool '{tool}' (explainability=Require)")]
pub struct MissingEvidence {
    pub tool: String,
}

impl From<MissingEvidence> for AgentError {
    fn from(e: MissingEvidence) -> Self {
        AgentError::PolicyDenied(e.to_string())
    }
}

impl ExplainabilityPolicy {
    /// Enforce the policy against a return. Returns `Ok(was_present)` for
    /// `Warn`/`Off`; `Err(MissingEvidence)` for `Require` with no trace.
    pub fn enforce(&self, tool: &str, ret: &ToolReturn) -> Result<bool, MissingEvidence> {
        let present = EvidenceTrace::extract(ret).is_some();
        match self {
            ExplainabilityPolicy::Require if !present => Err(MissingEvidence { tool: tool.into() }),
            _ => Ok(present),
        }
    }
}

/// Middleware enforcing an [`ExplainabilityPolicy`] on any [`RichTool`].
/// Wraps without touching the inner tool's constructor (cf. `WalledTool`).
pub struct ExplainedTool<T: RichTool> {
    inner: T,
    policy: ExplainabilityPolicy,
}

impl<T: RichTool> ExplainedTool<T> {
    pub fn new(inner: T, policy: ExplainabilityPolicy) -> Self {
        Self { inner, policy }
    }
}

#[async_trait]
impl<T: RichTool> RichTool for ExplainedTool<T> {
    fn descriptor(&self) -> &ToolDescriptor {
        self.inner.descriptor()
    }

    async fn invoke_rich(&self, args: Value, ctx: &InvokeCtx) -> Result<ToolReturn> {
        let ret = self.inner.invoke_rich(args, ctx).await?;
        self.policy.enforce(&self.inner.descriptor().name, &ret)?;
        Ok(ret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{ToolDescriptor, ToolSchema};
    use atomr_agents_core::{CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget, ToolId};
    use std::time::Duration;

    fn ictx() -> InvokeCtx {
        InvokeCtx {
            call: CallCtx::new(
                None,
                TokenBudget::new(100),
                TimeBudget::new(Duration::from_secs(1)),
                MoneyBudget::from_usd(1.0),
                IterationBudget::new(1),
                vec![],
            ),
            tool_call_id: "t".into(),
            raw_args: Value::Null,
        }
    }

    fn desc(name: &str) -> ToolDescriptor {
        ToolDescriptor {
            id: ToolId::from(name),
            name: name.into(),
            description: "".into(),
            schema: ToolSchema::empty_object(),
        }
    }

    struct OrderTool {
        d: ToolDescriptor,
        attach_evidence: bool,
    }
    #[async_trait]
    impl RichTool for OrderTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.d
        }
        async fn invoke_rich(&self, _args: Value, _ctx: &InvokeCtx) -> Result<ToolReturn> {
            let base = ToolReturn::Content(serde_json::json!({"order_id": "o1"}));
            if self.attach_evidence {
                Ok(EvidenceTrace::new(
                    "carry signal above threshold",
                    vec![EvidenceRef::DocHash("abc123".into())],
                )
                .attach(base))
            } else {
                Ok(base)
            }
        }
    }

    #[test]
    fn attach_then_extract_roundtrip() {
        let ret = EvidenceTrace::new("why", vec![EvidenceRef::MeasureId("m1".into())])
            .attach(ToolReturn::Content(serde_json::json!({"x": 1})));
        let got = EvidenceTrace::extract(&ret).unwrap();
        assert_eq!(got.rationale, "why");
        assert_eq!(got.inputs, vec![EvidenceRef::MeasureId("m1".into())]);
        // content preserved
        if let ToolReturn::ContentAndArtifact { content, .. } = ret {
            assert_eq!(content, serde_json::json!({"x": 1}));
        } else {
            panic!("expected ContentAndArtifact");
        }
    }

    #[tokio::test]
    async fn require_rejects_money_tool_without_evidence() {
        let tool = ExplainedTool::new(
            OrderTool {
                d: desc("place_order"),
                attach_evidence: false,
            },
            ExplainabilityPolicy::Require,
        );
        let err = tool.invoke_rich(Value::Null, &ictx()).await.unwrap_err();
        assert!(err.to_string().contains("missing evidence"));
    }

    #[tokio::test]
    async fn require_allows_when_evidence_present() {
        let tool = ExplainedTool::new(
            OrderTool {
                d: desc("place_order"),
                attach_evidence: true,
            },
            ExplainabilityPolicy::Require,
        );
        let ret = tool.invoke_rich(Value::Null, &ictx()).await.unwrap();
        assert!(EvidenceTrace::extract(&ret).is_some());
    }
}
