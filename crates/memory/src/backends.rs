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

    // ----------------------------------------------------------------
    // PgvectorVectorStore — real `VectorStore` over Postgres via sqlx
    // (FR-10).
    //
    // Approach: to stay portable across the sqlx `any` driver (which does
    // not expose the pgvector `vector` type or its `<=>` operator), the
    // embedding is stored as a JSON-encoded `text` column and cosine
    // similarity is computed in Rust over the candidate rows. Equality
    // `MetadataFilter` predicates are pushed into the SQL `WHERE` clause
    // (JSON-extracted as text); range predicates are applied in Rust
    // after fetch. This trades the pgvector ANN index for portability and
    // correctness; a deployment wanting ANN can swap the `query` body for
    // a native `vector` column + `<=>` operator without changing the
    // public contract.
    // ----------------------------------------------------------------
    use crate::vector_store::{cosine, Hit, MetadataFilter, VectorStore};
    use sqlx::any::AnyPoolOptions;
    use sqlx::{AnyPool, Row};

    pub struct PgvectorVectorStore {
        pool: AnyPool,
        table: String,
    }

    impl PgvectorVectorStore {
        /// Connect and ensure the backing table exists. `table` defaults
        /// to `agents_vectors` via [`PgvectorVectorStore::connect`].
        pub async fn connect_table(url: &str, table: &str) -> Result<Self> {
            sqlx::any::install_default_drivers();
            let pool = AnyPoolOptions::new()
                .max_connections(5)
                .connect(url)
                .await
                .map_err(|e| AgentError::Internal(format!("pgvector connect: {e}")))?;
            let ddl = format!(
                "CREATE TABLE IF NOT EXISTS {table} (id TEXT PRIMARY KEY, embedding TEXT NOT NULL, metadata TEXT NOT NULL)"
            );
            sqlx::query(&ddl)
                .execute(&pool)
                .await
                .map_err(|e| AgentError::Internal(format!("pgvector ddl: {e}")))?;
            Ok(Self {
                pool,
                table: table.to_string(),
            })
        }

        pub async fn connect(url: &str) -> Result<Self> {
            Self::connect_table(url, "agents_vectors").await
        }
    }

    #[async_trait]
    impl VectorStore for PgvectorVectorStore {
        async fn upsert(&self, items: Vec<(String, Vec<f32>, Value)>) -> Result<()> {
            for (id, emb, meta) in items {
                let emb_s = serde_json::to_string(&emb)
                    .map_err(|e| AgentError::Internal(format!("encode embedding: {e}")))?;
                let meta_s = serde_json::to_string(&meta)
                    .map_err(|e| AgentError::Internal(format!("encode metadata: {e}")))?;
                // Portable upsert: delete-then-insert (works on the `any` driver).
                let del = format!("DELETE FROM {} WHERE id = $1", self.table);
                sqlx::query(&del)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AgentError::Internal(format!("pgvector delete-on-upsert: {e}")))?;
                let ins = format!(
                    "INSERT INTO {} (id, embedding, metadata) VALUES ($1, $2, $3)",
                    self.table
                );
                sqlx::query(&ins)
                    .bind(&id)
                    .bind(&emb_s)
                    .bind(&meta_s)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AgentError::Internal(format!("pgvector insert: {e}")))?;
            }
            Ok(())
        }

        async fn query(&self, embedding: Vec<f32>, k: usize, filter: &MetadataFilter) -> Result<Vec<Hit>> {
            // Fetch candidate rows; equality predicates are honored in Rust
            // over the decoded metadata (the `any` driver lacks portable
            // JSON operators), then cosine is computed in Rust.
            let sql = format!("SELECT id, embedding, metadata FROM {}", self.table);
            let rows = sqlx::query(&sql)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AgentError::Internal(format!("pgvector query: {e}")))?;
            let mut hits: Vec<Hit> = Vec::new();
            for row in rows {
                let id: String = row.get("id");
                let emb_s: String = row.get("embedding");
                let meta_s: String = row.get("metadata");
                let emb: Vec<f32> = serde_json::from_str(&emb_s).unwrap_or_default();
                let meta: Value = serde_json::from_str(&meta_s).unwrap_or(Value::Null);
                if !filter.matches(&meta) {
                    continue;
                }
                hits.push(Hit {
                    id,
                    score: cosine(&embedding, &emb),
                    metadata: meta,
                });
            }
            hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            hits.truncate(k);
            Ok(hits)
        }

        async fn delete(&self, ids: Vec<String>) -> Result<()> {
            for id in ids {
                let del = format!("DELETE FROM {} WHERE id = $1", self.table);
                sqlx::query(&del)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AgentError::Internal(format!("pgvector delete: {e}")))?;
            }
            Ok(())
        }
    }
}

