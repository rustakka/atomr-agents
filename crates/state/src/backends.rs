//! Backend feature-flag stubs for `Checkpointer`. Real wiring (sqlx
//! / postgres) lands in deployment work; the types and constructors
//! are visible behind the feature flag so callers can program against
//! them today.

#[cfg(feature = "sqlite")]
pub mod sqlite {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, RunId, WorkflowId};

    use crate::checkpointer::{
        CheckpointKey, CheckpointMeta, Checkpointer, Snapshot,
    };

    /// SQLite-backed checkpointer.
    ///
    /// **Stub**: wire-up to `sqlx` lives in a deployment patch. Until
    /// then this returns `AgentError::Internal` on every call so a
    /// caller targeting the feature flag fails loudly instead of
    /// silently falling back.
    pub struct SqliteCheckpointer {
        pub url: String,
    }

    impl SqliteCheckpointer {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Ok(Self { url: url.into() })
        }
    }

    fn unsupported<T>() -> Result<T> {
        Err(AgentError::Internal(
            "SqliteCheckpointer: backend stub. Enable the real implementation in your deployment patch.".into(),
        ))
    }

    #[async_trait]
    impl Checkpointer for SqliteCheckpointer {
        async fn save(&self, _snapshot: Snapshot) -> Result<()> {
            unsupported()
        }
        async fn load(&self, _key: &CheckpointKey) -> Result<Option<Snapshot>> {
            unsupported()
        }
        async fn latest(
            &self,
            _workflow_id: &WorkflowId,
            _run_id: &RunId,
        ) -> Result<Option<Snapshot>> {
            unsupported()
        }
        async fn list(
            &self,
            _workflow_id: &WorkflowId,
            _run_id: &RunId,
        ) -> Result<Vec<CheckpointMeta>> {
            unsupported()
        }
        async fn fork(
            &self,
            _from: &CheckpointKey,
            _edits: Vec<(String, atomr_agents_core::Value)>,
        ) -> Result<RunId> {
            unsupported()
        }
    }
}

#[cfg(feature = "postgres")]
pub mod postgres {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, RunId, WorkflowId};

    use crate::checkpointer::{
        CheckpointKey, CheckpointMeta, Checkpointer, Snapshot,
    };

    pub struct PostgresCheckpointer {
        pub url: String,
    }

    impl PostgresCheckpointer {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Ok(Self { url: url.into() })
        }
    }

    fn unsupported<T>() -> Result<T> {
        Err(AgentError::Internal(
            "PostgresCheckpointer: backend stub. Enable the real implementation in your deployment patch.".into(),
        ))
    }

    #[async_trait]
    impl Checkpointer for PostgresCheckpointer {
        async fn save(&self, _snapshot: Snapshot) -> Result<()> {
            unsupported()
        }
        async fn load(&self, _key: &CheckpointKey) -> Result<Option<Snapshot>> {
            unsupported()
        }
        async fn latest(
            &self,
            _workflow_id: &WorkflowId,
            _run_id: &RunId,
        ) -> Result<Option<Snapshot>> {
            unsupported()
        }
        async fn list(
            &self,
            _workflow_id: &WorkflowId,
            _run_id: &RunId,
        ) -> Result<Vec<CheckpointMeta>> {
            unsupported()
        }
        async fn fork(
            &self,
            _from: &CheckpointKey,
            _edits: Vec<(String, atomr_agents_core::Value)>,
        ) -> Result<RunId> {
            unsupported()
        }
    }
}
