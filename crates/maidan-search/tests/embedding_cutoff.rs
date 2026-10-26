//! The hard allowance cutoff for embeddings, on both backends: a workspace
//! cut off on `embedding_calls` has its new messages stored but not queued
//! for embedding or counted, while another workspace's messages are embedded
//! and counted as before.

use std::sync::atomic::Ordering;
use std::{sync::Arc, time::Duration};

use maidan_search::{
    BatchConfig, BatchingEmbeddingHandler, EventHandler, HashV1Provider, IndexerMetrics,
    PostgresSearch, Search, SqliteSearch,
};
use maidan_store::usage_meters::UsageMeterBuffer;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    Event, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, UsageMeter,
    WorkspaceId,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn posted(store: &dyn Store, name: &str) -> (WorkspaceId, Event) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let message = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: author.id,
            body: format!("hello from {name}"),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let event = Event::MessagePosted {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws.id,
        channel_id: ch.id,
        thread_id: thread.id,
        dm_conversation_id: None,
        message,
        sealed: None,
    };
    (ws.id, event)
}

async fn run_suite(store: Arc<dyn Store>, search: Arc<dyn Search>) {
    let config = BatchConfig {
        queue_capacity: 16,
        batch_size: 4,
        retry: maidan_search::RetryPolicy::none(),
    };
    let metrics = Arc::new(IndexerMetrics::new(config.queue_capacity));
    let meters = Arc::new(UsageMeterBuffer::new());
    let handler = BatchingEmbeddingHandler::spawn(
        store.clone(),
        search,
        Arc::new(HashV1Provider),
        config,
        metrics.clone(),
        None,
    )
    .with_usage_meters(meters.clone());

    let (cut, cut_event) = posted(store.as_ref(), "cut").await;
    let (open, open_event) = posted(store.as_ref(), "open").await;
    meters.set_cut_off([(cut, UsageMeter::EmbeddingCalls)]);

    handler.handle(&cut_event).await;
    handler.handle(&open_event).await;

    let mut embedded = 0;
    for _ in 0..60 {
        embedded = metrics.embedded_total.load(Ordering::Relaxed);
        if embedded >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        metrics.embedded_total.load(Ordering::Relaxed),
        1,
        "only the workspace inside its allowance is embedded (saw {embedded})"
    );
    let counted = meters.take();
    assert!(
        counted
            .iter()
            .all(|d| d.workspace_id != cut || d.meter != UsageMeter::EmbeddingCalls),
        "the cut-off workspace is not counted: {counted:?}"
    );
    assert!(
        counted.iter().any(|d| d.workspace_id == open
            && d.meter == UsageMeter::EmbeddingCalls
            && d.quantity == 1),
        "the other workspace is counted as before: {counted:?}"
    );
}

#[tokio::test]
async fn a_cut_off_workspace_is_not_embedded_while_another_is_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn Search> = Arc::new(SqliteSearch::new(pool));
    run_suite(store, search).await;
}

#[tokio::test]
async fn a_cut_off_workspace_is_not_embedded_while_another_is_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool));
    run_suite(store, search).await;
}
