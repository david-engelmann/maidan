//! PostgresSearch integration test.

mod common;

use std::{sync::Arc, time::Duration};

use maidan_search::PostgresSearch;
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn full_text_search_against_postgres() {
    let container = match Postgres::default().start().await {
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
}
