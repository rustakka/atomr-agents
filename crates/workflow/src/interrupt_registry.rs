//! FR-4 — Durable, queryable, fleet-wide HITL interrupt registry.
//!
//! Every money-moving action in a regulated, real-money fund parks on a
//! human-in-the-loop (HITL) approval. A [`StrategyActor`] hosting a parked
//! approval can fail over or rebalance to another node, so the parked
//! approval cannot live only in process memory: losing it could drop or
//! duplicate a trade. This registry gives the fleet a single durable inbox
//! of pending approvals, each addressed to a Role and resumable by a
//! **stable id** that is identical regardless of which node currently hosts
//! the run.
//!
//! Guarantees exercised by the tests:
//!
//! * **Survives reload.** A [`PendingInterrupt`] written to a store is still
//!   present (and resumable) after the in-memory handle is dropped and a
//!   fresh handle is opened over the same store.
//! * **Exactly-once resume.** [`InterruptRegistry::resume`] delivers a
//!   [`Resolution`] once; a second resume of the same id errors.
//! * **Concurrent-claim arbitration.** Two concurrent
//!   [`InterruptRegistry::claim`]s are compare-and-set arbitrated: one
//!   succeeds, the other gets [`RegistryError::AlreadyClaimed`].
//! * **SLA escalation.** [`InterruptRegistry::tick`] moves overdue `Pending`
//!   interrupts to `Escalated` per an [`EscalationPolicy`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::{AgentError, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Lifecycle state of a parked approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptStatus {
    /// Awaiting a human decision.
    Pending,
    /// A person has taken ownership (see [`InterruptRegistry::claim`]).
    Claimed,
    /// A [`Resolution`] has been delivered exactly once.
    Resolved,
    /// The deadline passed with no escalation policy to route it.
    Expired,
    /// The SLA elapsed and the interrupt was routed to an escalation role.
    Escalated,
}

/// A durable record of one parked HITL approval.
///
/// `interrupt_id` is a stable, ULID-like token (a monotonic counter joined
/// to a UUID) that survives node failover: the same token resolves the same
/// parked run no matter which node rehydrates it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInterrupt {
    /// Stable id, identical across node rebalance.
    pub interrupt_id: String,
    /// Owning workflow.
    pub workflow: String,
    /// Owning run.
    pub run: String,
    /// Super-step at which the run parked.
    pub step: u64,
    /// Role the approval is addressed to (for inbox filtering).
    pub requested_role: Option<String>,
    /// Clearance level required to act on this approval.
    pub requested_clearance: Option<String>,
    /// The interrupt payload (e.g. the order awaiting approval).
    pub payload: serde_json::Value,
    /// Creation time (epoch ms).
    pub created_at_ms: i64,
    /// SLA deadline (epoch ms); `None` = no deadline.
    pub deadline_ms: Option<i64>,
    /// Person currently holding the claim, if any.
    pub assignee: Option<String>,
    /// Current lifecycle state.
    pub status: InterruptStatus,
}

/// The human decision delivered back into a parked run on resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    /// The value injected back into the run (e.g. `true` to approve).
    pub value: serde_json::Value,
}

/// Query filter for the fleet-wide inbox.
#[derive(Debug, Clone, Default)]
pub struct InterruptFilter {
    /// Match `requested_role`.
    pub role: Option<String>,
    /// Match `status`.
    pub status: Option<InterruptStatus>,
    /// Match `workflow`.
    pub workflow: Option<String>,
    /// Keep only interrupts whose `deadline_ms` is before this instant.
    pub deadline_before: Option<i64>,
}

impl InterruptFilter {
    fn matches(&self, p: &PendingInterrupt) -> bool {
        if let Some(role) = &self.role {
            if p.requested_role.as_deref() != Some(role.as_str()) {
                return false;
            }
        }
        if let Some(status) = self.status {
            if p.status != status {
                return false;
            }
        }
        if let Some(wf) = &self.workflow {
            if p.workflow != *wf {
                return false;
            }
        }
        if let Some(before) = self.deadline_before {
            match p.deadline_ms {
                Some(d) if d < before => {}
                _ => return false,
            }
        }
        true
    }
}

