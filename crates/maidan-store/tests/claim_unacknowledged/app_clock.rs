//! Claim writes stamp the server's clock on both backends (#1605): a claim,
//! an assignment and an unassignment each set `updated_at` from the app's
//! `Utc::now()`, read between the caller's before and after readings, so a
//! database clock that drifts from the server's cannot move a lease's
//! timestamps. Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// The app clock, truncated to the microsecond a timestamp column keeps.
fn app_now() -> DateTime<Utc> {
    Utc::now()
        .duration_trunc(TimeDelta::microseconds(1))
        .unwrap()
}

fn assert_app_clock(what: &str, at: DateTime<Utc>, before: DateTime<Utc>, after: DateTime<Utc>) {
    assert!(
        before <= at && at <= after + TimeDelta::microseconds(1),
        "{what} {at} is on the app clock, between {before} and {after}"
    );
}

async fn run(store: Arc<dyn Store>) {
    let (channel, holder) = super::setup(store.as_ref(), 3).await;
    let threads = store.list_threads(channel).await.unwrap();

    let before = app_now();
    let claimed = store.claim_thread(threads[0].id, holder).await.unwrap();
    let after = app_now();
    assert!(claimed.claimed, "the open thread is claimed");
    assert_app_clock(
        "a claim's updated_at",
        claimed.thread.updated_at,
        before,
        after,
    );

    let before = app_now();
    let assigned = store.assign_thread(threads[1].id, holder).await.unwrap();
    let after = app_now();
    assert_app_clock(
        "an assignment's updated_at",
        assigned.updated_at,
        before,
        after,
    );

    let before = app_now();
    let cleared = store.unassign_thread(threads[1].id).await.unwrap();
    let after = app_now();
    assert_eq!(cleared.assignee_id, None);
    assert_app_clock(
        "an unassignment's updated_at",
        cleared.updated_at,
        before,
        after,
    );
}

#[tokio::test]
async fn claim_writes_stamp_the_app_clock_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn claim_writes_stamp_the_app_clock_on_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
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
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run(Arc::new(PostgresStore::for_tests(pool))).await;
}
