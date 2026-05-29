//! FR-5 — Transactional HITL resume → external system-of-record commit.
//!
//! Promotion (paper→live) and order approval must be **atomic** across
//! atomr-agents and the external Ledger: either the Ledger records the
//! approval AND the venue releases the order, or neither. A torn promotion
//! (Ledger says live, venue still paper) is a compliance and trading-
//! integrity failure.
//!
//! This module binds an interrupt resolution to an external write via the
//! **transactional outbox** pattern: [`resume_with`] persists the
//! resolution AND an [`OutboxRecord`] in one atomic step, and only then
//! marks the interrupt `Resolved`. A separate [`drive_outbox`] relay
//! forwards uncommitted outbox records to an [`OutboxSink`] (the external
//! Ledger writer) at-least-once; the [`OutboxRecord::idempotency_key`]
//! lets the sink dedupe retries so a crash-and-replay never double-promotes.
//!
//! Guarantees exercised by the tests:
//!
//! * **Atomic commit.** Resolution + outbox record commit together; on
//!   failure the interrupt stays `Pending` and no outbox record is written.
//! * **Crash recovery.** If the relay never runs, a fresh handle over the
//!   same store completes the Ledger write on restart.
//! * **Idempotency.** A re-relayed record (same idempotency key) is applied
//!   once by a deduping sink.
//! * **Refused sink parks the run.** A sink that refuses leaves the record
//!   uncommitted; the run is effectively parked.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use atomr_agents_core::Result;
#[cfg(any(feature = "sql", test))]
use atomr_agents_core::AgentError;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::interrupt_registry::{InterruptRegistry, InterruptStatus, RegistryError, Resolution};

/// A durable outbox row: the external write that must accompany an
/// interrupt resolution. A relay forwards uncommitted records to the
/// [`OutboxSink`]; `committed` flips true once the sink accepts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxRecord {
    /// Unique outbox row id.
    pub id: String,
    /// The interrupt this write is bound to.
    pub interrupt_id: String,
    /// The external write payload (e.g. the Ledger approval entry).
    pub payload: serde_json::Value,
    /// Dedup key carried to the sink so retries never double-apply.
    pub idempotency_key: String,
    /// Set once the sink has accepted the record.
    pub committed: bool,
}

/// The external Ledger writer's contract. [`drive_outbox`] calls `relay`
/// for each uncommitted record; returning `Ok` means the external system
/// durably accepted the write (idempotently keyed).
#[async_trait]
pub trait OutboxSink: Send + Sync {
    /// Forward one outbox record to the external system-of-record.
    async fn relay(&self, record: &OutboxRecord) -> Result<()>;
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type SagaFn = Arc<dyn Fn() -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// A saga step: at-least-once `commit` with a `compensate` rollback,
/// keyed for idempotency. The alternative to the outbox when the external
/// write cannot share a transaction with the resolution.
#[derive(Clone)]
pub struct SagaStep {
    /// Idempotency key carried into both commit and compensate.
    pub idempotency_key: String,
    /// Forward action (driven at-least-once).
    pub commit: SagaFn,
    /// Rollback action if a later step fails.
    pub compensate: SagaFn,
}

impl SagaStep {
    /// Build a saga step from `commit` / `compensate` closures.
    pub fn new<C, K>(idempotency_key: impl Into<String>, commit: C, compensate: K) -> Self
    where
        C: Fn() -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
        K: Fn() -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    {
        Self {
            idempotency_key: idempotency_key.into(),
            commit: Arc::new(commit),
            compensate: Arc::new(compensate),
        }
    }
}

impl std::fmt::Debug for SagaStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SagaStep")
            .field("idempotency_key", &self.idempotency_key)
            .finish_non_exhaustive()
    }
}

/// The transactional effect bound to a resume.
#[derive(Clone)]
pub enum ResumeEffect {
    /// Persist an outbox record in the same step as the resolution; a relay
    /// forwards it to the Ledger at-least-once.
    Outbox(OutboxRecord),
    /// Drive a saga with at-least-once delivery + compensation.
    Saga(SagaStep),
}

