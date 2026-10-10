//! The Postgres event log is partitioned by month of `occurred_at` (0162).
//! These run against a real Postgres: rows reach the month they belong to,
//! reads and cursors cross partitions in `id` order and stay inside their
//! workspace, retention drops a month only when every row in it would have
//! been deleted, the migration keeps every pre-existing row, and maintenance
//! keeps the coming months ready. SQLite keeps one table; nothing here.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_store::postgres::partitions::{self, month_start, next_month, partition_name, EVENTS};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{Event, Member, MemberKind, NewMember, NewWorkspace, WorkspaceId};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
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

async fn append_at(store: &dyn Store, member: &Member, at: DateTime<Utc>) -> i64 {
    store
        .append_event(&Event::MemberJoined {
            occurred_at: at,
            workspace_id: member.workspace_id,
            member: member.clone(),
        })
        .await
        .expect("append")
        .id
}

/// The partition an event row is in.
async fn home(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT tableoid::regclass::text FROM maidan_events WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("home")
}

async fn partition_names(pool: &PgPool) -> Vec<String> {
    partitions::list(pool, &EVENTS)
        .await
        .expect("list")
        .into_iter()
        .map(|p| p.name)
        .collect()
}

async fn event_ids(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT id FROM maidan_events ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("ids")
}

async fn outbox_log_ids(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT log_id FROM maidan_outbox ORDER BY log_id")
        .fetch_all(pool)
        .await
        .expect("outbox")
}

/// The retention worker's loop: call until a call removes fewer than `limit`.
async fn prune_all(store: &dyn Store, cutoff: DateTime<Utc>, max_id: i64, limit: i64) -> u64 {
    let mut total = 0;
    loop {
        let n = store
            .prune_events(cutoff, max_id, limit)
            .await
            .expect("prune");
        total += n;
        if n < u64::try_from(limit).expect("limit") {
            return total;
        }
    }
}

/// Every id in `ws`, read through the cursor `limit` rows at a time.
async fn read_by_cursor(store: &dyn Store, ws: WorkspaceId, limit: i64) -> Vec<i64> {
    let mut seen = Vec::new();
    let mut cursor = 0;
    loop {
        let page = store
            .list_events_after(ws, cursor, limit)
            .await
            .expect("page");
        if page.is_empty() {
            return seen;
        }
        for event in &page {
            assert_eq!(event.workspace_id, Some(ws), "a page left its workspace");
            assert!(event.id > cursor, "ids go up across pages and partitions");
            cursor = event.id;
            seen.push(event.id);
        }
    }
}

#[tokio::test]
async fn an_event_lands_in_the_month_it_occurred_in() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "route").await;

    // Boot made the current month's successors through three ahead, and
    // DEFAULT; the current month is still in the pre-partitioning partition.
    assert_eq!(
        partition_names(&pg.pool).await,
        vec![
            "maidan_events_legacy".to_string(),
            partition_name(&EVENTS, month(1)),
            partition_name(&EVENTS, month(2)),
            partition_name(&EVENTS, month(3)),
            "maidan_events_default".to_string(),
        ]
    );

    let now = append_at(&pg.store, &alice, Utc::now()).await;
    let old = append_at(&pg.store, &alice, Utc::now() - days(500)).await;
    let next = append_at(&pg.store, &alice, month(1) + days(2)).await;
    let edge = append_at(&pg.store, &alice, month(3)).await;
    let last = append_at(
        &pg.store,
        &alice,
        month(4) - chrono::Duration::microseconds(1),
    )
    .await;
    let ahead = append_at(&pg.store, &alice, month(4) + days(1)).await;

    assert_eq!(home(&pg.pool, now).await, "maidan_events_legacy");
    assert_eq!(home(&pg.pool, old).await, "maidan_events_legacy");
    assert_eq!(
        home(&pg.pool, next).await,
        partition_name(&EVENTS, month(1))
    );
    assert_eq!(
        home(&pg.pool, edge).await,
        partition_name(&EVENTS, month(3)),
        "a lower bound is inclusive"
    );
    assert_eq!(
        home(&pg.pool, last).await,
        partition_name(&EVENTS, month(3)),
        "an upper bound is exclusive"
    );
    assert_eq!(
        home(&pg.pool, ahead).await,
        "maidan_events_default",
        "past the ready months"
    );

    // Each row is read back whole, wherever it lives.
    for id in [now, old, next, edge, last, ahead] {
        assert_eq!(pg.store.get_stored_event(id).await.expect("get").id, id);
    }
}

