//! Env-gated integration tests for the production `VectorStore` backends
//! (FR-10). These are skipped unless the relevant service URL is set:
//!
//! * pgvector: `ATOMR_IT_SQL_URL` (e.g. `postgres://user:pass@localhost/db`)
//! * redis:    `REDIS_URL` (e.g. `redis://127.0.0.1/`)
//!
//! Run with: `cargo test -p atomr-agents-memory --features pgvector,redis`.

#![cfg(any(feature = "pgvector", feature = "redis"))]

#[cfg(feature = "pgvector")]
#[tokio::test]
async fn pgvector_roundtrip_upsert_query_delete() {
    use atomr_agents_memory::{MetadataFilter, PgvectorVectorStore, VectorStore};

    let Ok(url) = std::env::var("ATOMR_IT_SQL_URL") else {
        eprintln!("skipping pgvector IT: ATOMR_IT_SQL_URL not set");
        return;
    };
    let table = format!("agents_vectors_it_{}", std::process::id());
    let store = PgvectorVectorStore::connect_table(&url, &table)
        .await
        .expect("connect");
    store
        .upsert(vec![
            ("a".into(), vec![1.0, 0.0], serde_json::json!({"lang": "rust"})),
            ("b".into(), vec![0.0, 1.0], serde_json::json!({"lang": "python"})),
        ])
        .await
        .expect("upsert");
    let hits = store
        .query(
            vec![1.0, 0.0],
            5,
            &MetadataFilter::Eq("lang".into(), serde_json::json!("rust")),
        )
        .await
        .expect("query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "a");

    store.delete(vec!["a".into(), "b".into()]).await.expect("delete");
    let hits = store.query(vec![1.0, 0.0], 5, &MetadataFilter::All).await.expect("query2");
    assert!(hits.is_empty());
}

#[cfg(feature = "redis")]
#[tokio::test]
async fn redis_roundtrip_upsert_query_delete() {
    use atomr_agents_memory::{MetadataFilter, RedisVectorStore, VectorStore};

    let Ok(url) = std::env::var("REDIS_URL") else {
        eprintln!("skipping redis IT: REDIS_URL not set");
        return;
    };
    let prefix = format!("agents:vec:it:{}", std::process::id());
    let store = RedisVectorStore::connect_prefix(&url, &prefix).await.expect("connect");
    store
        .upsert(vec![
            ("a".into(), vec![1.0, 0.0], serde_json::json!({"lang": "rust"})),
            ("b".into(), vec![0.0, 1.0], serde_json::json!({"lang": "python"})),
        ])
        .await
        .expect("upsert");
    let hits = store
        .query(
            vec![1.0, 0.0],
            5,
            &MetadataFilter::Eq("lang".into(), serde_json::json!("rust")),
        )
        .await
        .expect("query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "a");

    store.delete(vec!["a".into(), "b".into()]).await.expect("delete");
    let hits = store.query(vec![1.0, 0.0], 5, &MetadataFilter::All).await.expect("query2");
    assert!(hits.is_empty());
}
