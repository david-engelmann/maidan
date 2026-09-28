//! Shared helpers for `maidan-server` integration tests.

use std::time::Duration;

use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Starts `pgvector/pgvector:pg17` and runs Maidan Postgres migrations.
pub async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };

    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");

    maidan_store::run_postgres_migrations(&pool)
        .await
        .expect("migrate");
    Some((container, pool))
}
