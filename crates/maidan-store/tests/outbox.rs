//! Postgres outbox integration tests.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{postgres::outbox, run_postgres_migrations, PostgresStore, Store};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping outbox tests: docker unavailable ({err})");
            return None;
        }
    };

    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;

    run_postgres_migrations(&pool).await.ok()?;
    Some((container, pool))
}

fn workspace_created_event(name: &str) -> Event {
    Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

#[tokio::test]
async fn append_enqueues_unpublished_outbox_row() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let event = workspace_created_event("outbox-ws");
    let stored = store.append_event(&event).await.unwrap();
    assert!(outbox::count_pending(&pool).await.unwrap() >= 1);

    let pending = outbox::list_pending(&pool, 8).await.unwrap();
    assert!(pending.iter().any(|row| row.log_id == stored.id));
    assert_eq!(pending[0].attempts, 0);
}

#[tokio::test]
async fn record_attempt_increments_attempts_while_row_stays_pending() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let stored = store
        .append_event(&workspace_created_event("attempts-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    let row = pending
        .into_iter()
        .find(|r| r.log_id == stored.id)
        .expect("pending row");

    outbox::record_attempt(&pool, row.id).await.unwrap();
    outbox::record_attempt(&pool, row.id).await.unwrap();

    let again = outbox::list_pending(&pool, 8).await.unwrap();
    let updated = again
        .into_iter()
        .find(|r| r.id == row.id)
        .expect("still pending");
    assert_eq!(updated.attempts, 2);
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn mark_published_clears_pending_and_rejects_unknown_id() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let stored = store
        .append_event(&workspace_created_event("published-ws"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    let row = pending
        .into_iter()
        .find(|r| r.log_id == stored.id)
        .expect("pending row");

    outbox::mark_published(&pool, row.id).await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);

    let err = outbox::mark_published(&pool, row.id).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));

    let err = outbox::mark_published(&pool, 9_999_999).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

#[tokio::test]
async fn list_pending_orders_by_id_and_respects_limit() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let first = store
        .append_event(&workspace_created_event("order-a"))
        .await
        .unwrap();
    let second = store
        .append_event(&workspace_created_event("order-b"))
        .await
        .unwrap();

    let one = outbox::list_pending(&pool, 1).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].log_id, first.id);

    let two = outbox::list_pending(&pool, 2).await.unwrap();
    assert_eq!(two.len(), 2);
    assert!(two[0].id < two[1].id);
    assert_eq!(two[1].log_id, second.id);
}

#[tokio::test]
async fn multiple_appends_enqueue_one_outbox_row_per_event() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    store
        .append_event(&workspace_created_event("multi-a"))
        .await
        .unwrap();
    store
        .append_event(&workspace_created_event("multi-b"))
        .await
        .unwrap();
    store
        .append_event(&workspace_created_event("multi-c"))
        .await
        .unwrap();

    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 3);
}
