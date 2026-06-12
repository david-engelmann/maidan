//! Concurrent boot-time migrations (Cluster 105): several replicas starting
//! against one fresh Postgres database must all succeed. Without the advisory
//! lock around `run_postgres_migrations`, they race on non-transactional DDL
//! (concurrent `CREATE EXTENSION` → `pg_extension` unique violation).

use std::time::Duration;

use maidan_store::run_postgres_migrations;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn concurrent_boot_migrations_all_succeed() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping concurrent migrations: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    // One pool per replica; the migrations are driven concurrently with join!
    // (same task, no Send bound) but hit Postgres on independent connections, so
    // their non-transactional DDL genuinely races at the database level — which
    // the advisory lock must serialize.
    async fn pool(url: &str) -> sqlx::PgPool {
        PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(30))
            .connect(url)
            .await
            .unwrap()
    }
    let (p0, p1, p2, p3) = (
        pool(&url).await,
        pool(&url).await,
        pool(&url).await,
        pool(&url).await,
    );

    let (r0, r1, r2, r3) = tokio::join!(
        run_postgres_migrations(&p0),
        run_postgres_migrations(&p1),
        run_postgres_migrations(&p2),
        run_postgres_migrations(&p3),
    );
    for r in [r0, r1, r2, r3] {
        r.expect("concurrent boot migration must succeed");
    }
}