#[cfg(feature = "redis")]
pub mod redis {
    //! Redis-backed [`VectorStore`] (FR-10). The hot tier stores each
    //! vector + metadata as a Redis hash under `{prefix}:{id}` and tracks
    //! the id set under `{prefix}:ids`. `query` performs an **exact**
    //! cosine scan over the stored vectors (documented hot-tier behavior:
    //! exact, not ANN) and honors [`MetadataFilter`] in Rust.

    use async_trait::async_trait;
    use atomr_agents_core::{AgentError, Result, Value};
    use redis::AsyncCommands;

    use crate::vector_store::{cosine, Hit, MetadataFilter, VectorStore};

    pub struct RedisVectorStore {
        client: redis::Client,
        prefix: String,
    }

    impl RedisVectorStore {
        pub async fn connect(url: &str) -> Result<Self> {
            Self::connect_prefix(url, "agents:vec").await
        }

        pub async fn connect_prefix(url: &str, prefix: &str) -> Result<Self> {
            let client = redis::Client::open(url)
                .map_err(|e| AgentError::Internal(format!("redis open: {e}")))?;
            // Validate connectivity eagerly.
            let _ = client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| AgentError::Internal(format!("redis connect: {e}")))?;
            Ok(Self {
                client,
                prefix: prefix.to_string(),
            })
        }

        fn key(&self, id: &str) -> String {
            format!("{}:{}", self.prefix, id)
        }

        fn ids_key(&self) -> String {
            format!("{}:ids", self.prefix)
        }
    }

    #[async_trait]
    impl VectorStore for RedisVectorStore {
        async fn upsert(&self, items: Vec<(String, Vec<f32>, Value)>) -> Result<()> {
            let mut con = self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| AgentError::Internal(format!("redis conn: {e}")))?;
            for (id, emb, meta) in items {
                let emb_s = serde_json::to_string(&emb)
                    .map_err(|e| AgentError::Internal(format!("encode embedding: {e}")))?;
                let meta_s = serde_json::to_string(&meta)
                    .map_err(|e| AgentError::Internal(format!("encode metadata: {e}")))?;
                let key = self.key(&id);
                let _: () = con
                    .hset_multiple(&key, &[("embedding", emb_s), ("metadata", meta_s)])
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis hset: {e}")))?;
                let _: () = con
                    .sadd(self.ids_key(), &id)
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis sadd: {e}")))?;
            }
            Ok(())
        }

        async fn query(&self, embedding: Vec<f32>, k: usize, filter: &MetadataFilter) -> Result<Vec<Hit>> {
            let mut con = self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| AgentError::Internal(format!("redis conn: {e}")))?;
            let ids: Vec<String> = con
                .smembers(self.ids_key())
                .await
                .map_err(|e| AgentError::Internal(format!("redis smembers: {e}")))?;
            let mut hits: Vec<Hit> = Vec::new();
            for id in ids {
                let key = self.key(&id);
                let emb_opt: Option<String> = con
                    .hget(&key, "embedding")
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis hget: {e}")))?;
                let emb_s = match emb_opt {
                    Some(s) => s,
                    None => continue,
                };
                let meta_s: String = con
                    .hget(&key, "metadata")
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis hget: {e}")))?;
                let emb: Vec<f32> = serde_json::from_str(&emb_s).unwrap_or_default();
                let meta: Value = serde_json::from_str(&meta_s).unwrap_or(Value::Null);
                if !filter.matches(&meta) {
                    continue;
                }
                hits.push(Hit {
                    id,
                    score: cosine(&embedding, &emb),
                    metadata: meta,
                });
            }
            hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            hits.truncate(k);
            Ok(hits)
        }

        async fn delete(&self, ids: Vec<String>) -> Result<()> {
            let mut con = self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|e| AgentError::Internal(format!("redis conn: {e}")))?;
            for id in ids {
                let _: () = con
                    .del(self.key(&id))
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis del: {e}")))?;
                let _: () = con
                    .srem(self.ids_key(), &id)
                    .await
                    .map_err(|e| AgentError::Internal(format!("redis srem: {e}")))?;
            }
            Ok(())
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
