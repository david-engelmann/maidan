//! pgvector-backed embedding round-trip + semantic ranking.

mod common;

use std::{sync::Arc, time::Duration};

use maidan_search::{postgres::EMBEDDING_DIM, PostgresSearch, Search, SearchError, SearchFilters};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{MessageId, NewMessage};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// One-hot-style vector with a 1.0 at the given index, 0.0 elsewhere.
/// Cosine distance between two such vectors is 0 if the indices match,
/// 1 otherwise — perfect for asserting deterministic ordering.
fn one_hot(index: usize) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[index] = 1.0;
    v
}

#[tokio::test]
async fn semantic_search_orders_by_cosine_distance() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping embeddings test: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();

    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search = PostgresSearch::new(pool);
    let fx = common::seed(&*store).await;

    // Seeded fixture has six messages; we pick three to embed at known
    // axes so semantic_search returns a deterministic order.
    let alive_ids: Vec<MessageId> = fx
        .message_ids
        .iter()
        .copied()
        .filter(|id| *id != fx.tombstoned)
        .collect();
    assert!(alive_ids.len() >= 3, "fixture should provide ≥3 messages");

    for (i, id) in alive_ids.iter().take(3).enumerate() {
        search
            .upsert_embedding(*id, "test-model", &one_hot(i))
            .await
            .unwrap();
    }

    // Querying near axis 1 ranks message-1 first, then 0/2 close behind
    // (cosine distance 1.0 each).
    let query = one_hot(1);
    let hits = search
        .semantic_search(
            fx.workspace_id,
            &query,
            3,
            &SearchFilters::default(),
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(
        hits[0].message_id, alive_ids[1],
        "top hit should be the axis-1 message"
    );
    // top hit has cosine distance 0 → rank 1.0; others have distance 1 → rank 0.0
    assert!((hits[0].rank - 1.0).abs() < 1e-6);
    assert!(hits[1].rank.abs() < 1e-6);
}

#[tokio::test]
async fn upsert_replaces_existing_embedding() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping embeddings test: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search = PostgresSearch::new(pool);

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "embed".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(maidan_types::NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: ws.id,
            name: "ch".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(maidan_types::NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "single message".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // First upsert at axis 0.
    search
        .upsert_embedding(msg.id, "v1", &one_hot(0))
        .await
        .unwrap();
    let hits = search
        .semantic_search(ws.id, &one_hot(0), 1, &SearchFilters::default(), "v1")
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!((hits[0].rank - 1.0).abs() < 1e-6);
    assert_eq!(hits[0].embedding_model.as_deref(), Some("v1"));

    // A second model on the same message lands in its own table (Cluster 47).
    search
        .upsert_embedding(msg.id, "v2", &one_hot(1))
        .await
        .unwrap();
    let v1_hits = search
        .semantic_search(ws.id, &one_hot(0), 1, &SearchFilters::default(), "v1")
        .await
        .unwrap();
    assert_eq!(v1_hits.len(), 1, "v1 table still holds the v1 embedding");

    let v2_hits = search
        .semantic_search(ws.id, &one_hot(1), 1, &SearchFilters::default(), "v2")
        .await
        .unwrap();
    assert_eq!(v2_hits.len(), 1);
    assert!(
        (v2_hits[0].rank - 1.0).abs() < 1e-6,
        "expected near-perfect rank for v2 query on v2 embedding"
    );

    // Re-upsert within the same model replaces that table's row.
    search
        .upsert_embedding(msg.id, "v1", &one_hot(2))
        .await
        .unwrap();
    let replaced = search
        .semantic_search(ws.id, &one_hot(0), 1, &SearchFilters::default(), "v1")
        .await
        .unwrap();
    assert!(
        replaced.is_empty() || replaced[0].rank < 0.5,
        "v1 embedding should no longer align with axis 0"
    );
    let hits = search
        .semantic_search(ws.id, &one_hot(2), 1, &SearchFilters::default(), "v1")
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
        (hits[0].rank - 1.0).abs() < 1e-6,
        "expected near-perfect rank after in-model replace"
    );
}

#[tokio::test]
async fn rejects_wrong_dimension() {
    // No docker dependency; the dim check fires before any SQL runs.
    // But we still need a pool to construct PostgresSearch; use a known-
    // bad URL and assert the dim error comes back before the connection
    // is touched. To keep the test hermetic, we use a minimal SqlitePool
    // wrapped in a no-op (impossible — PostgresSearch takes PgPool). So
    // we accept that this test needs Docker too.
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping embeddings test: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    let search = PostgresSearch::new(pool);

    let err = search
        .upsert_embedding(MessageId(uuid::Uuid::new_v4()), "wrong", &[0.0; 10])
        .await
        .unwrap_err();
    assert!(matches!(err, SearchError::InvalidQuery(_)));
}

#[tokio::test]
async fn semantic_search_respects_author_channel_and_kind_facets() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping embeddings test: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();

    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search = PostgresSearch::new(pool);
    let fx = common::seed(&*store).await;

    let general_msg = fx.message_ids[1];
    let release_msg = fx.message_ids[5];
    let axis = one_hot(0);
    search
        .upsert_embedding(general_msg, "test-model", &axis)
        .await
        .unwrap();
    search
        .upsert_embedding(release_msg, "test-model", &axis)
        .await
        .unwrap();

    let both = search
        .semantic_search(
            fx.workspace_id,
            &axis,
            10,
            &SearchFilters::default(),
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(both.len(), 2);

    let general_only = search
        .semantic_search(
            fx.workspace_id,
            &axis,
            10,
            &SearchFilters {
                channel_id: Some(fx.general_channel_id),
                ..SearchFilters::default()
            },
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(general_only.len(), 1);
    assert_eq!(general_only[0].message_id, general_msg);

    let human_only = search
        .semantic_search(
            fx.workspace_id,
            &axis,
            10,
            &SearchFilters {
                author_kind: Some(maidan_types::MemberKind::Human),
                ..SearchFilters::default()
            },
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(human_only.len(), 1);
    assert_eq!(human_only[0].message_id, general_msg);

    let agent_only = search
        .semantic_search(
            fx.workspace_id,
            &axis,
            10,
            &SearchFilters {
                author_kind: Some(maidan_types::MemberKind::Agent),
                ..SearchFilters::default()
            },
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(agent_only.len(), 1);
    assert_eq!(agent_only[0].message_id, release_msg);
}
