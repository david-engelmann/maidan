//! Integration test for the SQLite backend, mirror of postgres_roundtrip.

mod common;

use maidan_store::{run_sqlite_migrations, SqliteStore};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory");
    // Enable foreign keys (SQLite default-off).
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    run_sqlite_migrations(&pool).await.expect("migrate sqlite");
    SqliteStore::new(pool)
}

#[tokio::test]
async fn full_roundtrip() {
    let store = spawn().await;
    common::run_full_roundtrip(&store).await;
}

#[tokio::test]
async fn channel_members_roundtrip() {
    let store = spawn().await;
    common::run_channel_members_scenario(&store).await;
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let store = spawn().await;
    run_sqlite_migrations(store.pool())
        .await
        .expect("re-apply sqlite migrations");
}