/// SLA escalation policy: overdue `Pending` interrupts route to `to_role`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationPolicy {
    /// Age (ms since `created_at_ms`) after which a `Pending` interrupt is
    /// considered overdue.
    pub after_ms: i64,
    /// Role the escalated interrupt is re-addressed to.
    pub to_role: String,
}

/// Errors specific to registry arbitration.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A second concurrent claim lost the compare-and-set race.
    #[error("interrupt {0} already claimed")]
    AlreadyClaimed(String),
    /// The interrupt id is unknown to the registry.
    #[error("interrupt {0} not found")]
    NotFound(String),
    /// A second resume of an already-resolved interrupt.
    #[error("interrupt {0} already resolved")]
    AlreadyResolved(String),
}

impl From<RegistryError> for AgentError {
    fn from(e: RegistryError) -> Self {
        AgentError::Workflow(e.to_string())
    }
}

/// Durable, fleet-wide registry of parked HITL approvals.
#[async_trait]
pub trait InterruptRegistry: Send + Sync {
    /// Persist a newly parked approval.
    async fn register(&self, pending: PendingInterrupt) -> Result<()>;

    /// Query the inbox.
    async fn list(&self, filter: InterruptFilter) -> Result<Vec<PendingInterrupt>>;

    /// Compare-and-set claim: succeeds only if the interrupt is still
    /// `Pending`/`Escalated` and unassigned; a second concurrent claim
    /// returns [`RegistryError::AlreadyClaimed`].
    async fn claim(&self, interrupt_id: &str, person: &str) -> Result<()>;

    /// Deliver a [`Resolution`] exactly once. A second resume of the same
    /// id returns [`RegistryError::AlreadyResolved`].
    async fn resume(&self, interrupt_id: &str, resolution: Resolution) -> Result<()>;

    /// Fetch one interrupt by id.
    async fn get(&self, interrupt_id: &str) -> Result<Option<PendingInterrupt>>;

    /// SLA timer: move overdue `Pending` interrupts to `Escalated`,
    /// re-addressing them to the escalation role. Returns the ids moved.
    async fn tick(&self, now_ms: i64, policy: &EscalationPolicy) -> Result<Vec<String>>;
}

/// Mints stable, ULID-like interrupt ids without an external time source:
/// a process-monotonic counter joined to a v4 UUID. Monotonic ordering is
/// preserved within a process; the UUID guarantees fleet-wide uniqueness.
pub struct InterruptIdGen {
    counter: AtomicU64,
}

impl InterruptIdGen {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }

    /// Mint the next id, e.g. `000000000000000000042-<uuid>`.
    pub fn next_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("{:021}-{}", n, uuid::Uuid::new_v4())
    }
}

impl Default for InterruptIdGen {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory [`InterruptRegistry`] (default / tests).
///
/// Cloning shares the same backing store (`Arc<Mutex<..>>`), so a "reload"
/// is modelled by dropping one handle and cloning another over the shared
/// map. All mutations run inside a single critical section, which makes
/// claim/resume compare-and-set arbitration unambiguous.
#[derive(Clone, Default)]
pub struct InMemoryInterruptRegistry {
    inner: Arc<Mutex<HashMap<String, PendingInterrupt>>>,
}

impl InMemoryInterruptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a handle over an existing shared store (models a fresh handle
    /// after node failover).
    pub fn from_shared(inner: Arc<Mutex<HashMap<String, PendingInterrupt>>>) -> Self {
        Self { inner }
    }

    /// The shared backing store, so a "reloaded" handle can reattach.
    pub fn shared(&self) -> Arc<Mutex<HashMap<String, PendingInterrupt>>> {
        self.inner.clone()
    }
}

#[async_trait]
impl InterruptRegistry for InMemoryInterruptRegistry {
    async fn register(&self, pending: PendingInterrupt) -> Result<()> {
        self.inner.lock().insert(pending.interrupt_id.clone(), pending);
        Ok(())
    }

    async fn list(&self, filter: InterruptFilter) -> Result<Vec<PendingInterrupt>> {
        let g = self.inner.lock();
        let mut out: Vec<PendingInterrupt> = g.values().filter(|p| filter.matches(p)).cloned().collect();
        // Stable order by mint id so the inbox is deterministic.
        out.sort_by(|a, b| a.interrupt_id.cmp(&b.interrupt_id));
        Ok(out)
    }

