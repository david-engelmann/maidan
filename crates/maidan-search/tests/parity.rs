//! Cross-dialect parity for lexical search. Same seed, same query,
//! asserts the result message-id sets match between Postgres and SQLite.

mod common;

use std::{sync::Arc, time::Duration};

use maidan_search::{PostgresSearch, Search, SqliteSearch};
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, PostgresStore, SqliteStore, Store,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn rust_query_returns_same_ids() {
    let container = match Postgres::default().with_tag("17-alpine").start().await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping parity: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pg_pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pg_pool).await.unwrap();
    let pg_store: Arc<dyn Store> = Arc::new(PostgresStore::new(pg_pool.clone()));
    let pg_search = PostgresSearch::new(pg_pool);

    let sqlite_pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&sqlite_pool)
        .await
        .unwrap();
    run_sqlite_migrations(&sqlite_pool).await.unwrap();
    let sqlite_store: Arc<dyn Store> = Arc::new(SqliteStore::new(sqlite_pool.clone()));
    let sqlite_search = SqliteSearch::new(sqlite_pool);

    let pg_fx = common::seed(&*pg_store).await;
    let sqlite_fx = common::seed(&*sqlite_store).await;

    let pg_hits = pg_search
        .search_messages(pg_fx.workspace_id, "rust", 10)
        .await
        .unwrap();
    let sqlite_hits = sqlite_search
        .search_messages(sqlite_fx.workspace_id, "rust", 10)
        .await
        .unwrap();

    // ids differ between fixtures, so compare by body instead.
    let mut pg_bodies: Vec<String> = pg_hits.iter().map(|h| h.body.clone()).collect();
    let mut sqlite_bodies: Vec<String> = sqlite_hits.iter().map(|h| h.body.clone()).collect();
    pg_bodies.sort();
    sqlite_bodies.sort();
    assert_eq!(pg_bodies, sqlite_bodies);
}