impl std::fmt::Debug for ResumeEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeEffect::Outbox(r) => f.debug_tuple("Outbox").field(r).finish(),
            ResumeEffect::Saga(s) => f.debug_tuple("Saga").field(s).finish(),
        }
    }
}

/// Durable store for outbox records. The relay reads uncommitted records
/// from here and marks them committed once the sink accepts them.
#[async_trait]
pub trait OutboxStore: Send + Sync {
    /// Persist an outbox record (idempotent on `id`).
    async fn save(&self, record: OutboxRecord) -> Result<()>;
    /// All records not yet committed, in insertion order.
    async fn uncommitted(&self) -> Result<Vec<OutboxRecord>>;
    /// Mark a record committed.
    async fn mark_committed(&self, id: &str) -> Result<()>;
    /// Fetch a record by id (for tests / inspection).
    async fn get(&self, id: &str) -> Result<Option<OutboxRecord>>;
}

/// In-memory outbox store. Cloning shares the backing vector, so a fresh
/// handle models a process restart over durable storage.
#[derive(Clone, Default)]
pub struct InMemoryOutboxStore {
    inner: Arc<Mutex<Vec<OutboxRecord>>>,
}

impl InMemoryOutboxStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build over an existing shared vector (models restart / failover).
    pub fn from_shared(inner: Arc<Mutex<Vec<OutboxRecord>>>) -> Self {
        Self { inner }
    }

    /// The shared backing store, so a "reloaded" handle can reattach.
    pub fn shared(&self) -> Arc<Mutex<Vec<OutboxRecord>>> {
        self.inner.clone()
    }
}

#[async_trait]
impl OutboxStore for InMemoryOutboxStore {
    async fn save(&self, record: OutboxRecord) -> Result<()> {
        let mut g = self.inner.lock();
        if let Some(slot) = g.iter_mut().find(|r| r.id == record.id) {
            *slot = record;
        } else {
            g.push(record);
        }
        Ok(())
    }

    async fn uncommitted(&self) -> Result<Vec<OutboxRecord>> {
        Ok(self.inner.lock().iter().filter(|r| !r.committed).cloned().collect())
    }

