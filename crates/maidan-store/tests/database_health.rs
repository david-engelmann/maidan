//! `MetaStore::database_health`: the Postgres catalogs behind the
//! `maidan_postgres_*` gauges, and none on SQLite.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::NewWorkspace;

#[tokio::test]
async fn sqlite_reports_no_database_health() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool);
    assert_eq!(store.database_health().await.expect("read"), None);
}

#[tokio::test]
async fn postgres_reports_xid_age_wal_and_an_inactive_slot_retaining_wal() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());

    let fresh = store.database_health().await.expect("read").expect("some");
    assert!(fresh.xid_age > 0, "{fresh:?}");
    assert!(fresh.wal_bytes > 0, "{fresh:?}");
    assert!(fresh.dead_tuples >= 0 && fresh.live_tuples >= 0, "{fresh:?}");
    assert_eq!(fresh.inactive_slots, Some(0), "no slot yet: {fresh:?}");
    assert_eq!(fresh.slot_retained_bytes, Some(0), "no slot yet: {fresh:?}");

    // A physical slot reserved now and read by nobody: inactive, and every
    // WAL byte written after it is held for it.
    sqlx::query("SELECT pg_create_physical_replication_slot('health_probe', true)")
        .execute(&pool)
        .await
        .expect("create slot");
    for n in 0..20 {
        store
            .create_workspace(NewWorkspace {
                name: format!("health-{n}"),
            })
            .await
            .expect("write");
    }
    let held = store.database_health().await.expect("read").expect("some");
    assert_eq!(held.inactive_slots, Some(1), "{held:?}");
    assert!(held.slot_retained_bytes.unwrap_or(0) > 0, "{held:?}");
    assert!(held.wal_bytes > fresh.wal_bytes, "{held:?}");
}