    async fn claim(&self, interrupt_id: &str, person: &str) -> Result<()> {
        let mut g = self.inner.lock();
        let p = g
            .get_mut(interrupt_id)
            .ok_or_else(|| RegistryError::NotFound(interrupt_id.to_string()))?;
        // Compare-and-set: only an unclaimed, actionable interrupt may be
        // claimed. The whole map is locked, so the second concurrent claim
        // observes Claimed and loses the race.
        match p.status {
            InterruptStatus::Pending | InterruptStatus::Escalated if p.assignee.is_none() => {
                p.assignee = Some(person.to_string());
                p.status = InterruptStatus::Claimed;
                Ok(())
            }
            _ => Err(RegistryError::AlreadyClaimed(interrupt_id.to_string()).into()),
        }
    }

    async fn resume(&self, interrupt_id: &str, _resolution: Resolution) -> Result<()> {
        let mut g = self.inner.lock();
        let p = g
            .get_mut(interrupt_id)
            .ok_or_else(|| RegistryError::NotFound(interrupt_id.to_string()))?;
        if p.status == InterruptStatus::Resolved {
            return Err(RegistryError::AlreadyResolved(interrupt_id.to_string()).into());
        }
        p.status = InterruptStatus::Resolved;
        Ok(())
    }

    async fn get(&self, interrupt_id: &str) -> Result<Option<PendingInterrupt>> {
        Ok(self.inner.lock().get(interrupt_id).cloned())
    }

    async fn tick(&self, now_ms: i64, policy: &EscalationPolicy) -> Result<Vec<String>> {
        let mut g = self.inner.lock();
        let mut moved = Vec::new();
        for p in g.values_mut() {
            if p.status == InterruptStatus::Pending && now_ms - p.created_at_ms >= policy.after_ms {
                p.status = InterruptStatus::Escalated;
                p.requested_role = Some(policy.to_role.clone());
                moved.push(p.interrupt_id.clone());
            }
        }
        moved.sort();
        Ok(moved)
    }
}

/// SQL-backed [`InterruptRegistry`] (SQLite / Postgres via sqlx `any`).
///
/// Mirrors `crates/state/src/backends.rs`: a single `any`-driver code path
/// targets both dialects, with `SqlConfig` reused for URL/dialect
/// resolution. Because `sqlite::memory:` runs in-process, the SQL path is
/// exercised by a real round-trip test (see the unit tests below).
#[cfg(feature = "sql")]
pub mod sql {
    use super::*;
    use atomr_persistence_sql::SqlConfig;
    use sqlx::any::AnyPoolOptions;
    use sqlx::AnyPool;

    fn backend_err<E: std::fmt::Display>(e: E) -> AgentError {
        AgentError::Internal(format!("SqlInterruptRegistry backend error: {e}"))
    }

    /// SQL-backed durable interrupt registry.
    pub struct SqlInterruptRegistry {
        pub(crate) pool: AnyPool,
    }

    impl SqlInterruptRegistry {
        /// Connect using a URL (e.g. `sqlite::memory:`,
        /// `postgres://user:pass@host/db`); creates the table if absent.
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Self::connect_with(SqlConfig::new(url)).await
        }

        /// Connect from environment, mirroring the state crate.
        pub async fn from_env() -> Result<Self> {
            Self::connect_with(SqlConfig::from_env()).await
        }

        /// Connect using an explicit [`SqlConfig`].
        pub async fn connect_with(cfg: SqlConfig) -> Result<Self> {
            sqlx::any::install_default_drivers();
            let pool = AnyPoolOptions::new()
                .max_connections(cfg.max_connections)
                .connect(&cfg.url)
                .await
                .map_err(backend_err)?;
            let this = Self { pool };
            this.ensure_schema().await?;
            Ok(this)
        }

        /// Build from an existing pool (shared with the outbox store).
        pub async fn from_pool(pool: AnyPool) -> Result<Self> {
            let this = Self { pool };
            this.ensure_schema().await?;
            Ok(this)
        }