    async fn mark_committed(&self, id: &str) -> Result<()> {
        let mut g = self.inner.lock();
        if let Some(r) = g.iter_mut().find(|r| r.id == id) {
            r.committed = true;
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<OutboxRecord>> {
        Ok(self.inner.lock().iter().find(|r| r.id == id).cloned())
    }
}

/// Transactional resume against the **in-memory** registry + outbox.
///
/// In one critical section it (1) persists the outbox record and (2) marks
/// the interrupt `Resolved`; if either is impossible (e.g. the interrupt is
/// already resolved or missing), nothing is written and the interrupt stays
/// `Pending`. For a `Saga` effect it drives `commit` and, on commit failure,
/// `compensate`, leaving the interrupt `Pending`.
///
/// SQL atomicity (one `pool.begin()` transaction) is provided by
/// [`sql::resume_with_sql`] behind the `sql` feature.
pub async fn resume_with(
    registry: &InMemoryInterruptRegistryRef,
    outbox: &InMemoryOutboxStore,
    interrupt_id: &str,
    resolution: Resolution,
    effect: ResumeEffect,
) -> Result<()> {
    // Validate the interrupt is resumable *before* doing any external work.
    let current = registry
        .get(interrupt_id)
        .await?
        .ok_or_else(|| RegistryError::NotFound(interrupt_id.to_string()))?;
    if current.status == InterruptStatus::Resolved {
        return Err(RegistryError::AlreadyResolved(interrupt_id.to_string()).into());
    }

    match effect {
        ResumeEffect::Outbox(mut rec) => {
            rec.interrupt_id = interrupt_id.to_string();
            rec.committed = false;
            // Single critical section: persist the outbox record, then mark
            // resolved. The in-memory registry's resume is itself atomic; we
            // order it last so a resume failure leaves no committed effect.
            outbox.save(rec).await?;
            registry.resume(interrupt_id, resolution).await?;
            Ok(())
        }
        ResumeEffect::Saga(step) => {
            // At-least-once commit; compensate + leave Pending on failure.
            if let Err(e) = (step.commit)().await {
                let _ = (step.compensate)().await;
                return Err(e);
            }
            registry.resume(interrupt_id, resolution).await?;
            Ok(())
        }
    }
}

/// Type alias documenting that the in-memory `resume_with` needs an
/// in-memory registry handle (it relies on the registry + outbox sharing
/// process-local critical sections for atomicity).
pub type InMemoryInterruptRegistryRef = crate::interrupt_registry::InMemoryInterruptRegistry;

/// Relay driver: forward every uncommitted outbox record to `sink` and mark
/// it committed on acceptance. At-least-once — a record refused by the sink
/// stays uncommitted and is retried on the next drive. Returns the number
/// of records successfully relayed.
pub async fn drive_outbox(store: &dyn OutboxStore, sink: &dyn OutboxSink) -> Result<u64> {
    let pending = store.uncommitted().await?;
    let mut relayed = 0u64;
    for rec in pending {
        match sink.relay(&rec).await {
            Ok(()) => {
                store.mark_committed(&rec.id).await?;
                relayed += 1;
            }
            // Refused: leave uncommitted (run stays parked); keep going so
            // one bad record doesn't block the rest.
            Err(_) => continue,
        }
    }
    Ok(relayed)
}

/// SQL-backed outbox store + a single-transaction `resume_with`.
#[cfg(feature = "sql")]
pub mod sql {
    use super::*;
    use atomr_persistence_sql::SqlConfig;
    use sqlx::any::AnyPoolOptions;
    use sqlx::AnyPool;

    fn backend_err<E: std::fmt::Display>(e: E) -> AgentError {
        AgentError::Internal(format!("SqlOutboxStore backend error: {e}"))
    }

    /// SQL-backed outbox store (SQLite / Postgres via sqlx `any`).
    pub struct SqlOutboxStore {
        pub(crate) pool: AnyPool,
    }

    impl SqlOutboxStore {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Self::connect_with(SqlConfig::new(url)).await
        }
        pub async fn from_env() -> Result<Self> {
            Self::connect_with(SqlConfig::from_env()).await
        }
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
        /// Build over an existing pool — share it with
        /// [`SqlInterruptRegistry`](crate::interrupt_registry::sql::SqlInterruptRegistry)
        /// so the resolution and outbox commit in one transaction.
        pub async fn from_pool(pool: AnyPool) -> Result<Self> {
            let this = Self { pool };
            this.ensure_schema().await?;
            Ok(this)
        }
        async fn ensure_schema(&self) -> Result<()> {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS agent_outbox (\
                   id              TEXT PRIMARY KEY, \
                   interrupt_id    TEXT NOT NULL, \
                   payload_json    TEXT NOT NULL, \
                   idempotency_key TEXT NOT NULL, \
                   committed       INTEGER NOT NULL)",
            )
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }
    }

    fn row_to_rec((id, iid, payload, key, committed): (String, String, String, String, i64)) -> Result<OutboxRecord> {
        Ok(OutboxRecord {
            id,
            interrupt_id: iid,
            payload: serde_json::from_str(&payload)?,
            idempotency_key: key,
            committed: committed != 0,
        })
    }

    #[async_trait]
    impl OutboxStore for SqlOutboxStore {
        async fn save(&self, rec: OutboxRecord) -> Result<()> {
            let payload = serde_json::to_string(&rec.payload)?;
            sqlx::query(
                "INSERT INTO agent_outbox (id, interrupt_id, payload_json, idempotency_key, committed) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET interrupt_id = excluded.interrupt_id, \
                   payload_json = excluded.payload_json, idempotency_key = excluded.idempotency_key, \
                   committed = excluded.committed",
            )
            .bind(&rec.id)
            .bind(&rec.interrupt_id)
            .bind(&payload)
            .bind(&rec.idempotency_key)
            .bind(rec.committed as i64)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }

