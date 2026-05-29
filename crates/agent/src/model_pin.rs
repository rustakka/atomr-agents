//! Model/provider version pinning + drift detection (FR-3).
//!
//! A model change is a gate-governed promotion event: a silently
//! upgraded model changes live trading behavior un-gated and breaks the
//! "which model produced this trade?" audit chain. [`PinnedClient`] is an
//! [`InferenceClient`] middleware that, at call time, resolves the
//! concrete model + version, compares it to a declared [`ModelPin`], and:
//!
//! * refuses to run (typed [`ModelPinViolation`]) when `strict_pin` is on
//!   and no pin is set;
//! * emits a [`RunEventKind::ModelDrift`] onto the telemetry backbone
//!   (FR-19) whenever the resolved model differs from the pin;
//! * otherwise delegates to the inner client unchanged.
//!
//! The resolved pin is also what the recording path stamps into every
//! [`InferenceRecord`](atomr_agents_state::InferenceRecord).

use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
use atomr_agents_observability::{ModelPinRef, RunEvent, RunEventKind, Telemetry};
use atomr_agents_tool::Provider;
use atomr_infer_core::batch::ExecuteBatch;
use thiserror::Error;

use crate::inference::{InferenceClient, TurnResult};

/// A first-class pin on `(provider, model, version, params)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPin {
    pub provider: Provider,
    pub model_id: String,
    /// Semantic version or content digest of the model.
    pub model_version: String,
    /// Hash of the sampling/params that affect output.
    pub params_hash: String,
}

impl ModelPin {
    pub fn new(
        provider: Provider,
        model_id: impl Into<String>,
        model_version: impl Into<String>,
        params_hash: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            model_id: model_id.into(),
            model_version: model_version.into(),
            params_hash: params_hash.into(),
        }
    }

    fn as_ref(&self) -> ModelPinRef {
        ModelPinRef {
            provider: provider_str(self.provider).into(),
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            params_hash: self.params_hash.clone(),
        }
    }
}

/// What a provider actually resolves to at call time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub provider: Provider,
    pub model_id: String,
    pub model_version: String,
    pub params_hash: String,
}

/// Kind of drift detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    VersionDrift,
    ParamsDrift,
}

impl DriftKind {
    fn as_str(self) -> &'static str {
        match self {
            DriftKind::VersionDrift => "version_drift",
            DriftKind::ParamsDrift => "params_drift",
        }
    }
}

/// Resolves the concrete model/version a client would use right now.
/// Hosts implement this (often by querying the provider) so a deployed
/// pin can be re-validated on a heartbeat to catch provider-side silent
/// upgrades.
pub trait ModelResolver: Send + Sync + 'static {
    fn resolve_version(&self) -> ResolvedModel;
}

/// A static resolver (tests / known-version deployments).
pub struct StaticResolver(pub ResolvedModel);

impl ModelResolver for StaticResolver {
    fn resolve_version(&self) -> ResolvedModel {
        self.0.clone()
    }
}

/// Pin violation: strict mode with no pin, or (optionally) a hard
/// refusal on drift.
#[derive(Debug, Error)]
pub enum ModelPinViolation {
    #[error("strict_pin: a run requires a ModelPin but none was set")]
    NoPinInStrictMode,
    #[error("model pin mismatch ({kind:?}): expected {expected:?}, got {actual:?}")]
    Mismatch {
        kind: DriftKind,
        expected: ModelPin,
        actual: ResolvedModel,
    },
}

impl From<ModelPinViolation> for atomr_agents_core::AgentError {
    fn from(e: ModelPinViolation) -> Self {
        atomr_agents_core::AgentError::PolicyDenied(e.to_string())
    }
}

/// Compare a resolved model to a pin; `None` if they match.
pub fn detect_drift(pin: &ModelPin, actual: &ResolvedModel) -> Option<DriftKind> {
    if pin.provider != actual.provider
        || pin.model_id != actual.model_id
        || pin.model_version != actual.model_version
    {
        Some(DriftKind::VersionDrift)
    } else if pin.params_hash != actual.params_hash {
        Some(DriftKind::ParamsDrift)
    } else {
        None
    }
}

/// `InferenceClient` middleware enforcing a [`ModelPin`].
pub struct PinnedClient {
    inner: Arc<dyn InferenceClient>,
    resolver: Arc<dyn ModelResolver>,
    pin: Option<ModelPin>,
    strict: bool,
    /// If true, drift is a hard refusal (not just an emitted event).
    refuse_on_drift: bool,
    telemetry: Option<Telemetry>,
    run_id: Option<String>,
}

impl PinnedClient {
    pub fn new(inner: Arc<dyn InferenceClient>, resolver: Arc<dyn ModelResolver>) -> Self {
        Self {
            inner,
            resolver,
            pin: None,
            strict: false,
            refuse_on_drift: false,
            telemetry: None,
            run_id: None,
        }
    }

    /// Pin the model (see [`crate::Agent`] wiring as `pin_model`).
    pub fn pin_model(mut self, pin: ModelPin) -> Self {
        self.pin = Some(pin);
        self
    }

    /// Require a pin to be set, else refuse to run.
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Make drift a hard refusal in addition to an emitted event.
    pub fn refuse_on_drift(mut self, refuse: bool) -> Self {
        self.refuse_on_drift = refuse;
        self
    }

    pub fn with_telemetry(mut self, telemetry: Telemetry, run_id: impl Into<String>) -> Self {
        self.telemetry = Some(telemetry);
        self.run_id = Some(run_id.into());
        self
    }