#[tokio::test]
async fn cursor_reads_cross_partitions_in_id_order_and_stay_in_their_workspace() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let bob = tenant(&pg.store, "b").await;

    // Each tenant's rows go round the partitions, so consecutive ids sit in
    // different ones, and the two tenants interleave.
    let homes = [
        Utc::now(),
        month(2) + days(3),
        month(1) + days(1),
        month(5) + days(1),
        month(3) + days(4),
        Utc::now() - days(40),
    ];
    let (mut a_ids, mut b_ids) = (Vec::new(), Vec::new());
    for round in 0..4 {
        for (i, at) in homes.iter().enumerate() {
            let at = *at + chrono::Duration::minutes(round);
            if (i + usize::try_from(round).expect("round")) % 2 == 0 {
                a_ids.push(append_at(&pg.store, &alice, at).await);
            } else {
                b_ids.push(append_at(&pg.store, &bob, at).await);
            }
        }
    }
    let spread: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT tableoid::regclass::text FROM maidan_events WHERE workspace_id = $1",
    )
    .bind(alice.workspace_id.0)
    .fetch_all(&pg.pool)
    .await
    .expect("spread");
    assert!(
        spread.len() >= 5,
        "alice's rows span the partitions: {spread:?}"
    );

    for limit in [1, 3, 100] {
        assert_eq!(
            read_by_cursor(&pg.store, alice.workspace_id, limit).await,
            a_ids
        );
        assert_eq!(
            read_by_cursor(&pg.store, bob.workspace_id, limit).await,
            b_ids
        );
    }
    assert_eq!(
        pg.store
            .min_event_id(alice.workspace_id)
            .await
            .expect("min"),
        a_ids.first().copied()
    );
    // The hash chain links each tenant's rows in id order, across partitions.
    for ws in [alice.workspace_id, bob.workspace_id] {
        let report = pg.store.verify_event_chain(ws).await.expect("verify");
        assert!(report.ok, "chain intact across partitions: {report:?}");
    }
}

#[tokio::test]
async fn retention_drops_months_past_the_cutoff_and_deletes_inside_the_one_it_falls_in() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let bob = tenant(&pg.store, "b").await;

    let legacy = append_at(&pg.store, &alice, Utc::now() - days(10)).await;
    let m1a = append_at(&pg.store, &alice, month(1) + days(2)).await;
    let m1b = append_at(&pg.store, &bob, month(1) + days(3)).await;
    let m2 = append_at(&pg.store, &bob, month(2) + days(2)).await;
    let m3_before = append_at(&pg.store, &alice, month(3) + days(1)).await;
    let m3_after = append_at(&pg.store, &bob, month(3) + days(20)).await;
    let ahead = append_at(&pg.store, &alice, month(5) + days(1)).await;
    let cutoff = month(3) + days(10);

    let removed = prune_all(&pg.store, cutoff, i64::MAX, 1_000).await;
    assert_eq!(
        removed, 5,
        "every row before the cutoff, dropped or deleted"
    );
    assert_eq!(
        partition_names(&pg.pool).await,
        vec![
            partition_name(&EVENTS, month(3)),
            "maidan_events_default".to_string()
        ],
        "the three partitions that end by the cutoff are dropped; the one it falls in stays"
    );
    assert_eq!(event_ids(&pg.pool).await, vec![m3_after, ahead]);
    // The dropped rows' outbox rows went with them, as the cascade did.
    assert_eq!(outbox_log_ids(&pg.pool).await, vec![m3_after, ahead]);
    for gone in [legacy, m1a, m1b, m2, m3_before] {
        assert!(pg.store.get_stored_event(gone).await.is_err());
    }
    // The chain still verifies after the whole-month drop and the
    // partial-month batch delete.
    for ws in [alice.workspace_id, bob.workspace_id] {
        assert!(pg.store.verify_event_chain(ws).await.expect("verify").ok);
    }
    // The next sweep finds nothing more.
    assert_eq!(prune_all(&pg.store, cutoff, i64::MAX, 1_000).await, 0);
    // And a new row still gets a fresh id.
    let after = append_at(&pg.store, &alice, Utc::now()).await;
    assert!(after > ahead);
}

