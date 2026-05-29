//! Durable [`Checkpointer`](crate::Checkpointer) backends.
//!
//! * [`sql::SqlCheckpointer`] — sqlx over the `any` driver, so a single
//!   code path targets SQLite and Postgres. `SqliteCheckpointer` and
//!   `PostgresCheckpointer` are dialect-named aliases; the dialect is
//!   auto-detected from the connection URL. Because `sqlite::memory:`
//!   runs in-process, the SQL backend is exercised by a real round-trip
//!   test (see the unit tests below); Postgres is covered by an
//!   integration test that runs when `ATOMR_IT_SQL_URL` is set.
//! * [`redis_backend::RedisCheckpointer`] — Redis hot-tier store keyed by
//!   `(workflow, run, super_step)` with a per-run sorted-set index for
//!   `latest`/`list`.

#[cfg(feature = "sql")]
pub mod sql {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, RunId, Value, WorkflowId};
    use atomr_persistence_sql::SqlConfig;
    use sqlx::any::AnyPoolOptions;
    use sqlx::AnyPool;
    use std::collections::HashMap;

    use crate::checkpointer::{CheckpointKey, CheckpointMeta, Checkpointer, Snapshot};

    /// SQL-backed checkpointer (SQLite / Postgres via sqlx `any`).
    pub struct SqlCheckpointer {
        pool: AnyPool,
    }

    fn backend_err<E: std::fmt::Display>(e: E) -> AgentError {
        AgentError::Internal(format!("SqlCheckpointer backend error: {e}"))
    }

    impl SqlCheckpointer {
        /// Connect using a URL (e.g. `sqlite::memory:`,
        /// `postgres://user:pass@host/db`). The dialect is detected from
        /// the URL; the `agent_checkpoints` table is created if absent.
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            let cfg = SqlConfig::new(url);
            Self::connect_with(cfg).await
        }

        /// Connect from environment (`ATOMR_PERSISTENCE_SQL_URL`,
        /// `ATOMR_IT_SQL_URL`, `DATABASE_URL`, else `sqlite::memory:`).
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

        /// Build from an existing pool (e.g. shared with other stores).
        pub async fn from_pool(pool: AnyPool) -> Result<Self> {
            let this = Self { pool };
            this.ensure_schema().await?;
            Ok(this)
        }

        async fn ensure_schema(&self) -> Result<()> {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS agent_checkpoints (\
                   workflow_id  TEXT   NOT NULL, \
                   run_id       TEXT   NOT NULL, \
                   super_step   BIGINT NOT NULL, \
                   label        TEXT   NOT NULL, \
                   values_json  TEXT   NOT NULL, \
                   timestamp_ms BIGINT NOT NULL, \
                   PRIMARY KEY (workflow_id, run_id, super_step))",
            )
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }
    }

    #[async_trait]
    impl Checkpointer for SqlCheckpointer {
        async fn save(&self, snapshot: Snapshot) -> Result<()> {
            let values_json = serde_json::to_string(&snapshot.values)?;
            // Upsert: a re-save at the same (workflow, run, step) replaces.
            sqlx::query(
                "INSERT INTO agent_checkpoints \
                   (workflow_id, run_id, super_step, label, values_json, timestamp_ms) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (workflow_id, run_id, super_step) DO UPDATE SET \
                   label = excluded.label, \
                   values_json = excluded.values_json, \
                   timestamp_ms = excluded.timestamp_ms",
            )
            .bind(snapshot.key.workflow_id.as_str())
            .bind(snapshot.key.run_id.as_str())
            .bind(snapshot.key.super_step as i64)
            .bind(&snapshot.label)
            .bind(&values_json)
            .bind(snapshot.timestamp_ms)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }

        async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>> {
            let row: Option<(String, String, i64, String, String, i64)> = sqlx::query_as(
                "SELECT workflow_id, run_id, super_step, label, values_json, timestamp_ms \
                 FROM agent_checkpoints \
                 WHERE workflow_id = ? AND run_id = ? AND super_step = ?",
            )
            .bind(key.workflow_id.as_str())
            .bind(key.run_id.as_str())
            .bind(key.super_step as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            row.map(row_to_snapshot).transpose()
        }

        async fn latest(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Option<Snapshot>> {
            let row: Option<(String, String, i64, String, String, i64)> = sqlx::query_as(
                "SELECT workflow_id, run_id, super_step, label, values_json, timestamp_ms \
                 FROM agent_checkpoints \
                 WHERE workflow_id = ? AND run_id = ? \
                 ORDER BY super_step DESC LIMIT 1",
            )
            .bind(workflow_id.as_str())
            .bind(run_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            row.map(row_to_snapshot).transpose()
        }

        async fn list(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<CheckpointMeta>> {
            let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
                "SELECT workflow_id, run_id, super_step, timestamp_ms \
                 FROM agent_checkpoints \
                 WHERE workflow_id = ? AND run_id = ? \
                 ORDER BY super_step ASC",
            )
            .bind(workflow_id.as_str())
            .bind(run_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(rows
                .into_iter()
                .map(|(wf, run, step, ts)| CheckpointMeta {
                    workflow_id: WorkflowId::from(wf),
                    run_id: RunId::from(run),
                    super_step: step as u64,
                    timestamp_ms: ts,
                })
                .collect())
        }

        async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId> {
            let snap = self.load(from).await?.ok_or_else(|| {
                AgentError::Internal(format!(
                    "fork: source checkpoint {}#{} not found",
                    from.run_id.as_str(),
                    from.super_step
                ))
            })?;
            let new_run = RunId::new();
            let mut values = snap.values.clone();
            for (k, v) in edits {
                values.insert(k, v);
            }
            self.save(Snapshot {
                key: CheckpointKey {
                    workflow_id: snap.key.workflow_id.clone(),
                    run_id: new_run.clone(),
                    super_step: snap.key.super_step,
                },
                values,
                label: format!("fork-of:{}", from.run_id.as_str()),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            })
            .await?;
            Ok(new_run)
        }
    }

    fn row_to_snapshot(
        (wf, run, step, label, values_json, ts): (String, String, i64, String, String, i64),
    ) -> Result<Snapshot> {
        let values: HashMap<String, Value> = serde_json::from_str(&values_json)?;
        Ok(Snapshot {
            key: CheckpointKey {
                workflow_id: WorkflowId::from(wf),
                run_id: RunId::from(run),
                super_step: step as u64,
            },
            values,
            label,
            timestamp_ms: ts,
        })
    }

    /// SQL-backed [`StepRecordStore`](crate::StepRecordStore) (FR-1),
    /// sharing the sqlx `any` pool pattern with [`SqlCheckpointer`].
    pub struct SqlStepRecordStore {
        pool: AnyPool,
    }

    impl SqlStepRecordStore {
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
        pub async fn from_pool(pool: AnyPool) -> Result<Self> {
            let this = Self { pool };
            this.ensure_schema().await?;
            Ok(this)
        }
        async fn ensure_schema(&self) -> Result<()> {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS agent_step_records (\
                   workflow_id TEXT   NOT NULL, \
                   run_id      TEXT   NOT NULL, \
                   super_step  BIGINT NOT NULL, \
                   record_json TEXT   NOT NULL, \
                   PRIMARY KEY (workflow_id, run_id, super_step))",
            )
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }
    }

    #[async_trait]
    impl crate::record::StepRecordStore for SqlStepRecordStore {
        async fn save_record(&self, record: crate::record::StepRecord) -> Result<()> {
            let json = serde_json::to_string(&record)?;
            sqlx::query(
                "INSERT INTO agent_step_records (workflow_id, run_id, super_step, record_json) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT (workflow_id, run_id, super_step) DO UPDATE SET record_json = excluded.record_json",
            )
            .bind(record.key.workflow_id.as_str())
            .bind(record.key.run_id.as_str())
            .bind(record.key.super_step as i64)
            .bind(&json)
            .execute(&self.pool)
            .await
            .map_err(backend_err)?;
            Ok(())
        }

        async fn load_record(&self, key: &CheckpointKey) -> Result<Option<crate::record::StepRecord>> {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT record_json FROM agent_step_records \
                 WHERE workflow_id = ? AND run_id = ? AND super_step = ?",
            )
            .bind(key.workflow_id.as_str())
            .bind(key.run_id.as_str())
            .bind(key.super_step as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;
            row.map(|(j,)| serde_json::from_str(&j).map_err(Into::into))
                .transpose()
        }

        async fn list_records(
            &self,
            workflow_id: &WorkflowId,
            run_id: &RunId,
        ) -> Result<Vec<crate::record::StepRecord>> {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT record_json FROM agent_step_records \
                 WHERE workflow_id = ? AND run_id = ? ORDER BY super_step ASC",
            )
            .bind(workflow_id.as_str())
            .bind(run_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(backend_err)?;
            rows.into_iter()
                .map(|(j,)| serde_json::from_str(&j).map_err(Into::into))
                .collect()
        }
    }

    /// Dialect-named alias of [`SqlCheckpointer`] (URL auto-detects).
    #[cfg(feature = "sqlite")]
    pub type SqliteCheckpointer = SqlCheckpointer;
    /// Dialect-named alias of [`SqlCheckpointer`] (URL auto-detects).
    #[cfg(feature = "postgres")]
    pub type PostgresCheckpointer = SqlCheckpointer;

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::checkpointer::Checkpointer;
        use serde_json::json;
        use std::collections::HashMap;

        fn snap(wf: &str, run: &str, step: u64, label: &str, k: &str, v: Value) -> Snapshot {
            let mut values = HashMap::new();
            values.insert(k.to_string(), v);
            Snapshot {
                key: CheckpointKey {
                    workflow_id: WorkflowId::from(wf),
                    run_id: RunId::from(run),
                    super_step: step,
                },
                values,
                label: label.into(),
                timestamp_ms: 1,
            }
        }

        // Runs in-process against SQLite — no external service needed.
        #[tokio::test]
        async fn sqlite_roundtrip_save_load_latest_list_fork() {
            let c = SqlCheckpointer::connect("sqlite::memory:").await.unwrap();

            c.save(snap("wf", "r", 0, "init", "a", json!(1))).await.unwrap();
            c.save(snap("wf", "r", 2, "after", "a", json!(2))).await.unwrap();

            // load specific step
            let s0 = c
                .load(&CheckpointKey {
                    workflow_id: WorkflowId::from("wf"),
                    run_id: RunId::from("r"),
                    super_step: 0,
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(s0.values["a"], json!(1));

            // latest
            let latest = c
                .latest(&WorkflowId::from("wf"), &RunId::from("r"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(latest.key.super_step, 2);
            assert_eq!(latest.values["a"], json!(2));

            // list ordered
            let metas = c.list(&WorkflowId::from("wf"), &RunId::from("r")).await.unwrap();
            assert_eq!(metas.len(), 2);
            assert_eq!(metas[0].super_step, 0);
            assert_eq!(metas[1].super_step, 2);

            // upsert replaces
            c.save(snap("wf", "r", 2, "after2", "a", json!(99)))
                .await
                .unwrap();
            let latest = c
                .latest(&WorkflowId::from("wf"), &RunId::from("r"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(latest.values["a"], json!(99));
            assert_eq!(
                c.list(&WorkflowId::from("wf"), &RunId::from("r"))
                    .await
                    .unwrap()
                    .len(),
                2
            );

            // fork
            let new_run = c
                .fork(
                    &CheckpointKey {
                        workflow_id: WorkflowId::from("wf"),
                        run_id: RunId::from("r"),
                        super_step: 0,
                    },
                    vec![("a".into(), json!(7))],
                )
                .await
                .unwrap();
            let forked = c
                .latest(&WorkflowId::from("wf"), &new_run)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(forked.values["a"], json!(7));
            assert!(forked.label.starts_with("fork-of:r"));
        }

        // Postgres path: same code, exercised only when a server is set.
        #[tokio::test]
        async fn postgres_roundtrip_when_configured() {
            let Ok(url) = std::env::var("ATOMR_IT_SQL_URL") else {
                eprintln!("skipping: ATOMR_IT_SQL_URL not set");
                return;
            };
            if !url.starts_with("postgres") {
                return;
            }
            let c = SqlCheckpointer::connect(url).await.unwrap();
            let run = format!("it-{}", RunId::new().as_str());
            c.save(snap("wf-it", &run, 0, "init", "a", json!(1)))
                .await
                .unwrap();
            let latest = c
                .latest(&WorkflowId::from("wf-it"), &RunId::from(run.as_str()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(latest.values["a"], json!(1));
        }
    }
}

#[cfg(feature = "redis")]
pub mod redis_backend {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, RunId, Value, WorkflowId};
    use redis::aio::ConnectionManager;
    use redis::AsyncCommands;

    use crate::checkpointer::{CheckpointKey, CheckpointMeta, Checkpointer, Snapshot};

    /// Redis hot-tier checkpointer. Each snapshot is a JSON string at
    /// `ckpt:{wf}:{run}:{step}`; a per-run sorted set `ckpt-idx:{wf}:{run}`
    /// (score = super_step) backs `latest`/`list`.
    pub struct RedisCheckpointer {
        conn: ConnectionManager,
        prefix: String,
    }

    fn backend_err<E: std::fmt::Display>(e: E) -> AgentError {
        AgentError::Internal(format!("RedisCheckpointer backend error: {e}"))
    }

    impl RedisCheckpointer {
        /// Connect to Redis at `url` (e.g. `redis://127.0.0.1/`).
        pub async fn connect(url: impl AsRef<str>) -> Result<Self> {
            let client = redis::Client::open(url.as_ref()).map_err(backend_err)?;
            let conn = ConnectionManager::new(client).await.map_err(backend_err)?;
            Ok(Self {
                conn,
                prefix: "ckpt".to_string(),
            })
        }

        fn data_key(&self, k: &CheckpointKey) -> String {
            format!(
                "{}:{}:{}:{}",
                self.prefix,
                k.workflow_id.as_str(),
                k.run_id.as_str(),
                k.super_step
            )
        }
        fn index_key(&self, wf: &str, run: &str) -> String {
            format!("{}-idx:{}:{}", self.prefix, wf, run)
        }
    }

    #[async_trait]
    impl Checkpointer for RedisCheckpointer {
        async fn save(&self, snapshot: Snapshot) -> Result<()> {
            let mut conn = self.conn.clone();
            let data = serde_json::to_string(&snapshot)?;
            let dk = self.data_key(&snapshot.key);
            let ik = self.index_key(snapshot.key.workflow_id.as_str(), snapshot.key.run_id.as_str());
            let _: () = conn.set(&dk, data).await.map_err(backend_err)?;
            let _: () = conn
                .zadd(&ik, snapshot.key.super_step, snapshot.key.super_step as f64)
                .await
                .map_err(backend_err)?;
            Ok(())
        }

        async fn load(&self, key: &CheckpointKey) -> Result<Option<Snapshot>> {
            let mut conn = self.conn.clone();
            let data: Option<String> = conn.get(self.data_key(key)).await.map_err(backend_err)?;
            data.map(|d| serde_json::from_str(&d).map_err(AgentError::from))
                .transpose()
        }

        async fn latest(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Option<Snapshot>> {
            let mut conn = self.conn.clone();
            let ik = self.index_key(workflow_id.as_str(), run_id.as_str());
            let top: Vec<u64> = conn.zrevrange(&ik, 0, 0).await.map_err(backend_err)?;
            match top.first() {
                None => Ok(None),
                Some(step) => {
                    self.load(&CheckpointKey {
                        workflow_id: workflow_id.clone(),
                        run_id: run_id.clone(),
                        super_step: *step,
                    })
                    .await
                }
            }
        }

        async fn list(&self, workflow_id: &WorkflowId, run_id: &RunId) -> Result<Vec<CheckpointMeta>> {
            let mut conn = self.conn.clone();
            let ik = self.index_key(workflow_id.as_str(), run_id.as_str());
            let steps: Vec<u64> = conn.zrange(&ik, 0, -1).await.map_err(backend_err)?;
            let mut metas = Vec::with_capacity(steps.len());
            for step in steps {
                if let Some(s) = self
                    .load(&CheckpointKey {
                        workflow_id: workflow_id.clone(),
                        run_id: run_id.clone(),
                        super_step: step,
                    })
                    .await?
                {
                    metas.push(CheckpointMeta {
                        workflow_id: s.key.workflow_id,
                        run_id: s.key.run_id,
                        super_step: s.key.super_step,
                        timestamp_ms: s.timestamp_ms,
                    });
                }
            }
            Ok(metas)
        }

        async fn fork(&self, from: &CheckpointKey, edits: Vec<(String, Value)>) -> Result<RunId> {
            let snap = self.load(from).await?.ok_or_else(|| {
                AgentError::Internal(format!(
                    "fork: source checkpoint {}#{} not found",
                    from.run_id.as_str(),
                    from.super_step
                ))
            })?;
            let new_run = RunId::new();
            let mut values = snap.values.clone();
            for (k, v) in edits {
                values.insert(k, v);
            }
            self.save(Snapshot {
                key: CheckpointKey {
                    workflow_id: snap.key.workflow_id.clone(),
                    run_id: new_run.clone(),
                    super_step: snap.key.super_step,
                },
                values,
                label: format!("fork-of:{}", from.run_id.as_str()),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            })
            .await?;
            Ok(new_run)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        use std::collections::HashMap;

        // Runs only when a Redis server is configured via REDIS_URL.
        #[tokio::test]
        async fn redis_roundtrip_when_configured() {
            let Ok(url) = std::env::var("REDIS_URL") else {
                eprintln!("skipping: REDIS_URL not set");
                return;
            };
            let c = RedisCheckpointer::connect(url).await.unwrap();
            let run = format!("it-{}", RunId::new().as_str());
            let mut values = HashMap::new();
            values.insert("a".to_string(), json!(5));
            c.save(Snapshot {
                key: CheckpointKey {
                    workflow_id: WorkflowId::from("wf-it"),
                    run_id: RunId::from(run.as_str()),
                    super_step: 0,
                },
                values,
                label: "init".into(),
                timestamp_ms: 1,
            })
            .await
            .unwrap();
            let latest = c
                .latest(&WorkflowId::from("wf-it"), &RunId::from(run.as_str()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(latest.values["a"], json!(5));
        }
    }
}
