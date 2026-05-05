//! Backend feature-flag stubs for `LongStore`. Real wiring lives in
//! deployment patches; the types are visible behind the feature flag
//! so callers can program against them today.

#[cfg(feature = "pgvector")]
pub mod pgvector {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, Value};

    use crate::long_term::{LongStore, Namespace, StoreItem};

    pub struct PgvectorStore {
        pub url: String,
    }

    impl PgvectorStore {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Ok(Self { url: url.into() })
        }
    }

    fn unsupported<T>() -> Result<T> {
        Err(AgentError::Internal(
            "PgvectorStore: backend stub. Enable in your deployment patch.".into(),
        ))
    }

    #[async_trait]
    impl LongStore for PgvectorStore {
        async fn put(
            &self,
            _namespace: &Namespace,
            _key: &str,
            _value: Value,
            _embedding: Option<Vec<f32>>,
        ) -> Result<()> {
            unsupported()
        }
        async fn get(&self, _namespace: &Namespace, _key: &str) -> Result<Option<StoreItem>> {
            unsupported()
        }
        async fn delete(&self, _namespace: &Namespace, _key: &str) -> Result<()> {
            unsupported()
        }
        async fn search(
            &self,
            _namespace: &Namespace,
            _query_embedding: Option<&[f32]>,
            _top_k: usize,
        ) -> Result<Vec<StoreItem>> {
            unsupported()
        }
        async fn list_namespaces(&self, _prefix: &Namespace) -> Result<Vec<Namespace>> {
            unsupported()
        }
    }
}

#[cfg(feature = "qdrant")]
pub mod qdrant {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, Value};

    use crate::long_term::{LongStore, Namespace, StoreItem};

    pub struct QdrantStore {
        pub url: String,
    }

    impl QdrantStore {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Ok(Self { url: url.into() })
        }
    }

    fn unsupported<T>() -> Result<T> {
        Err(AgentError::Internal(
            "QdrantStore: backend stub. Enable in your deployment patch.".into(),
        ))
    }

    #[async_trait]
    impl LongStore for QdrantStore {
        async fn put(
            &self,
            _namespace: &Namespace,
            _key: &str,
            _value: Value,
            _embedding: Option<Vec<f32>>,
        ) -> Result<()> {
            unsupported()
        }
        async fn get(&self, _namespace: &Namespace, _key: &str) -> Result<Option<StoreItem>> {
            unsupported()
        }
        async fn delete(&self, _namespace: &Namespace, _key: &str) -> Result<()> {
            unsupported()
        }
        async fn search(
            &self,
            _namespace: &Namespace,
            _query_embedding: Option<&[f32]>,
            _top_k: usize,
        ) -> Result<Vec<StoreItem>> {
            unsupported()
        }
        async fn list_namespaces(&self, _prefix: &Namespace) -> Result<Vec<Namespace>> {
            unsupported()
        }
    }
}

#[cfg(feature = "chroma")]
pub mod chroma {
    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, Value};

    use crate::long_term::{LongStore, Namespace, StoreItem};

    pub struct ChromaStore {
        pub url: String,
    }

    impl ChromaStore {
        pub async fn connect(url: impl Into<String>) -> Result<Self> {
            Ok(Self { url: url.into() })
        }
    }

    fn unsupported<T>() -> Result<T> {
        Err(AgentError::Internal(
            "ChromaStore: backend stub. Enable in your deployment patch.".into(),
        ))
    }

    #[async_trait]
    impl LongStore for ChromaStore {
        async fn put(
            &self,
            _namespace: &Namespace,
            _key: &str,
            _value: Value,
            _embedding: Option<Vec<f32>>,
        ) -> Result<()> {
            unsupported()
        }
        async fn get(&self, _namespace: &Namespace, _key: &str) -> Result<Option<StoreItem>> {
            unsupported()
        }
        async fn delete(&self, _namespace: &Namespace, _key: &str) -> Result<()> {
            unsupported()
        }
        async fn search(
            &self,
            _namespace: &Namespace,
            _query_embedding: Option<&[f32]>,
            _top_k: usize,
        ) -> Result<Vec<StoreItem>> {
            unsupported()
        }
        async fn list_namespaces(&self, _prefix: &Namespace) -> Result<Vec<Namespace>> {
            unsupported()
        }
    }
}