#[tokio::test]
async fn retention_keeps_a_month_holding_a_held_or_undelivered_row_and_deletes_the_rest() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let bob = tenant(&pg.store, "b").await;
    pg.store
        .place_legal_hold(bob.workspace_id, "matter", None)
        .await
        .expect("hold");

    // Month 1 holds both tenants; month 2 only alice. Bob is held.
    let a1 = append_at(&pg.store, &alice, month(1) + days(1)).await;
    let b1 = append_at(&pg.store, &bob, month(1) + days(2)).await;
    let a1_late = append_at(&pg.store, &alice, month(1) + days(3)).await;
    let a2 = append_at(&pg.store, &alice, month(2) + days(1)).await;
    let a2_late = append_at(&pg.store, &alice, month(2) + days(2)).await;
    let cutoff = month(3);

    // The delivery floor sits at a2: a2_late has not reached every consumer.
    let removed = prune_all(&pg.store, cutoff, a2, 1_000).await;
    assert_eq!(removed, 3, "alice's rows at or below the floor");
    assert_eq!(
        event_ids(&pg.pool).await,
        vec![b1, a2_late],
        "bob's held row and alice's undelivered row stay; their months are kept"
    );
    let names = partition_names(&pg.pool).await;
    assert!(
        names.contains(&partition_name(&EVENTS, month(1))),
        "{names:?}"
    );
    assert!(
        names.contains(&partition_name(&EVENTS, month(2))),
        "{names:?}"
    );
    assert!(
        !names.contains(&"maidan_events_legacy".to_string()),
        "the empty legacy month is dropped"
    );
    let _ = (a1, a1_late);

    // Once the hold lifts and the floor passes, both months go whole.
    let holds = pg.store.list_legal_holds().await.expect("holds");
    for hold in holds
        .into_iter()
        .filter(|h| h.workspace_id == bob.workspace_id)
    {
        pg.store
            .lift_legal_hold(bob.workspace_id, hold.id)
            .await
            .expect("lift");
    }
    assert_eq!(prune_all(&pg.store, cutoff, i64::MAX, 1_000).await, 2);
    assert!(event_ids(&pg.pool).await.is_empty());
    assert_eq!(
        partition_names(&pg.pool).await,
        vec![
            partition_name(&EVENTS, month(3)),
            "maidan_events_default".to_string(),
        ]
    );
}

#[tokio::test]
async fn a_partial_month_is_pruned_in_batches_inside_that_partition() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let mut before = Vec::new();
    for i in 0..7 {
        before.push(
            append_at(
                &pg.store,
                &alice,
                month(1) + days(1) + chrono::Duration::minutes(i),
            )
            .await,
        );
    }
    let after = append_at(&pg.store, &alice, month(1) + days(20)).await;
    let cutoff = month(1) + days(10);

    // Three at a time, oldest first, the caller looping as the worker does.
    assert_eq!(
        pg.store
            .prune_events(cutoff, i64::MAX, 3)
            .await
            .expect("prune"),
        3
    );
    assert_eq!(event_ids(&pg.pool).await[0], before[3], "oldest ids first");
    assert_eq!(prune_all(&pg.store, cutoff, i64::MAX, 3).await, 4);
    assert_eq!(event_ids(&pg.pool).await, vec![after]);
    assert!(partition_names(&pg.pool)
        .await
        .contains(&partition_name(&EVENTS, month(1))));
}

