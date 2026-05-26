//! Postgres `websearch_to_tsquery` operator pass-through (v1.2.3).

mod common;

use std::{sync::Arc, time::Duration};

use maidan_search::{PostgresSearch, Search, SearchFilters};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn postgres_websearch_operator_pass_through() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping websearch test: docker unavailable ({err})");
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
    let filters = SearchFilters::default();

    let plain = search
        .search_messages(fx.workspace_id, "rust", 10, &filters)
        .await
        .unwrap();
    assert_eq!(plain.len(), 4);

    let minus = search
        .search_messages(fx.workspace_id, "rust -tokio", 10, &filters)
        .await
        .unwrap();
    assert_eq!(minus.len(), plain.len() - 1);
    assert!(
        !minus.iter().any(|h| h.body.contains("tokio")),
        "minus tokio: {:?}",
        minus.iter().map(|h| &h.body).collect::<Vec<_>>()
    );

    let hits = search
        .search_messages(
            fx.workspace_id,
            "\"ferris the unofficial\"",
            10,
            &filters,
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].body.contains("ferris"));

    let hits = search
        .search_messages(fx.workspace_id, "ferris or go", 10, &filters)
        .await
        .unwrap();
    assert!(hits.len() >= 2);
    assert!(hits.iter().any(|h| h.body.contains("ferris")));
    assert!(hits.iter().any(|h| h.body.contains("go")));
}