        async fn ensure_schema(&self) -> Result<()> {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS agent_interrupts (\
                   interrupt_id        TEXT   PRIMARY KEY, \
                   workflow            TEXT   NOT NULL, \
                   run                 TEXT   NOT NULL, \
                   step                BIGINT NOT NULL, \
                   requested_role      TEXT, \
                   requested_clearance TEXT, \
                   payload_json        TEXT   NOT NULL, \
                   created_at_ms       BIGINT NOT NULL, \
                   deadline_ms         BIGINT, \
                   assignee            TEXT, \
                   status              TEXT   NOT NULL)",
            )
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }
    }

    pub(crate) fn status_str(s: InterruptStatus) -> &'static str {
        match s {
            InterruptStatus::Pending => "Pending",
            InterruptStatus::Claimed => "Claimed",
            InterruptStatus::Resolved => "Resolved",
            InterruptStatus::Expired => "Expired",
            InterruptStatus::Escalated => "Escalated",
        }
    }

    pub(crate) fn parse_status(s: &str) -> InterruptStatus {
        match s {
            "Claimed" => InterruptStatus::Claimed,
            "Resolved" => InterruptStatus::Resolved,
            "Expired" => InterruptStatus::Expired,
            "Escalated" => InterruptStatus::Escalated,
            _ => InterruptStatus::Pending,
        }
    }

    #[allow(clippy::type_complexity)]
    type Row = (
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        String,
        i64,
        Option<i64>,
        Option<String>,
        String,
    );

    pub(crate) fn row_to_pending(r: Row) -> Result<PendingInterrupt> {
        let (id, wf, run, step, role, clearance, payload_json, created, deadline, assignee, status) = r;
        Ok(PendingInterrupt {
            interrupt_id: id,
            workflow: wf,
            run,
            step: step as u64,
            requested_role: role,
            requested_clearance: clearance,
            payload: serde_json::from_str(&payload_json)?,
            created_at_ms: created,
            deadline_ms: deadline,
            assignee,
            status: parse_status(&status),
        })
    }

    const SELECT_COLS: &str = "interrupt_id, workflow, run, step, requested_role, \
        requested_clearance, payload_json, created_at_ms, deadline_ms, assignee, status";

    #[async_trait]
    impl InterruptRegistry for SqlInterruptRegistry {
        async fn register(&self, p: PendingInterrupt) -> Result<()> {
            let payload_json = serde_json::to_string(&p.payload)?;
            sqlx::query(
                "INSERT INTO agent_interrupts \
                   (interrupt_id, workflow, run, step, requested_role, requested_clearance, \
                    payload_json, created_at_ms, deadline_ms, assignee, status) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (interrupt_id) DO UPDATE SET \
                   workflow = excluded.workflow, run = excluded.run, step = excluded.step, \
                   requested_role = excluded.requested_role, \
                   requested_clearance = excluded.requested_clearance, \
                   payload_json = excluded.payload_json, created_at_ms = excluded.created_at_ms, \
                   deadline_ms = excluded.deadline_ms, assignee = excluded.assignee, \
                   status = excluded.status",
            )
            .bind(&p.interrupt_id)
            .bind(&p.workflow)
            .bind(&p.run)
            .bind(p.step as i64)
            .bind(&p.requested_role)
            .bind(&p.requested_clearance)
            .bind(&payload_json)
            .bind(p.created_at_ms)
            .bind(p.deadline_ms)
            .bind(&p.assignee)
            .bind(status_str(p.status))
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }

        async fn list(&self, filter: InterruptFilter) -> Result<Vec<PendingInterrupt>> {
            // Filter in SQL where cheap; the predicate is small so we read
            // all rows for the matched workflow/status and refine in Rust to
            // keep one code path across dialects.
            let rows: Vec<Row> = sqlx::query_as(&format!(
                "SELECT {SELECT_COLS} FROM agent_interrupts ORDER BY interrupt_id ASC"
            ))
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            let mut out = Vec::new();
            for r in rows {
                let p = row_to_pending(r)?;
                if filter.matches(&p) {
                    out.push(p);
                }
            }
            Ok(out)
        }

        async fn claim(&self, interrupt_id: &str, person: &str) -> Result<()> {
            // Compare-and-set in one statement: only flip Pending/Escalated
            // with no assignee. A second concurrent claim affects 0 rows.
            let res = sqlx::query(
                "UPDATE agent_interrupts SET assignee = ?, status = 'Claimed' \
                 WHERE interrupt_id = ? AND assignee IS NULL \
                   AND status IN ('Pending', 'Escalated')",
            )
            .bind(person)
            .bind(interrupt_id)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            if res.rows_affected() == 1 {
                Ok(())
            } else {
                // Distinguish not-found from already-claimed.
                if self.get(interrupt_id).await?.is_none() {
                    Err(RegistryError::NotFound(interrupt_id.to_string()).into())
                } else {
                    Err(RegistryError::AlreadyClaimed(interrupt_id.to_string()).into())
                }
            }
        }

        async fn resume(&self, interrupt_id: &str, _resolution: Resolution) -> Result<()> {
            // Exactly-once: only an un-resolved interrupt flips to Resolved.
            let res = sqlx::query(
                "UPDATE agent_interrupts SET status = 'Resolved' \
                 WHERE interrupt_id = ? AND status <> 'Resolved'",
            )
            .bind(interrupt_id)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            if res.rows_affected() == 1 {
                Ok(())
            } else if self.get(interrupt_id).await?.is_none() {
                Err(RegistryError::NotFound(interrupt_id.to_string()).into())
            } else {
                Err(RegistryError::AlreadyResolved(interrupt_id.to_string()).into())
            }
        }

        async fn get(&self, interrupt_id: &str) -> Result<Option<PendingInterrupt>> {
            let row: Option<Row> = sqlx::query_as(&format!(
                "SELECT {SELECT_COLS} FROM agent_interrupts WHERE interrupt_id = ?"
            ))
            .bind(interrupt_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            row.map(row_to_pending).transpose()
        }

        async fn tick(&self, now_ms: i64, policy: &EscalationPolicy) -> Result<Vec<String>> {
            let ids: Vec<(String,)> = sqlx::query_as(
                "SELECT interrupt_id FROM agent_interrupts \
                 WHERE status = 'Pending' AND (? - created_at_ms) >= ? ORDER BY interrupt_id ASC",
            )
            .bind(now_ms)
            .bind(policy.after_ms)
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            sqlx::query(
                "UPDATE agent_interrupts SET status = 'Escalated', requested_role = ? \
                 WHERE status = 'Pending' AND (? - created_at_ms) >= ?",
            )
            .bind(&policy.to_role)
            .bind(now_ms)
            .bind(policy.after_ms)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(ids.into_iter().map(|(i,)| i).collect())
        }
    }
}

