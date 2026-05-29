//! FR-15 — Schema / data-contract validation operator with HITL-interrupt
//! binding in pipelines.
//!
//! Data-quality gating (type drift, bar gaps, late / out-of-order data) is
//! core to trustworthy ingestion feeding money-moving decisions. A contract
//! breach must be able to raise a **durable** HITL interrupt from inside a
//! pipeline stage so a human reviews suspect data before it propagates onto
//! real capital.
//!
//! A [`DataContract`] validates a record's shape ([`TypeSchema`]),
//! freshness ([`MaxLag`] against a `ts_ms` field), and continuity
//! ([`ExpectedCadence`] against the gap from a previous timestamp), plus
//! arbitrary [`custom`](DataContract::custom) checks. On breach,
//! [`ContractValidateStage`] applies an [`OnBreach`] policy: `Drop`,
//! `RouteTo` a sink, or `Interrupt` — which raises a durable interrupt via
//! the FR-4 [`InterruptRegistry`] carrying the [`ContractViolation`] as
//! payload, parking the branch until a human resolves it.

use std::sync::Arc;

use atomr_agents_core::Result;
use serde::{Deserialize, Serialize};

use crate::interrupt_registry::{
    InterruptIdGen, InterruptRegistry, InterruptStatus, PendingInterrupt,
};

/// Minimal JSON shape check: each required key must be present and carry a
/// value of the declared [`JsonType`].
#[derive(Debug, Clone, Default)]
pub struct TypeSchema {
    /// `(key, expected JSON type)` pairs that must all hold.
    pub required: Vec<(String, JsonType)>,
}

impl TypeSchema {
    /// Build an empty schema.
    pub fn new() -> Self {
        Self::default()
    }
    /// Require `key` to be present with JSON type `ty`.
    pub fn require(mut self, key: impl Into<String>, ty: JsonType) -> Self {
        self.required.push((key.into(), ty));
        self
    }
}

/// The JSON value-kinds a [`TypeSchema`] can assert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    /// A JSON string.
    String,
    /// A JSON number (integer or float).
    Number,
    /// A JSON boolean.
    Bool,
    /// A JSON array.
    Array,
    /// A JSON object.
    Object,
}

impl JsonType {
    fn matches(&self, v: &serde_json::Value) -> bool {
        matches!(
            (self, v),
            (JsonType::String, serde_json::Value::String(_))
                | (JsonType::Number, serde_json::Value::Number(_))
                | (JsonType::Bool, serde_json::Value::Bool(_))
                | (JsonType::Array, serde_json::Value::Array(_))
                | (JsonType::Object, serde_json::Value::Object(_))
        )
    }
    fn name(&self) -> &'static str {
        match self {
            JsonType::String => "string",
            JsonType::Number => "number",
            JsonType::Bool => "bool",
            JsonType::Array => "array",
            JsonType::Object => "object",
        }
    }
}

/// Maximum tolerated staleness in ms, validated against a record's `ts_ms`
/// field versus `now_ms`.
#[derive(Debug, Clone, Copy)]
pub struct MaxLag(pub i64);

/// Expected inter-record cadence in ms; a gap larger than this from the
/// previous timestamp is a continuity breach (a missing bar).
#[derive(Debug, Clone, Copy)]
pub struct ExpectedCadence(pub i64);

/// A custom check: returns `Some(detail)` to signal a breach.
pub type CustomCheck = Box<dyn Fn(&serde_json::Value) -> Option<String> + Send + Sync>;

/// The full data contract for a record/batch.
pub struct DataContract {
    /// Required shape.
    pub schema: TypeSchema,
    /// Maximum staleness (validated against the `ts_ms` field).
    pub freshness: Option<MaxLag>,
    /// Expected cadence (validated against a provided previous ts).
    pub continuity: Option<ExpectedCadence>,
    /// Arbitrary additional checks.
    pub custom: Vec<CustomCheck>,
}

impl Default for DataContract {
    fn default() -> Self {
        Self {
            schema: TypeSchema::new(),
            freshness: None,
            continuity: None,
            custom: Vec::new(),
        }
    }
}

/// The kind of contract breach detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationKind {
    /// A required key is missing or has the wrong JSON type.
    TypeDrift,
    /// The record is older than the allowed [`MaxLag`].
    StaleData,
    /// The gap from the previous record exceeds the [`ExpectedCadence`].
    Gap,
    /// A custom check failed.
    Custom,
}

