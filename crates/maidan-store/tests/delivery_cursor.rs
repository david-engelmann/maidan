//! Postgres delivery cursor integration tests.

use std::time::Duration;

use maidan_store::{postgres::delivery_cursor, run_postgres_migrations, PostgresStore, Store};
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
            eprintln!("skipping delivery_cursor tests: docker unavailable ({err})");
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

#[tokio::test]
async fn advance_cursor_is_monotonic_and_get_returns_watermark() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cursor-ws".into(),
        })
        .await
        .unwrap();

    assert_eq!(
        delivery_cursor::get_cursor(&pool, "agent-a", ws.id)
            .await
            .unwrap(),
        0
    );

    assert_eq!(
        delivery_cursor::advance_cursor(&pool, "agent-a", ws.id, 10)
            .await
            .unwrap(),
        10
    );
    assert_eq!(
        delivery_cursor::advance_cursor(&pool, "agent-a", ws.id, 5)
            .await
            .unwrap(),
        10
    );
    assert_eq!(
        delivery_cursor::advance_cursor(&pool, "agent-a", ws.id, 42)
            .await
            .unwrap(),
        42
    );
    assert_eq!(
        store.get_delivery_cursor("agent-a", ws.id).await.unwrap(),
        42
    );
}

#[tokio::test]
async fn federation_style_consumer_ids_are_scoped_per_peer() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let ws = store
        .create_workspace(NewWorkspace {
            name: "fed-style-ws".into(),
        })
        .await
        .unwrap();

    let peer_a = uuid::Uuid::new_v4();
    let peer_b = uuid::Uuid::new_v4();
    let id_a = format!("federation:{peer_a}");
    let id_b = format!("federation:{peer_b}");

    delivery_cursor::advance_cursor(&pool, &id_a, ws.id, 7)
        .await
        .unwrap();
    delivery_cursor::advance_cursor(&pool, &id_b, ws.id, 3)
        .await
        .unwrap();

    assert_eq!(
        delivery_cursor::get_cursor(&pool, &id_a, ws.id)
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        delivery_cursor::get_cursor(&pool, &id_b, ws.id)
            .await
            .unwrap(),
        3
    );
}