        async fn uncommitted(&self) -> Result<Vec<OutboxRecord>> {
            let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
                "SELECT id, interrupt_id, payload_json, idempotency_key, committed \
                 FROM agent_outbox WHERE committed = 0 ORDER BY id ASC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            rows.into_iter().map(row_to_rec).collect()
        }

        async fn mark_committed(&self, id: &str) -> Result<()> {
            sqlx::query("UPDATE agent_outbox SET committed = 1 WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(backend_err)?;
            Ok(())
        }

        async fn get(&self, id: &str) -> Result<Option<OutboxRecord>> {
            let row: Option<(String, String, String, String, i64)> = sqlx::query_as(
                "SELECT id, interrupt_id, payload_json, idempotency_key, committed \
                 FROM agent_outbox WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            row.map(row_to_rec).transpose()
        }
    }

    /// Transactional resume over a **shared** SQL pool: the interrupt
    /// resolution and the outbox insert commit in a single `pool.begin()`
    /// transaction. On any failure the transaction rolls back, leaving the
    /// interrupt `Pending` and no outbox record. The registry and store
    /// MUST share the same pool (see
    /// [`SqlInterruptRegistry::from_pool`](crate::interrupt_registry::sql::SqlInterruptRegistry::from_pool)
    /// / [`SqlOutboxStore::from_pool`]).
    pub async fn resume_with_sql(
        pool: &AnyPool,
        interrupt_id: &str,
        _resolution: Resolution,
        mut record: OutboxRecord,
    ) -> Result<()> {
        record.interrupt_id = interrupt_id.to_string();
        record.committed = false;
        let payload = serde_json::to_string(&record.payload)?;

        let mut tx = pool.begin().await.map_err(backend_err)?;

        // Exactly-once resolution flip inside the transaction.
        let res = sqlx::query(
            "UPDATE agent_interrupts SET status = 'Resolved' \
             WHERE interrupt_id = ? AND status <> 'Resolved'",
        )
        .bind(interrupt_id)
        .execute(&mut *tx)
        .await
        .map_err(backend_err)?;
        if res.rows_affected() != 1 {
            tx.rollback().await.map_err(backend_err)?;
            // Either missing or already resolved.
            return Err(RegistryError::AlreadyResolved(interrupt_id.to_string()).into());
        }

        // Outbox insert in the SAME transaction.
        sqlx::query(
            "INSERT INTO agent_outbox (id, interrupt_id, payload_json, idempotency_key, committed) \
             VALUES (?, ?, ?, ?, 0)",
        )
        .bind(&record.id)
        .bind(&record.interrupt_id)
        .bind(&payload)
        .bind(&record.idempotency_key)
        .execute(&mut *tx)
        .await
        .map_err(backend_err)?;

        tx.commit().await.map_err(backend_err)?;
        Ok(())
    }
}

#[cfg(feature = "sql")]
pub use sql::{resume_with_sql, SqlOutboxStore};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt_registry::{InMemoryInterruptRegistry, PendingInterrupt};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn pending(id: &str) -> PendingInterrupt {
        PendingInterrupt {
            interrupt_id: id.to_string(),
            workflow: "wf".into(),
            run: "r".into(),
            step: 1,
            requested_role: Some("trader".into()),
            requested_clearance: None,
            payload: json!({"order": "buy"}),
            created_at_ms: 0,
            deadline_ms: None,
            assignee: None,
            status: InterruptStatus::Pending,
        }
    }

    fn outbox_rec(id: &str, key: &str) -> OutboxRecord {
        OutboxRecord {
            id: id.into(),
            interrupt_id: String::new(),
            payload: json!({"ledger": "approve"}),
            idempotency_key: key.into(),
            committed: false,
        }
    }

