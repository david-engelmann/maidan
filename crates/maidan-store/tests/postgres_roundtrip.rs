//! Integration test for the Postgres backend.
//!
//! Spins up a real Postgres testcontainer, applies migrations, then
//! delegates to the shared backend-agnostic suite in `common::`. Skips
//! gracefully if Docker is not available.

mod common;

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn spawn() -> Option<(PostgresStore, testcontainers::ContainerAsync<Postgres>)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping postgres_roundtrip: docker unavailable ({err})");
            return None;
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
        .expect("connect to testcontainer postgres");
    run_postgres_migrations(&pool)
        .await
        .expect("apply migrations");
    Some((PostgresStore::new(pool), container))
}

#[tokio::test]
async fn full_roundtrip() {
    let Some((store, _container)) = spawn().await else {
        return;
    };
    common::run_full_roundtrip(&store).await;
}

#[tokio::test]
async fn channel_members_roundtrip() {
    let Some((store, _container)) = spawn().await else {
        return;
    };
    common::run_channel_members_scenario(&store).await;
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let Some((store, _container)) = spawn().await else {
        return;
    };
    run_postgres_migrations(store.pool())
        .await
        .expect("re-apply migrations");
}