/// A single detected breach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractViolation {
    /// The category of breach.
    pub kind: ViolationKind,
    /// Human-readable detail (carried into the interrupt payload).
    pub detail: String,
}

impl DataContract {
    /// Validate `record`. `now_ms` is the wall clock for freshness;
    /// `prev_ts` is the previous record's `ts_ms` for cadence (None = first
    /// record, cadence not checked). Returns the first breach found.
    pub fn validate(
        &self,
        record: &serde_json::Value,
        now_ms: i64,
        prev_ts: Option<i64>,
    ) -> std::result::Result<(), ContractViolation> {
        // 1. Type schema.
        for (key, ty) in &self.schema.required {
            match record.get(key) {
                None => {
                    return Err(ContractViolation {
                        kind: ViolationKind::TypeDrift,
                        detail: format!("missing required key `{key}`"),
                    })
                }
                Some(v) if !ty.matches(v) => {
                    return Err(ContractViolation {
                        kind: ViolationKind::TypeDrift,
                        detail: format!("key `{key}` expected {} ", ty.name()),
                    })
                }
                Some(_) => {}
            }
        }
        // 2. Freshness against ts_ms.
        if let Some(MaxLag(max)) = self.freshness {
            let ts = record.get("ts_ms").and_then(|v| v.as_i64()).ok_or_else(|| ContractViolation {
                kind: ViolationKind::StaleData,
                detail: "missing or non-integer `ts_ms` for freshness check".into(),
            })?;
            let lag = now_ms - ts;
            if lag > max {
                return Err(ContractViolation {
                    kind: ViolationKind::StaleData,
                    detail: format!("data is {lag}ms old, exceeds MaxLag({max}ms)"),
                });
            }
        }
        // 3. Continuity against previous ts.
        if let (Some(ExpectedCadence(cadence)), Some(prev)) = (self.continuity, prev_ts) {
            let ts = record.get("ts_ms").and_then(|v| v.as_i64()).ok_or_else(|| ContractViolation {
                kind: ViolationKind::Gap,
                detail: "missing or non-integer `ts_ms` for continuity check".into(),
            })?;
            let gap = ts - prev;
            if gap > cadence {
                return Err(ContractViolation {
                    kind: ViolationKind::Gap,
                    detail: format!("gap of {gap}ms exceeds ExpectedCadence({cadence}ms)"),
                });
            }
        }
        // 4. Custom checks.
        for check in &self.custom {
            if let Some(detail) = check(record) {
                return Err(ContractViolation {
                    kind: ViolationKind::Custom,
                    detail,
                });
            }
        }
        Ok(())
    }
}

/// Where to address the durable interrupt raised on breach.
#[derive(Debug, Clone, Default)]
pub struct InterruptSpec {
    /// Workflow the parked branch belongs to.
    pub workflow: String,
    /// Run the parked branch belongs to.
    pub run: String,
    /// Super-step at which the branch parks.
    pub step: u64,
    /// Role the approval is addressed to.
    pub requested_role: Option<String>,
    /// Clearance required to resolve it.
    pub requested_clearance: Option<String>,
    /// Optional SLA deadline (epoch ms).
    pub deadline_ms: Option<i64>,
}

/// A sink for records routed away on breach (the `RouteTo` policy).
#[async_trait::async_trait]
pub trait BreachSink: Send + Sync {
    /// Receive a record + its violation for out-of-band handling.
    async fn route(&self, record: serde_json::Value, violation: ContractViolation) -> Result<()>;
}

/// Policy applied when a record breaches its contract.
#[derive(Clone)]
pub enum OnBreach {
    /// Discard the record.
    Drop,
    /// Route the record + violation to a sink.
    RouteTo(Arc<dyn BreachSink>),
    /// Raise a durable HITL interrupt carrying the violation payload.
    Interrupt(InterruptSpec),
}

impl std::fmt::Debug for OnBreach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnBreach::Drop => write!(f, "Drop"),
            OnBreach::RouteTo(_) => write!(f, "RouteTo(..)"),
            OnBreach::Interrupt(s) => f.debug_tuple("Interrupt").field(s).finish(),
        }
    }
}

