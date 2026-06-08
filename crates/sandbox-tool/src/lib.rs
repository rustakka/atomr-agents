//! `SandboxTool` — adapts the [`SandboxHarness`] into an
//! [`atomr_agents_tool::Tool`] so agents call `execute_in_sandbox` uniformly.
//!
//! Pattern copied from `web-search-tool`: hold the dependency plus a cached
//! [`ToolDescriptor`], parse args into a typed struct, delegate, and shape the
//! result back to JSON. On top of that it resolves the toolchain profile,
//! lets the harness apply the Rust-floor budget, and emits the generic
//! `Event::ToolInvoked` on the harness `EventBus` for tracer integration.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Event, InvokeCtx, Result as CoreResult, ToolId, Value};
use atomr_agents_sandbox_core::{ExecRequest, Language, SandboxBackendSel, SandboxProfile};
use atomr_agents_sandbox_harness::SandboxHarness;
use atomr_agents_tool::{Tool, ToolDescriptor, ToolSchema};
use serde::Deserialize;
use serde_json::json;

/// Tool name / id stem.
pub const TOOL_NAME: &str = "execute_in_sandbox";

/// Typed view of the LLM-supplied arguments.
#[derive(Debug, Deserialize)]
struct SandboxToolArgs {
    language: Language,
    code: String,
    #[serde(default)]
    dependencies: Vec<String>,
    /// Explicit toolchain profile; inferred from `language` when absent.
    #[serde(default)]
    profile: Option<SandboxProfile>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// `Tool` impl backed by a [`SandboxHarness`].
pub struct SandboxTool {
    harness: Arc<SandboxHarness>,
    descriptor: ToolDescriptor,
}

impl SandboxTool {
    pub fn new(harness: Arc<SandboxHarness>) -> Self {
        let descriptor = ToolDescriptor {
            id: ToolId::from(format!("sandbox.{TOOL_NAME}")),
            name: TOOL_NAME.into(),
            description: "Execute code in an ephemeral sandbox (Python / Bash / JavaScript / \
                          Rust). With a Docker or Firecracker backend this is a hardware- or \
                          container-isolated sandbox suitable for untrusted code; the default \
                          in-memory mock backend performs NO isolation and is for testing only. \
                          Optionally install dependencies first and choose a toolchain profile. \
                          Rust profiles are provisioned with at least 2 GB RAM / 2 vCPUs. \
                          Returns { exec_id, exit_code, success, stdout, stderr, timed_out }."
                .into(),
            schema: ToolSchema(json!({
                "type": "object",
                "required": ["language", "code"],
                "properties": {
                    "language":     { "type": "string", "enum": ["python", "bash", "js", "rust"] },
                    "code":         { "type": "string", "minLength": 1 },
                    "dependencies": { "type": "array", "items": { "type": "string" } },
                    "profile":      { "type": "string",
                                      "enum": ["python_only", "npm_only", "rust_only",
                                               "python_and_npm", "full_stack"] },
                    "stdin":        { "type": "string" },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 600 }
                }
            })),
        };
        Self { harness, descriptor }
    }

    pub fn harness(&self) -> &Arc<SandboxHarness> {
        &self.harness
    }
}

fn hash_args(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

#[async_trait]
impl Tool for SandboxTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn invoke(&self, args: Value, _ctx: &InvokeCtx) -> CoreResult<Value> {
        let args_hash = hash_args(&args);
        let parsed: SandboxToolArgs = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("{TOOL_NAME}: invalid args: {e}")))?;

        let exec = ExecRequest {
            language: parsed.language,
            code: parsed.code,
            dependencies: parsed.dependencies,
            stdin: parsed.stdin,
            env: Default::default(),
            timeout_secs: parsed.timeout_secs,
        };

        let started = Instant::now();
        // The harness resolves the profile (Rust → RustOnly etc.) and normalizes
        // the budget so the Rust 2 GB / 2 vCPU floor is enforced before dispatch.
        // Backend selection is a deployment concern (how the harness is built),
        // not an LLM-controllable argument, so the tool always requests `Auto`.
        let result = self
            .harness
            .run_exec(exec, parsed.profile, SandboxBackendSel::Auto)
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        // Generic telemetry on the process-local EventBus — integrates with
        // tracers without touching `core`. The harness separately broadcasts
        // richer SandboxEvents.
        self.harness.bus.emit(Event::ToolInvoked {
            tool_id: self.descriptor.id.clone(),
            args_hash,
            elapsed_ms,
            ok: result.is_ok(),
        });

        let res = result.map_err(|e| AgentError::Tool(format!("{TOOL_NAME}: {e}")))?;
        Ok(res.to_tool_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_core::{
        CallCtx, IterationBudget, MoneyBudget, TimeBudget, TokenBudget,
    };
    use atomr_agents_sandbox_core::SandboxEvent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn ctx() -> InvokeCtx {
        InvokeCtx {
            call: CallCtx {
                agent_id: None,
                tokens: TokenBudget::new(1000),
                time: TimeBudget::new(Duration::from_secs(5)),
                money: MoneyBudget::from_usd(1.0),
                iterations: IterationBudget::new(5),
                trace: vec![],
                extensions: Default::default(),
            },
            tool_call_id: "test-1".into(),
            raw_args: Value::Null,
        }
    }

    fn tool() -> SandboxTool {
        SandboxTool::new(Arc::new(SandboxHarness::local_default()))
    }

    #[tokio::test]
    async fn runs_code_and_shapes_result() {
        let t = tool();
        let out = t
            .invoke(json!({ "language": "python", "code": "print('hi')" }), &ctx())
            .await
            .unwrap();
        assert_eq!(out.get("success").and_then(|v| v.as_bool()), Some(true));
        assert!(out.get("stdout").unwrap().as_str().unwrap().contains("[mock:Python]"));
    }

    #[tokio::test]
    async fn rejects_missing_required_args() {
        let t = tool();
        let err = t.invoke(json!({ "language": "python" }), &ctx()).await.unwrap_err();
        match err {
            AgentError::Tool(m) => assert!(m.contains("invalid args"), "got: {m}"),
            other => panic!("expected Tool err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_unknown_language() {
        let t = tool();
        let err = t
            .invoke(json!({ "language": "cobol", "code": "x" }), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::Tool(_)));
    }

    #[tokio::test]
    async fn infers_rust_profile_from_language() {
        // No explicit profile: language=rust must resolve to a Rust-bearing
        // profile (RustOnly), observable via the Created lifecycle event.
        let harness = Arc::new(SandboxHarness::local_default());
        let mut events = harness.events();
        let t = SandboxTool::new(harness.clone());
        t.invoke(json!({ "language": "rust", "code": "fn main(){}" }), &ctx())
            .await
            .unwrap();
        let created = tokio::time::timeout(Duration::from_millis(200), events.recv())
            .await
            .unwrap()
            .unwrap();
        match created {
            SandboxEvent::Created { profile, .. } => {
                assert_eq!(profile, SandboxProfile::RustOnly)
            }
            other => panic!("expected Created, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emits_tool_invoked_telemetry() {
        let harness = Arc::new(SandboxHarness::local_default());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        harness.bus.subscribe(move |_env| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let t = SandboxTool::new(harness);
        t.invoke(json!({ "language": "bash", "code": "echo hi" }), &ctx())
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
