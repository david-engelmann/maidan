//! The Postgres audit log is partitioned by month of `occurred_at` (0165).
//! These run against a real Postgres: rows reach the month they belong to,
//! retention drops a month only when no row in it is held, the migration
//! keeps every pre-existing row, and maintenance keeps the coming months
//! ready. SQLite keeps one table; nothing here.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_store::postgres::partitions::{self, month_start, next_month, partition_name, AUDIT};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{Member, MemberKind, NewMember, NewWorkspace, WorkspaceId};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Pg {
    _container: ContainerAsync<Postgres>,
    pool: PgPool,
    store: PostgresStore,
}

/// A fresh Postgres; `before` runs on it before the migrations do.
async fn postgres_with(before: Option<&str>) -> Option<Pg> {
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
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    if let Some(sql) = before {
        sqlx::raw_sql(sql).execute(&pool).await.expect("before");
    }
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    Some(Pg {
        _container: container,
        pool,
        store,
    })
}

async fn postgres() -> Option<Pg> {
    postgres_with(None).await
}

/// The first instant of the month `n` months after now's.
fn month(n: u32) -> DateTime<Utc> {
    let mut at = month_start(Utc::now());
    for _ in 0..n {
        at = next_month(at);
    }
    at
}

fn days(n: i64) -> chrono::Duration {
    chrono::Duration::days(n)
}

async fn tenant(store: &dyn Store, name: &str) -> Member {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
}

/// One audit row at `at`, written the way the pre-partitioning server wrote
/// them all: straight at the table, which the parent routes by `occurred_at`.
async fn insert_at(pool: &PgPool, ws: Option<WorkspaceId>, at: DateTime<Utc>) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO maidan_audit (occurred_at, workspace_id, action)
         VALUES ($1, $2, 'test.action') RETURNING id",
    )
    .bind(at)
    .bind(ws.map(|w| w.0))
    .fetch_one(pool)
    .await
    .expect("insert audit row")
}

/// The partition an audit row is in.
async fn home(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT tableoid::regclass::text FROM maidan_audit WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("home")
}

async fn partition_names(pool: &PgPool) -> Vec<String> {
    partitions::list(pool, &AUDIT)
        .await
        .expect("partitions")
        .into_iter()
        .map(|p| p.name)
        .collect()
}

async fn audit_ids(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT id FROM maidan_audit ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("ids")
}

/// Prune until a call removes fewer than `limit`: the caller's loop.
async fn prune_all(store: &dyn Store, cutoff: DateTime<Utc>, limit: i64) -> u64 {
    let mut total = 0;
    loop {
        let n = store.prune_audit(cutoff, limit).await.expect("prune");
        total += n;
        if n < u64::try_from(limit).expect("limit") {
            return total;
        }
    }
}

#[tokio::test]
async fn an_audit_row_lands_in_the_month_it_occurred_in() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "route").await;

    // Boot made the current month's successors through three ahead, and
    // DEFAULT; the current month is still in the pre-partitioning partition.
    assert_eq!(
        partition_names(&pg.pool).await,
        vec![
            "maidan_audit_legacy".to_string(),
            partition_name(&AUDIT, month(1)),
            partition_name(&AUDIT, month(2)),
            partition_name(&AUDIT, month(3)),
            "maidan_audit_default".to_string(),
        ]
    );

    let now = insert_at(&pg.pool, Some(alice.workspace_id), Utc::now()).await;
    let old = insert_at(&pg.pool, Some(alice.workspace_id), Utc::now() - days(500)).await;
    let next = insert_at(&pg.pool, Some(alice.workspace_id), month(1) + days(2)).await;
    let edge = insert_at(&pg.pool, Some(alice.workspace_id), month(3)).await;
    let ahead = insert_at(&pg.pool, Some(alice.workspace_id), month(4) + days(1)).await;

    assert_eq!(home(&pg.pool, now).await, "maidan_audit_legacy");
    assert_eq!(home(&pg.pool, old).await, "maidan_audit_legacy");
    assert_eq!(home(&pg.pool, next).await, partition_name(&AUDIT, month(1)));
    assert_eq!(
        home(&pg.pool, edge).await,
        partition_name(&AUDIT, month(3)),
        "a lower bound is inclusive"
    );
    assert_eq!(
        home(&pg.pool, ahead).await,
        "maidan_audit_default",
        "past the ready months"
    );

    // Each row is read back whole through the parent, wherever it lives.
    assert_eq!(audit_ids(&pg.pool).await, vec![now, old, next, edge, ahead]);
}

/// Mark 0165 applied before the first run, so the log is built unpartitioned
/// and filled the way an older server filled it; then unmark it and migrate.
const SKIP_0165: &str = "CREATE TABLE maidan_migrations (
        version BIGINT PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    INSERT INTO maidan_migrations (version) VALUES (165);";