/// Outcome of running a record through a [`ContractValidateStage`].
#[derive(Debug, Clone, PartialEq)]
pub enum StageOutcome {
    /// The record passed; flows through unchanged.
    Pass(serde_json::Value),
    /// The record breached and was dropped.
    Dropped,
    /// The record breached and was routed to a sink.
    Routed,
    /// The record breached and parked a durable interrupt (id returned).
    Interrupted(String),
}

/// A pipeline-style contract-validation stage. Holds the contract, the
/// breach policy, and (for the `Interrupt` policy) a registry + id
/// generator to mint durable interrupts.
pub struct ContractValidateStage {
    contract: DataContract,
    on_breach: OnBreach,
    registry: Option<Arc<dyn InterruptRegistry>>,
    id_gen: Arc<InterruptIdGen>,
}

impl ContractValidateStage {
    /// Build a stage. `registry` is required only for the `Interrupt`
    /// policy; pass it for `Drop`/`RouteTo` too if you intend to switch.
    pub fn new(
        contract: DataContract,
        on_breach: OnBreach,
        registry: Option<Arc<dyn InterruptRegistry>>,
    ) -> Self {
        Self {
            contract,
            on_breach,
            registry,
            id_gen: Arc::new(InterruptIdGen::new()),
        }
    }