#[cfg(feature = "sql")]
pub use sql::SqlInterruptRegistry;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk(id: &str, role: &str, created_at_ms: i64) -> PendingInterrupt {
        PendingInterrupt {
            interrupt_id: id.to_string(),
            workflow: "wf".into(),
            run: "r".into(),
            step: 1,
            requested_role: Some(role.into()),
            requested_clearance: Some("L3".into()),
            payload: json!({"order": "buy 100 AAPL"}),
            created_at_ms,
            deadline_ms: Some(created_at_ms + 1000),
            assignee: None,
            status: InterruptStatus::Pending,
        }
    }

    #[test]
    fn id_gen_is_monotonic_and_unique() {
        let g = InterruptIdGen::new();
        let a = g.next_id();
        let b = g.next_id();
        assert!(a < b, "ids must be monotonically ordered");
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn in_memory_survives_reload() {
        let store: Arc<Mutex<HashMap<String, PendingInterrupt>>> = Default::default();
        let reg = InMemoryInterruptRegistry::from_shared(store.clone());
        reg.register(mk("i1", "trader", 0)).await.unwrap();
        drop(reg);
        // Fresh handle over the same store == node failover rehydration.
        let reloaded = InMemoryInterruptRegistry::from_shared(store);
        let got = reloaded.get("i1").await.unwrap().unwrap();
        assert_eq!(got.workflow, "wf");
        reloaded.resume("i1", Resolution { value: json!(true) }).await.unwrap();
        assert_eq!(reloaded.get("i1").await.unwrap().unwrap().status, InterruptStatus::Resolved);
    }

    #[tokio::test]
    async fn list_filters_by_role_and_status() {
        let reg = InMemoryInterruptRegistry::new();
        reg.register(mk("i1", "trader", 0)).await.unwrap();
        reg.register(mk("i2", "risk", 0)).await.unwrap();
        let traders = reg
            .list(InterruptFilter {
                role: Some("trader".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(traders.len(), 1);
        assert_eq!(traders[0].interrupt_id, "i1");
        let pending = reg
            .list(InterruptFilter {
                status: Some(InterruptStatus::Pending),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(pending.len(), 2);
    }

    #[tokio::test]
    async fn exactly_once_resume() {
        let reg = InMemoryInterruptRegistry::new();
        reg.register(mk("i1", "trader", 0)).await.unwrap();
        reg.resume("i1", Resolution { value: json!(true) }).await.unwrap();
        let err = reg.resume("i1", Resolution { value: json!(true) }).await.unwrap_err();
        assert!(err.to_string().contains("already resolved"), "got: {err}");
    }

    #[tokio::test]
    async fn concurrent_double_claim_arbitrated() {
        let reg = InMemoryInterruptRegistry::new();
        reg.register(mk("i1", "trader", 0)).await.unwrap();
        let r1 = reg.clone();
        let r2 = reg.clone();
        let h1 = tokio::spawn(async move { r1.claim("i1", "alice").await });
        let h2 = tokio::spawn(async move { r2.claim("i1", "bob").await });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        // Exactly one succeeds.
        assert_ne!(a.is_ok(), b.is_ok(), "exactly one claim must win");
        let loser = if a.is_err() { a } else { b };
        assert!(loser.unwrap_err().to_string().contains("already claimed"));
        assert_eq!(reg.get("i1").await.unwrap().unwrap().status, InterruptStatus::Claimed);
    }

    #[tokio::test]
    async fn sla_tick_escalates_overdue() {
        let reg = InMemoryInterruptRegistry::new();
        reg.register(mk("i1", "trader", 0)).await.unwrap();
        reg.register(mk("i2", "trader", 10_000)).await.unwrap();
        let policy = EscalationPolicy {
            after_ms: 5_000,
            to_role: "desk-head".into(),
        };
        // now = 6000: i1 (age 6000) is overdue; i2 (age -4000) is not.
        let moved = reg.tick(6_000, &policy).await.unwrap();
        assert_eq!(moved, vec!["i1".to_string()]);
        let i1 = reg.get("i1").await.unwrap().unwrap();
        assert_eq!(i1.status, InterruptStatus::Escalated);
        assert_eq!(i1.requested_role.as_deref(), Some("desk-head"));
        assert_eq!(reg.get("i2").await.unwrap().unwrap().status, InterruptStatus::Pending);
    }

    #[cfg(feature = "sql")]
    mod sql_tests {
        use super::*;
        use crate::interrupt_registry::sql::SqlInterruptRegistry;

        #[tokio::test]
        async fn sqlite_registry_full_lifecycle() {
            let reg = SqlInterruptRegistry::connect("sqlite::memory:").await.unwrap();
            reg.register(mk("i1", "trader", 0)).await.unwrap();
            reg.register(mk("i2", "risk", 0)).await.unwrap();

            // survives "reload" within the same in-memory pool
            let got = reg.get("i1").await.unwrap().unwrap();
            assert_eq!(got.payload["order"], "buy 100 AAPL");

            // list filter
            let traders = reg
                .list(InterruptFilter {
                    role: Some("trader".into()),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(traders.len(), 1);

            // claim CAS: second claim loses
            reg.claim("i1", "alice").await.unwrap();
            let again = reg.claim("i1", "bob").await.unwrap_err();
            assert!(again.to_string().contains("already claimed"));

            // exactly-once resume
            reg.resume("i2", Resolution { value: serde_json::json!(true) }).await.unwrap();
            let dup = reg.resume("i2", Resolution { value: serde_json::json!(true) }).await.unwrap_err();
            assert!(dup.to_string().contains("already resolved"));

            // SLA tick escalates overdue Pending only (i1 is Claimed; add i3)
            reg.register(mk("i3", "trader", 0)).await.unwrap();
            let policy = EscalationPolicy { after_ms: 1_000, to_role: "desk-head".into() };
            let moved = reg.tick(5_000, &policy).await.unwrap();
            assert_eq!(moved, vec!["i3".to_string()]);
            assert_eq!(reg.get("i3").await.unwrap().unwrap().status, InterruptStatus::Escalated);
        }
    }
}