#[tokio::test]
async fn the_migration_keeps_every_existing_row_in_order() {
    let Some(pg) = postgres_with(Some(SKIP_0165)).await else {
        return;
    };
    let relkind: String = sqlx::query_scalar(
        "SELECT relkind::text FROM pg_class WHERE oid = 'maidan_audit'::regclass",
    )
    .fetch_one(&pg.pool)
    .await
    .expect("relkind");
    assert_eq!(relkind, "r", "the log starts as one table");

    let alice = tenant(&pg.store, "a").await;
    let bob = tenant(&pg.store, "b").await;
    let mut ids = Vec::new();
    for i in 0..30i64 {
        let at = Utc::now() - days(400) + days(i * 13);
        let ws = if i % 3 == 0 {
            bob.workspace_id
        } else {
            alice.workspace_id
        };
        ids.push(insert_at(&pg.pool, Some(ws), at).await);
    }
    // One row dated ahead: the old table's partition must reach past it.
    ids.push(insert_at(&pg.pool, Some(alice.workspace_id), month(2) + days(5)).await);
    let before = audit_ids(&pg.pool).await;
    assert_eq!(before, ids, "ids come from one sequence, in insert order");

    sqlx::query("DELETE FROM maidan_migrations WHERE version = 165")
        .execute(&pg.pool)
        .await
        .expect("unmark");
    run_postgres_migrations(&pg.pool)
        .await
        .expect("migrate 0165");

    let relkind: String = sqlx::query_scalar(
        "SELECT relkind::text FROM pg_class WHERE oid = 'maidan_audit'::regclass",
    )
    .fetch_one(&pg.pool)
    .await
    .expect("relkind");
    assert_eq!(relkind, "p");
    assert_eq!(audit_ids(&pg.pool).await, before, "same rows, same ids");
    for id in &before {
        assert_eq!(home(&pg.pool, *id).await, "maidan_audit_legacy");
    }

    // The old table is the first partition and reaches the month after its
    // newest row; the ready months start there.
    let parts = partitions::list(&pg.pool, &AUDIT).await.expect("list");
    assert_eq!(parts[0].name, "maidan_audit_legacy");
    assert_eq!(parts[0].lower, None);
    assert_eq!(parts[0].upper, Some(month(3)));
    assert_eq!(parts[1].name, partition_name(&AUDIT, month(3)));
}

#[tokio::test]
async fn retention_drops_a_month_past_the_cutoff_and_deletes_inside_the_legacy_partition() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;

    // A whole future month of rows, reached by a cutoff past its end.
    let mut month_rows = Vec::new();
    for i in 0..3 {
        month_rows
            .push(insert_at(&pg.pool, Some(alice.workspace_id), month(1) + days(1 + i)).await);
    }
    // Rows in the legacy partition on both sides of a cutoff inside it.
    let old = insert_at(&pg.pool, Some(alice.workspace_id), Utc::now() - days(90)).await;
    let recent = insert_at(&pg.pool, Some(alice.workspace_id), Utc::now() - days(1)).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(
        removed,
        u64::try_from(month_rows.len()).expect("count"),
        "the dropped month's rows are counted"
    );
    assert_eq!(audit_ids(&pg.pool).await, vec![old, recent]);
    assert!(
        !partition_names(&pg.pool)
            .await
            .contains(&partition_name(&AUDIT, month(1))),
        "the month past the cutoff is dropped, not emptied"
    );

    // A cutoff inside the legacy partition deletes there, batch by batch,
    // and leaves the newer rows.
    let removed = prune_all(&pg.store, Utc::now() - days(30), 100).await;
    assert_eq!(removed, 1);
    assert_eq!(audit_ids(&pg.pool).await, vec![recent]);
}

#[tokio::test]
async fn retention_keeps_a_month_holding_a_held_row_and_deletes_its_unheld_rows() {
    let Some(pg) = postgres().await else { return };
    let held = tenant(&pg.store, "held").await;
    let free = tenant(&pg.store, "free").await;
    pg.store
        .place_legal_hold(held.workspace_id, "matter", None)
        .await
        .expect("hold");

    let held_row = insert_at(&pg.pool, Some(held.workspace_id), month(1) + days(1)).await;
    insert_at(&pg.pool, Some(free.workspace_id), month(1) + days(2)).await;
    insert_at(&pg.pool, None, month(1) + days(3)).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 2, "the unheld and instance rows go");
    assert_eq!(audit_ids(&pg.pool).await, vec![held_row]);
    assert!(
        partition_names(&pg.pool)
            .await
            .contains(&partition_name(&AUDIT, month(1))),
        "a month holding a held row is kept, not dropped"
    );
}

#[tokio::test]
async fn a_partial_month_is_pruned_in_batches_inside_that_partition() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    for i in 0..5 {
        insert_at(
            &pg.pool,
            Some(alice.workspace_id),
            Utc::now() - days(10 - i),
        )
        .await;
    }
    // The cutoff sits inside the legacy partition's span, so nothing is
    // droppable: the limit is the batch size, exactly as before 0165.
    let cutoff = Utc::now();
    assert_eq!(pg.store.prune_audit(cutoff, 2).await.expect("batch"), 2);
    assert_eq!(pg.store.prune_audit(cutoff, 2).await.expect("batch"), 2);
    assert_eq!(pg.store.prune_audit(cutoff, 2).await.expect("batch"), 1);
    assert!(audit_ids(&pg.pool).await.is_empty());
}

#[tokio::test]
async fn audit_rows_written_by_the_store_read_back_across_partitions() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    // The store's own write path lands at now (the legacy partition), beside
    // a raw row dated into a ready month; list_audit reads across both.
    let written = pg
        .store
        .append_audit(maidan_types::NewAuditEvent {
            scope: maidan_types::AuditScope::Workspace(alice.workspace_id),
            actor_id: Some(alice.id),
            action: "workspace.signup".into(),
            target_kind: None,
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("append audit");
    let future = insert_at(&pg.pool, Some(alice.workspace_id), month(1) + days(2)).await;

    assert_eq!(home(&pg.pool, written.id).await, "maidan_audit_legacy");
    assert_eq!(
        home(&pg.pool, future).await,
        partition_name(&AUDIT, month(1))
    );
    let listed = pg.store.list_audit(10).await.expect("list audit");
    let listed_ids: Vec<i64> = listed.iter().map(|row| row.id).collect();
    assert_eq!(
        listed_ids,
        vec![future, written.id],
        "newest first, both partitions"
    );
}