#[tokio::test]
async fn maintenance_keeps_months_ahead_and_moves_rows_out_of_default() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let far = append_at(&pg.store, &alice, month(6) + days(1)).await;
    assert_eq!(home(&pg.pool, far).await, "maidan_events_default");

    // Boot already ran it; running it again changes nothing.
    assert_eq!(
        pg.store
            .maintain_partitions(Utc::now())
            .await
            .expect("again"),
        0
    );

    // Four months on, months 4 to 7 are created, and month 6 takes the row
    // DEFAULT held for it, with its outbox row (a move is not a delete).
    let later = month(4) + days(1);
    assert_eq!(pg.store.maintain_partitions(later).await.expect("later"), 4);
    assert_eq!(home(&pg.pool, far).await, partition_name(&EVENTS, month(6)));
    assert_eq!(outbox_log_ids(&pg.pool).await, vec![far]);
    let in_default: i64 = sqlx::query_scalar("SELECT count(*) FROM maidan_events_default")
        .fetch_one(&pg.pool)
        .await
        .expect("default");
    assert_eq!(in_default, 0);
    assert_eq!(
        pg.store
            .maintain_partitions(later)
            .await
            .expect("idempotent"),
        0
    );

    // Every partition carries the table's autovacuum settings.
    let unset: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
         WHERE i.inhparent = 'maidan_events'::regclass
           AND NOT (COALESCE(c.reloptions, '{}') @> $1::text[])",
    )
    .bind(
        EVENTS
            .reloptions
            .iter()
            .map(|o| o.to_string())
            .collect::<Vec<_>>(),
    )
    .fetch_all(&pg.pool)
    .await
    .expect("reloptions");
    assert!(
        unset.is_empty(),
        "partitions without the settings: {unset:?}"
    );

    // A lost DEFAULT comes back.
    sqlx::query("DROP TABLE maidan_events_default")
        .execute(&pg.pool)
        .await
        .expect("drop default");
    pg.store.maintain_partitions(later).await.expect("restore");
    assert!(partition_names(&pg.pool)
        .await
        .contains(&"maidan_events_default".to_string()));
}

/// Mark 0162 applied before the first run, so the log is built unpartitioned
/// and filled the way an older server filled it; then unmark it and migrate.
const SKIP_0162: &str = "CREATE TABLE maidan_migrations (
        version BIGINT PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    INSERT INTO maidan_migrations (version) VALUES (162);";

