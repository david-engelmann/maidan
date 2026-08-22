//! Reader-pool split (Cluster 262): a `PostgresStore` built with a distinct read
//! pool still reads and writes correctly. Uses the same pool for both roles (no
//! real replica needed) — the point is the two-pool constructor, not routing
//! (which arrives in Cluster 264).

use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::NewWorkspace;

#[tokio::test]
async fn with_replica_reader_store_reads_and_writes() {
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");

    // Reader = the same pool here; the split is what's under test, not routing.
    let store = PostgresStore::with_replica_reader(pool.clone(), pool.clone());

    let ws = store
        .create_workspace(NewWorkspace {
            name: "reader-pool".into(),
        })
        .await
        .expect("write via primary pool");
    let got = store.get_workspace(ws.id).await.expect("read");
    assert_eq!(got.id, ws.id);
    assert_eq!(got.name, "reader-pool");
}