    /// Validate `record` and apply the breach policy. `now_ms`/`prev_ts`
    /// feed freshness/continuity checks. Valid records return
    /// [`StageOutcome::Pass`] with the record unchanged.
    pub async fn run(
        &self,
        record: serde_json::Value,
        now_ms: i64,
        prev_ts: Option<i64>,
    ) -> Result<StageOutcome> {
        match self.contract.validate(&record, now_ms, prev_ts) {
            Ok(()) => Ok(StageOutcome::Pass(record)),
            Err(violation) => match &self.on_breach {
                OnBreach::Drop => Ok(StageOutcome::Dropped),
                OnBreach::RouteTo(sink) => {
                    sink.route(record, violation).await?;
                    Ok(StageOutcome::Routed)
                }
                OnBreach::Interrupt(spec) => {
                    let registry = self.registry.as_ref().ok_or_else(|| {
                        atomr_agents_core::AgentError::Workflow(
                            "ContractValidateStage: Interrupt policy needs an InterruptRegistry".into(),
                        )
                    })?;
                    let id = self.id_gen.next_id();
                    let payload = serde_json::json!({
                        "violation": violation,
                        "record": record,
                    });
                    registry
                        .register(PendingInterrupt {
                            interrupt_id: id.clone(),
                            workflow: spec.workflow.clone(),
                            run: spec.run.clone(),
                            step: spec.step,
                            requested_role: spec.requested_role.clone(),
                            requested_clearance: spec.requested_clearance.clone(),
                            payload,
                            created_at_ms: now_ms,
                            deadline_ms: spec.deadline_ms,
                            assignee: None,
                            status: InterruptStatus::Pending,
                        })
                        .await?;
                    Ok(StageOutcome::Interrupted(id))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt_registry::InMemoryInterruptRegistry;
    use parking_lot::Mutex;
    use serde_json::json;

    fn bar_contract() -> DataContract {
        DataContract {
            schema: TypeSchema::new()
                .require("symbol", JsonType::String)
                .require("price", JsonType::Number)
                .require("ts_ms", JsonType::Number),
            freshness: Some(MaxLag(60_000)),
            continuity: Some(ExpectedCadence(1_000)),
            custom: vec![],
        }
    }

    #[test]
    fn detects_type_drift() {
        let c = bar_contract();
        // price is a string, not a number.
        let bad = json!({"symbol": "AAPL", "price": "100", "ts_ms": 1000});
        let v = c.validate(&bad, 1000, None).unwrap_err();
        assert_eq!(v.kind, ViolationKind::TypeDrift);

        let missing = json!({"symbol": "AAPL", "ts_ms": 1000});
        assert_eq!(c.validate(&missing, 1000, None).unwrap_err().kind, ViolationKind::TypeDrift);
    }

    #[test]
    fn detects_stale_data() {
        let c = bar_contract();
        let rec = json!({"symbol": "AAPL", "price": 100.0, "ts_ms": 0});
        // now is 120_000 → lag 120_000 > MaxLag 60_000.
        let v = c.validate(&rec, 120_000, None).unwrap_err();
        assert_eq!(v.kind, ViolationKind::StaleData);
    }

    #[test]
    fn detects_cadence_gap() {
        let c = bar_contract();
        let rec = json!({"symbol": "AAPL", "price": 100.0, "ts_ms": 10_000});
        // prev ts 5000 → gap 5000 > cadence 1000.
        let v = c.validate(&rec, 10_000, Some(5_000)).unwrap_err();
        assert_eq!(v.kind, ViolationKind::Gap);
    }

    #[test]
    fn custom_check_breaches() {
        let c = DataContract {
            schema: TypeSchema::new().require("price", JsonType::Number),
            custom: vec![Box::new(|v| {
                let p = v.get("price").and_then(|p| p.as_f64()).unwrap_or(0.0);
                (p <= 0.0).then(|| "price must be positive".to_string())
            })],
            ..Default::default()
        };
        let v = c.validate(&json!({"price": -1.0}), 0, None).unwrap_err();
        assert_eq!(v.kind, ViolationKind::Custom);
        assert_eq!(v.detail, "price must be positive");
    }

    #[tokio::test]
    async fn valid_record_passes_through() {
        let stage = ContractValidateStage::new(bar_contract(), OnBreach::Drop, None);
        let rec = json!({"symbol": "AAPL", "price": 100.0, "ts_ms": 1000});
        let out = stage.run(rec.clone(), 1000, None).await.unwrap();
        assert_eq!(out, StageOutcome::Pass(rec));
    }

    #[tokio::test]
    async fn drop_policy_drops_breach() {
        let stage = ContractValidateStage::new(bar_contract(), OnBreach::Drop, None);
        let bad = json!({"symbol": "AAPL", "ts_ms": 1000}); // missing price
        assert_eq!(stage.run(bad, 1000, None).await.unwrap(), StageOutcome::Dropped);
    }

    #[tokio::test]
    async fn route_policy_sends_to_sink() {
        #[derive(Default)]
        struct Collect {
            seen: Mutex<Vec<ContractViolation>>,
        }
        #[async_trait::async_trait]
        impl BreachSink for Collect {
            async fn route(&self, _r: serde_json::Value, v: ContractViolation) -> Result<()> {
                self.seen.lock().push(v);
                Ok(())
            }
        }
        let sink = Arc::new(Collect::default());
        let stage = ContractValidateStage::new(bar_contract(), OnBreach::RouteTo(sink.clone()), None);
        let bad = json!({"symbol": "AAPL", "ts_ms": 1000});
        assert_eq!(stage.run(bad, 1000, None).await.unwrap(), StageOutcome::Routed);
        assert_eq!(sink.seen.lock().len(), 1);
        assert_eq!(sink.seen.lock()[0].kind, ViolationKind::TypeDrift);
    }

    #[tokio::test]
    async fn interrupt_policy_raises_durable_interrupt_with_payload() {
        let registry: Arc<dyn InterruptRegistry> = Arc::new(InMemoryInterruptRegistry::new());
        let spec = InterruptSpec {
            workflow: "ingest".into(),
            run: "r1".into(),
            step: 7,
            requested_role: Some("data-steward".into()),
            requested_clearance: Some("L2".into()),
            deadline_ms: Some(99_999),
        };
        let stage = ContractValidateStage::new(
            bar_contract(),
            OnBreach::Interrupt(spec),
            Some(registry.clone()),
        );
        let stale = json!({"symbol": "AAPL", "price": 100.0, "ts_ms": 0});
        let out = stage.run(stale.clone(), 120_000, None).await.unwrap();
        let id = match out {
            StageOutcome::Interrupted(id) => id,
            other => panic!("expected Interrupted, got {other:?}"),
        };
        let parked = registry.get(&id).await.unwrap().unwrap();
        assert_eq!(parked.workflow, "ingest");
        assert_eq!(parked.step, 7);
        assert_eq!(parked.requested_role.as_deref(), Some("data-steward"));
        assert_eq!(parked.status, InterruptStatus::Pending);
        // Violation carried as payload.
        assert_eq!(parked.payload["violation"]["kind"], "StaleData");
        assert_eq!(parked.payload["record"], stale);
    }
}
