//! The usage-meter daily rollup on both backends: a flush adds to the day's
//! quantity, one workspace's counts never land on another's, and a read is
//! held to the days asked for.

use std::time::Duration;

use chrono::NaiveDate;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn row(ws: WorkspaceId, d: u32, meter: UsageMeter, quantity: i64) -> UsageMeterDay {
    UsageMeterDay {
        workspace_id: ws,
        day: day(d),
        meter,
        quantity,
    }
}

async fn run_suite(store: &dyn Store) {
    let a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    store.add_usage_meters(&[]).await.unwrap();
    store
        .add_usage_meters(&[
            row(a, 20, UsageMeter::Requests, 10),
            row(a, 21, UsageMeter::Requests, 4),
            row(a, 21, UsageMeter::McpCalls, 2),
            row(b, 21, UsageMeter::Requests, 7),
        ])
        .await
        .unwrap();
    store
        .add_usage_meters(&[row(a, 21, UsageMeter::Requests, 1)])
        .await
        .unwrap();

    assert_eq!(
        store.list_usage_meters(a, day(20)).await.unwrap(),
        vec![
            row(a, 20, UsageMeter::Requests, 10),
            row(a, 21, UsageMeter::McpCalls, 2),
            row(a, 21, UsageMeter::Requests, 5),
        ]
    );
    assert_eq!(
        store.list_usage_meters(a, day(21)).await.unwrap().len(),
        2,
        "only the days asked for"
    );
    assert_eq!(
        store.list_usage_meters(b, day(1)).await.unwrap(),
        vec![row(b, 21, UsageMeter::Requests, 7)],
        "A's counts never land on B"
    );

    // A workspace gone before the flush (erased, or never there) drops its
    // own count and fails nobody else's. (Found by the red-team review of
    // #1518.)
    let gone = WorkspaceId(uuid::Uuid::now_v7());
    store
        .add_usage_meters(&[
            row(gone, 23, UsageMeter::Requests, 1),
            row(a, 23, UsageMeter::Requests, 3),
        ])
        .await
        .unwrap();
    assert_eq!(
        store.list_usage_meters(a, day(23)).await.unwrap(),
        vec![row(a, 23, UsageMeter::Requests, 3)]
    );
    assert!(store
        .list_usage_meters(gone, day(1))
        .await
        .unwrap()
        .is_empty());
    // A gauge replaces its day's value; both workspaces are due a measurement
    // until each has a row for the day.
    let mut due = store
        .list_unmeasured_workspaces(day(22), UsageMeter::StoredBytes, 10)
        .await
        .unwrap();
    due.sort_by_key(|w| w.0);
    let mut both = vec![a, b];
    both.sort_by_key(|w| w.0);
    assert_eq!(due, both);
    store
        .set_usage_meters(&[row(a, 22, UsageMeter::StoredBytes, 100)])
        .await
        .unwrap();
    store
        .set_usage_meters(&[row(a, 22, UsageMeter::StoredBytes, 40)])
        .await
        .unwrap();
    assert_eq!(
        store
            .list_usage_meters(a, day(22))
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.day == day(22))
            .collect::<Vec<_>>(),
        vec![row(a, 22, UsageMeter::StoredBytes, 40)]
    );
    assert_eq!(
        store
            .list_unmeasured_workspaces(day(22), UsageMeter::StoredBytes, 10)
            .await
            .unwrap(),
        vec![b]
    );
    assert_eq!(store.workspace_stored_bytes(b).await.unwrap(), 0);
}

#[tokio::test]
async fn usage_meters_add_up_per_workspace_and_day_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn usage_meters_add_up_per_workspace_and_day_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
