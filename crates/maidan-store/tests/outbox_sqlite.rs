//! SQLite outbox integration tests.

use chrono::Utc;
use maidan_store::{run_sqlite_migrations, sqlite::outbox, SqliteStore, Store};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    pool
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
    let pool = sqlite_pool().await;
    let store = SqliteStore::new(pool.clone());
    store
        .append_event(&workspace_created_event("sqlite-outbox-ws"))
        .await
        .unwrap();
    assert!(outbox::count_pending(&pool).await.unwrap() >= 1);
}

#[tokio::test]
async fn quarantined_rows_are_excluded_from_pending_list_and_count() {
    let pool = sqlite_pool().await;
    let store = SqliteStore::new(pool.clone());
    store
        .append_event(&workspace_created_event("sqlite-quarantine"))
        .await
        .unwrap();
    let pending = outbox::list_pending(&pool, 1).await.unwrap();
    outbox::quarantine(&pool, pending[0].id).await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);
}
