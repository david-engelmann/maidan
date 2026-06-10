//! Cross-process resource-notification fan-out over Postgres LISTEN/NOTIFY
//! (Cluster 102). Two `PostgresResourceNotifier`s on the same database stand
//! in for two server replicas: a `publish_uris` on one must reach a subscriber
//! on the other.

use std::time::Duration;

use maidan_bus::{PostgresResourceNotifier, ResourceNotifier};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn pg_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping resource_notify_pg: docker unavailable ({err})");
            return None;
        }
    };

    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // No migrations needed: the notifier only uses pg_notify / LISTEN.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;
    Some((container, pool))
}

#[tokio::test]
async fn resource_uris_fan_out_across_replicas() {
    let Some((_container, pool)) = pg_pool().await else {
        return;
    };

    // Two notifiers on one database = two replicas behind a load balancer.
    let replica_a = PostgresResourceNotifier::connect(pool.clone())
        .await
        .unwrap();
    let replica_b = PostgresResourceNotifier::connect(pool.clone())
        .await
        .unwrap();
    // Let both LISTEN tasks attach before publishing.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub_b = replica_b.subscribe();

    replica_a
        .publish_uris(vec![
            "maidan://threads/t1".into(),
            "maidan://workspaces/w1".into(),
        ])
        .await
        .unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout waiting for cross-replica resource notify")
        .expect("channel closed");

    assert!(got.contains(&"maidan://threads/t1".to_string()));
    assert!(got.contains(&"maidan://workspaces/w1".to_string()));
}