    /// The pin this client carries, if any (for stamping into records).
    pub fn pin(&self) -> Option<&ModelPin> {
        self.pin.as_ref()
    }

    /// Re-validate the pin against a freshly resolved version (heartbeat).
    /// Returns the drift kind if the provider has drifted.
    pub fn revalidate(&self) -> Option<DriftKind> {
        let actual = self.resolver.resolve_version();
        self.pin.as_ref().and_then(|p| detect_drift(p, &actual))
    }

    // The error variant intentionally carries the full pin + resolved
    // model for audit; size is not a hot-path concern here.
    #[allow(clippy::result_large_err)]
    fn enforce(&self) -> std::result::Result<(), ModelPinViolation> {
        let Some(pin) = self.pin.as_ref() else {
            if self.strict {
                return Err(ModelPinViolation::NoPinInStrictMode);
            }
            return Ok(());
        };
        let actual = self.resolver.resolve_version();
        if let Some(kind) = detect_drift(pin, &actual) {
            if let (Some(t), Some(run)) = (&self.telemetry, &self.run_id) {
                t.emit(
                    RunEvent::new(RunEventKind::ModelDrift {
                        expected: format!("{:?}", pin.as_ref()),
                        actual: format!(
                            "{:?}",
                            ModelPinRef {
                                provider: provider_str(actual.provider).into(),
                                model_id: actual.model_id.clone(),
                                model_version: actual.model_version.clone(),
                                params_hash: actual.params_hash.clone(),
                            }
                        ),
                        drift: kind.as_str().into(),
                    })
                    .with_model_pin(pin.as_ref()),
                );
                let _ = run; // run id retained for callers that set it
            }
            if self.refuse_on_drift {
                return Err(ModelPinViolation::Mismatch {
                    kind,
                    expected: pin.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }
}

#[async_trait]
impl InferenceClient for PinnedClient {
    fn provider(&self) -> Provider {
        self.inner.provider()
    }

    async fn run(&self, batch: ExecuteBatch) -> Result<TurnResult> {
        self.enforce()?;
        self.inner.run(batch).await
    }
}

fn provider_str(p: Provider) -> &'static str {
    match p {
        Provider::OpenAi => "open_ai",
        Provider::Anthropic => "anthropic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_observability::InMemoryTelemetrySink;
    use atomr_infer_core::tokens::TokenUsage;

    struct FixedClient(Provider);
    #[async_trait]
    impl InferenceClient for FixedClient {
        fn provider(&self) -> Provider {
            self.0
        }
        async fn run(&self, _batch: ExecuteBatch) -> Result<TurnResult> {
            Ok(TurnResult {
                text: "ok".into(),
                usage: TokenUsage::default(),
                finish_reason: None,
                tool_calls: vec![],
            })
        }
    }

    fn batch() -> ExecuteBatch {
        ExecuteBatch {
            request_id: "r".into(),
            model: "m".into(),
            messages: vec![],
            sampling: Default::default(),
            stream: false,
            estimated_tokens: 0,
        }
    }

    fn resolved(version: &str, params: &str) -> ResolvedModel {
        ResolvedModel {
            provider: Provider::Anthropic,
            model_id: "claude".into(),
            model_version: version.into(),
            params_hash: params.into(),
        }
    }

    #[tokio::test]
    async fn strict_mode_without_pin_refuses() {
        let c = PinnedClient::new(
            Arc::new(FixedClient(Provider::Anthropic)),
            Arc::new(StaticResolver(resolved("1", "p"))),
        )
        .strict(true);
        assert!(c.run(batch()).await.is_err());
    }

    #[tokio::test]
    async fn matching_pin_passes_through() {
        let c = PinnedClient::new(
            Arc::new(FixedClient(Provider::Anthropic)),
            Arc::new(StaticResolver(resolved("1", "p"))),
        )
        .pin_model(ModelPin::new(Provider::Anthropic, "claude", "1", "p"))
        .strict(true);
        let r = c.run(batch()).await.unwrap();
        assert_eq!(r.text, "ok");
        assert!(c.revalidate().is_none());
    }

    #[tokio::test]
    async fn version_drift_emits_event_and_can_refuse() {
        let sink = Arc::new(InMemoryTelemetrySink::new());
        let tel = Telemetry::new().with_sink(sink.clone());
        // pin says version "1", provider resolves "2" -> VersionDrift
        let c = PinnedClient::new(
            Arc::new(FixedClient(Provider::Anthropic)),
            Arc::new(StaticResolver(resolved("2", "p"))),
        )
        .pin_model(ModelPin::new(Provider::Anthropic, "claude", "1", "p"))
        .with_telemetry(tel, "run-1")
        .refuse_on_drift(true);

        let err = c.run(batch()).await.unwrap_err();
        assert!(err.to_string().contains("mismatch"));
        let events = sink.events();
        assert_eq!(events.len(), 1);
        match &events[0].kind {
            RunEventKind::ModelDrift { drift, .. } => assert_eq!(drift, "version_drift"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(events[0].model_pin.is_some());
    }

    #[tokio::test]
    async fn params_drift_detected() {
        assert_eq!(
            detect_drift(
                &ModelPin::new(Provider::Anthropic, "claude", "1", "p1"),
                &resolved("1", "p2")
            ),
            Some(DriftKind::ParamsDrift)
        );
    }
}