#[tokio::test]
async fn the_migration_keeps_every_existing_row_in_order() {
    let Some(pg) = postgres_with(Some(SKIP_0162)).await else {
        return;
    };
    let relkind: String = sqlx::query_scalar(
        "SELECT relkind::text FROM pg_class WHERE oid = 'maidan_events'::regclass",
    )
    .fetch_one(&pg.pool)
    .await
    .expect("relkind");
    assert_eq!(relkind, "r", "the log starts as one table");

    let alice = tenant(&pg.store, "a").await;
    let bob = tenant(&pg.store, "b").await;
    let mut a_ids = Vec::new();
    let mut b_ids = Vec::new();
    for i in 0..120i64 {
        let at = Utc::now() - days(400) + days(i * 3);
        if i % 3 == 0 {
            b_ids.push(append_at(&pg.store, &bob, at).await);
        } else {
            a_ids.push(append_at(&pg.store, &alice, at).await);
        }
    }
    // One row dated ahead: the old table's partition must reach past it.
    a_ids.push(append_at(&pg.store, &alice, month(2) + days(5)).await);
    let before_rows = event_ids(&pg.pool).await;
    let before_outbox = outbox_log_ids(&pg.pool).await;
    let before_links: Vec<(i64, String, String)> =
        sqlx::query("SELECT id, prev_hash, content_hash FROM maidan_events ORDER BY id")
            .fetch_all(&pg.pool)
            .await
            .expect("links")
            .iter()
            .map(|r| (r.get("id"), r.get("prev_hash"), r.get("content_hash")))
            .collect();

    sqlx::query("DELETE FROM maidan_migrations WHERE version = 162")
        .execute(&pg.pool)
        .await
        .expect("unmark");
    run_postgres_migrations(&pg.pool)
        .await
        .expect("migrate 0162");

    let relkind: String = sqlx::query_scalar(
        "SELECT relkind::text FROM pg_class WHERE oid = 'maidan_events'::regclass",
    )
    .fetch_one(&pg.pool)
    .await
    .expect("relkind");
    assert_eq!(relkind, "p");
    assert_eq!(
        event_ids(&pg.pool).await,
        before_rows,
        "same rows, same ids"
    );
    assert_eq!(
        outbox_log_ids(&pg.pool).await,
        before_outbox,
        "outbox untouched"
    );
    let after_links: Vec<(i64, String, String)> =
        sqlx::query("SELECT id, prev_hash, content_hash FROM maidan_events ORDER BY id")
            .fetch_all(&pg.pool)
            .await
            .expect("links")
            .iter()
            .map(|r| (r.get("id"), r.get("prev_hash"), r.get("content_hash")))
            .collect();
    assert_eq!(after_links, before_links, "chain links unchanged");
    assert_eq!(
        read_by_cursor(&pg.store, alice.workspace_id, 7).await,
        a_ids
    );
    assert_eq!(read_by_cursor(&pg.store, bob.workspace_id, 7).await, b_ids);
    for ws in [alice.workspace_id, bob.workspace_id] {
        assert!(pg.store.verify_event_chain(ws).await.expect("verify").ok);
    }

    // The old table is the first partition and reaches the month after its
    // newest row; the ready months start there.
    let parts = partitions::list(&pg.pool, &EVENTS).await.expect("list");
    assert_eq!(parts[0].name, "maidan_events_legacy");
    assert_eq!(parts[0].lower, None);
    assert_eq!(parts[0].upper, Some(month(3)));
    assert_eq!(parts[1].name, partition_name(&EVENTS, month(3)));
    assert!(parts.last().expect("default").is_default);
    let homes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM maidan_events WHERE tableoid <> 'maidan_events_legacy'::regclass",
    )
    .fetch_one(&pg.pool)
    .await
    .expect("homes");
    assert_eq!(homes, 0, "every old row is in the old table's partition");

    // New rows continue the sequence and the chain.
    let next = append_at(&pg.store, &alice, Utc::now()).await;
    assert!(next > *before_rows.last().expect("rows"));
    assert!(
        pg.store
            .verify_event_chain(alice.workspace_id)
            .await
            .expect("verify")
            .ok
    );

    // Dropping the old table's partition does not take the id sequence.
    prune_all(&pg.store, month(3), i64::MAX, 1_000).await;
    assert!(!partition_names(&pg.pool)
        .await
        .contains(&"maidan_events_legacy".to_string()));
    let fresh = append_at(&pg.store, &alice, month(3) + days(1)).await;
    assert!(fresh > next);
}

#[tokio::test]
async fn deleting_an_event_still_cascades_to_its_outbox_and_ingest_rows() {
    let Some(pg) = postgres().await else { return };
    let alice = tenant(&pg.store, "a").await;
    let kept = append_at(&pg.store, &alice, Utc::now()).await;
    let gone = append_at(&pg.store, &alice, month(1) + days(1)).await;
    let peer = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_peers (id, workspace_id, name, base_url, token_hash, remote_workspace_id)
         VALUES ($1, $2, 'p', 'https://peer.invalid', $3, $2)",
    )
    .bind(peer)
    .bind(alice.workspace_id.0)
    .bind(peer.to_string())
    .execute(&pg.pool)
    .await
    .expect("peer");
    for (remote, local) in [(1i64, kept), (2, gone)] {
        sqlx::query(
            "INSERT INTO maidan_federated_ingest (peer_id, remote_event_id, local_event_id)
             VALUES ($1, $2, $3)",
        )
        .bind(peer)
        .bind(remote)
        .bind(local)
        .execute(&pg.pool)
        .await
        .expect("ingest");
    }
    sqlx::query("DELETE FROM maidan_events WHERE id = $1")
        .bind(gone)
        .execute(&pg.pool)
        .await
        .expect("delete");
    assert_eq!(outbox_log_ids(&pg.pool).await, vec![kept]);
    let ingest: Vec<i64> =
        sqlx::query_scalar("SELECT local_event_id FROM maidan_federated_ingest ORDER BY 1")
            .fetch_all(&pg.pool)
            .await
            .expect("ingest");
    assert_eq!(ingest, vec![kept]);
}