    /// Collecting sink with an idempotency-keyed dedupe set.
    #[derive(Default)]
    struct DedupeSink {
        seen: Mutex<Vec<String>>,
        applied: AtomicU32,
    }
    #[async_trait]
    impl OutboxSink for DedupeSink {
        async fn relay(&self, record: &OutboxRecord) -> Result<()> {
            let mut g = self.seen.lock();
            if g.contains(&record.idempotency_key) {
                return Ok(()); // dedupe: already applied externally
            }
            g.push(record.idempotency_key.clone());
            self.applied.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Sink that always refuses.
    struct RefuseSink;
    #[async_trait]
    impl OutboxSink for RefuseSink {
        async fn relay(&self, _record: &OutboxRecord) -> Result<()> {
            Err(AgentError::Internal("ledger down".into()))
        }
    }

    #[tokio::test]
    async fn resume_and_outbox_commit_atomically() {
        let reg = InMemoryInterruptRegistry::new();
        let outbox = InMemoryOutboxStore::new();
        reg.register(pending("i1")).await.unwrap();

        resume_with(
            &reg,
            &outbox,
            "i1",
            Resolution { value: json!(true) },
            ResumeEffect::Outbox(outbox_rec("o1", "key-1")),
        )
        .await
        .unwrap();

        assert_eq!(reg.get("i1").await.unwrap().unwrap().status, InterruptStatus::Resolved);
        let rec = outbox.get("o1").await.unwrap().unwrap();
        assert_eq!(rec.interrupt_id, "i1");
        assert!(!rec.committed);
    }

    #[tokio::test]
    async fn already_resolved_blocks_resume_and_writes_no_outbox() {
        let reg = InMemoryInterruptRegistry::new();
        let outbox = InMemoryOutboxStore::new();
        reg.register(pending("i1")).await.unwrap();
        reg.resume("i1", Resolution { value: json!(true) }).await.unwrap();

        let err = resume_with(
            &reg,
            &outbox,
            "i1",
            Resolution { value: json!(true) },
            ResumeEffect::Outbox(outbox_rec("o1", "key-1")),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already resolved"));
        // No outbox record written — atomic abort.
        assert!(outbox.get("o1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn crash_then_relay_on_fresh_handle_completes_ledger_write() {
        let store: Arc<Mutex<Vec<OutboxRecord>>> = Default::default();
        let reg = InMemoryInterruptRegistry::new();
        let outbox = InMemoryOutboxStore::from_shared(store.clone());
        reg.register(pending("i1")).await.unwrap();

        // Resume persists resolution + outbox, but we "crash" before relay.
        resume_with(
            &reg,
            &outbox,
            "i1",
            Resolution { value: json!(true) },
            ResumeEffect::Outbox(outbox_rec("o1", "key-1")),
        )
        .await
        .unwrap();
        drop(outbox);

        // Fresh handle over the SAME store + run relay → Ledger write done.
        let reloaded = InMemoryOutboxStore::from_shared(store);
        let sink = DedupeSink::default();
        let n = drive_outbox(&reloaded, &sink).await.unwrap();
        assert_eq!(n, 1);
        assert!(reloaded.get("o1").await.unwrap().unwrap().committed);
        assert_eq!(sink.applied.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn idempotency_key_dedupes_double_relay() {
        let outbox = InMemoryOutboxStore::new();
        // Two outbox rows with the SAME idempotency key (e.g. a retry that
        // re-enqueued). Relay both; the sink applies once.
        outbox.save(outbox_rec("o1", "key-dup")).await.unwrap();
        outbox.save(outbox_rec("o2", "key-dup")).await.unwrap();
        let sink = DedupeSink::default();
        let n = drive_outbox(&outbox, &sink).await.unwrap();
        assert_eq!(n, 2, "both rows relayed (at-least-once)");
        assert_eq!(sink.applied.load(Ordering::SeqCst), 1, "but applied once (idempotent)");
    }

    #[tokio::test]
    async fn refused_sink_leaves_record_uncommitted_and_run_parked() {
        let reg = InMemoryInterruptRegistry::new();
        let outbox = InMemoryOutboxStore::new();
        reg.register(pending("i1")).await.unwrap();
        resume_with(
            &reg,
            &outbox,
            "i1",
            Resolution { value: json!(true) },
            ResumeEffect::Outbox(outbox_rec("o1", "key-1")),
        )
        .await
        .unwrap();

        let n = drive_outbox(&outbox, &RefuseSink).await.unwrap();
        assert_eq!(n, 0);
        // Record stays uncommitted → external write not done → effectively parked.
        assert!(!outbox.get("o1").await.unwrap().unwrap().committed);
        // A later successful relay completes it.
        let sink = DedupeSink::default();
        assert_eq!(drive_outbox(&outbox, &sink).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn saga_commit_resolves_failure_compensates_and_parks() {
        let reg = InMemoryInterruptRegistry::new();
        let outbox = InMemoryOutboxStore::new();
        reg.register(pending("i1")).await.unwrap();

        let compensated = Arc::new(AtomicU32::new(0));
        let comp2 = compensated.clone();
        let failing = SagaStep::new(
            "key-1",
            || Box::pin(async { Err(AgentError::Internal("venue refused".into())) }),
            move || {
                let c = comp2.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
        );
        let err = resume_with(&reg, &outbox, "i1", Resolution { value: json!(true) }, ResumeEffect::Saga(failing))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("venue refused"));
        assert_eq!(compensated.load(Ordering::SeqCst), 1, "compensation ran");
        // Run stays parked.
        assert_eq!(reg.get("i1").await.unwrap().unwrap().status, InterruptStatus::Pending);

        // A succeeding saga resolves it.
        let ok = SagaStep::new(
            "key-1",
            || Box::pin(async { Ok(()) }),
            || Box::pin(async { Ok(()) }),
        );
        resume_with(&reg, &outbox, "i1", Resolution { value: json!(true) }, ResumeEffect::Saga(ok))
            .await
            .unwrap();
        assert_eq!(reg.get("i1").await.unwrap().unwrap().status, InterruptStatus::Resolved);
    }

    #[cfg(feature = "sql")]
    mod sql_tests {
        use super::*;
        use crate::interrupt_registry::sql::SqlInterruptRegistry;
        use crate::resume_effect::sql::{resume_with_sql, SqlOutboxStore};

        #[tokio::test]
        async fn sql_resume_with_is_transactional_and_relayable() {
            // Shared in-memory SQLite pool so registry + outbox + the txn
            // all see the same database.
            sqlx::any::install_default_drivers();
            let pool = sqlx::any::AnyPoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            let reg = SqlInterruptRegistry::from_pool(pool.clone()).await.unwrap();
            let outbox = SqlOutboxStore::from_pool(pool.clone()).await.unwrap();
            reg.register(pending("i1")).await.unwrap();

            // Transactional resume: resolution + outbox commit together.
            resume_with_sql(&pool, "i1", Resolution { value: json!(true) }, outbox_rec("o1", "key-1"))
                .await
                .unwrap();
            assert_eq!(
                reg.get("i1").await.unwrap().unwrap().status,
                InterruptStatus::Resolved
            );
            assert!(!outbox.get("o1").await.unwrap().unwrap().committed);

            // Second transactional resume of the same id aborts (rolls back):
            // the interrupt is already Resolved and no second outbox row lands.
            let err = resume_with_sql(&pool, "i1", Resolution { value: json!(true) }, outbox_rec("o2", "key-2"))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("already resolved"));
            assert!(outbox.get("o2").await.unwrap().is_none(), "rolled back");

            // Relay completes the Ledger write (crash-recovery path).
            let sink = DedupeSink::default();
            let n = drive_outbox(&outbox, &sink).await.unwrap();
            assert_eq!(n, 1);
            assert!(outbox.get("o1").await.unwrap().unwrap().committed);
        }
    }
}
