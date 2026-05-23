//! Cross-dialect parity: run the same sequence of operations against
//! Postgres and SQLite and assert the user-visible result is identical.
//!
//! Identity is checked at the domain level (handles, names, bodies,
//! metadata, ordering) — auto-generated ids and timestamps differ by
//! construction and are excluded from the snapshot comparison.

mod common;

use std::time::Duration;

use maidan_store::{run_postgres_migrations, run_sqlite_migrations, PostgresStore, SqliteStore};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn parity_between_postgres_and_sqlite() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping dialect_parity: docker unavailable ({err})");
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
        .expect("connect postgres");
    run_postgres_migrations(&pg_pool).await.expect("pg migrate");
    let pg = PostgresStore::new(pg_pool);

    let sqlite_pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&sqlite_pool)
        .await
        .unwrap();
    run_sqlite_migrations(&sqlite_pool)
        .await
        .expect("sqlite migrate");
    let sqlite = SqliteStore::new(sqlite_pool);

    let pg_snap = common::run_parity_scenario(&pg).await;
    let sqlite_snap = common::run_parity_scenario(&sqlite).await;

    // ids differ by construction (UUID v4) — strip them before comparing.
    let mut pg_cmp = pg_snap;
    let mut sqlite_cmp = sqlite_snap;
    pg_cmp.message_ids.clear();
    sqlite_cmp.message_ids.clear();
    assert_eq!(pg_cmp, sqlite_cmp);
}
