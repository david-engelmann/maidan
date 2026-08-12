//! PostgresSearch integration test.

mod common;

use std::{sync::Arc, time::Duration};

use maidan_search::{HnswParams, PostgresSearch, Search, SearchFilters};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn full_text_search_against_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping postgres_search: docker unavailable ({err})");
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
    common::run_search_suite(&search, &fx).await;
    common::assert_faceted_search(&search, &fx).await;
    common::assert_deny_channels_filter(&search, &fx).await;
}

#[tokio::test]
async fn configured_hnsw_build_params_and_ef_search() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping hnsw params test: docker unavailable ({err})");
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
    let search = PostgresSearch::new(pool.clone()).with_hnsw(HnswParams {
        m: Some(8),
        ef_construction: Some(32),
        ef_search: Some(64),
    });

    let ws = store
        .create_workspace(NewWorkspace {
            name: "hnsw".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: member.id,
            body: "hnsw target".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    // Use a fresh model (not the migration-seeded `hash-v1`) so the per-model
    // HNSW index is created *here* with the configured build params.
    let mut embedding = vec![0.0f32; 1024];
    embedding[0] = 1.0;
    search
        .upsert_embedding(msg.id, "tuned-v1", &embedding)
        .await
        .unwrap();

    // The index DDL carries the build params (pgvector renders them in indexdef).
    let indexdef: String =
        sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE indexname = $1")
            .bind("idx_maidan_emb_tuned_v1_hnsw")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(indexdef.contains("m='8'"), "indexdef missing m: {indexdef}");
    assert!(
        indexdef.contains("ef_construction='32'"),
        "indexdef missing ef_construction: {indexdef}"
    );

    // Semantic query exercises the `SET LOCAL hnsw.ef_search` transaction path.
    let hits = search
        .semantic_search(ws.id, &embedding, 5, &SearchFilters::default(), "tuned-v1")
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id, msg.id);
}
