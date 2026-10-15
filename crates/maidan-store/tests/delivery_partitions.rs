//! The Postgres delivery tables are partitioned by month (0166): webhook
//! and automation deliveries and the outbox by `created_at`, mail by
//! `updated_at`, the dead-letter queue by `failed_at`. These run against a
//! real Postgres: rows reach the month they belong to, retention drops a
//! month only when every row in it is one the batched delete would take,
//! and the migration keeps every pre-existing row. The egress outbox is
//! not partitioned (its dedup index is global); nothing here for it.
//! SQLite keeps one table each; nothing here.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_store::postgres::partitions::{
    self, month_start, next_month, partition_name, PartitionedTable, AGENT_WORK_DLQ,
    AUTOMATION_DELIVERIES, MAIL_OUTBOX, OUTBOX, WEBHOOK_DELIVERIES,
};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{
    ChannelId, Member, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

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

struct Tenant {
    member: Member,
    channel_id: ChannelId,
    thread_id: ThreadId,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("work".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    Tenant {
        member,
        channel_id: channel.id,
        thread_id: thread.id,
    }
}

async fn subscription(pool: &PgPool, ws: WorkspaceId) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO maidan_webhook_subscriptions (id, workspace_id, url, event_kinds, secret_ciphertext)
         VALUES (gen_random_uuid(), $1, 'https://example.invalid/hook', '[]', 'x')
         RETURNING id",
    )
    .bind(ws.0)
    .fetch_one(pool)
    .await
    .expect("subscription")
}

async fn webhook_at(pool: &PgPool, sub: Uuid, at: DateTime<Utc>, delivered: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO maidan_webhook_deliveries (subscription_id, log_id, payload, delivered_at, created_at)
         VALUES ($1, 1, '{}', CASE WHEN $3 THEN $2 ELSE NULL END, $2)
         RETURNING id",
    )
    .bind(sub)
    .bind(at)
    .bind(delivered)
    .fetch_one(pool)
    .await
    .expect("webhook delivery")
}

async fn automation_at(pool: &PgPool, ws: WorkspaceId, at: DateTime<Utc>, delivered: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO maidan_automation_deliveries
             (workspace_id, source_kind, source_id, target_url, header_name, header_value, payload, delivered_at, created_at)
         VALUES ($1, 'k', gen_random_uuid(), 'https://example.invalid', 'h', 'v', '{}', CASE WHEN $3 THEN $2 ELSE NULL END, $2)
         RETURNING id",
    )
    .bind(ws.0)
    .bind(at)
    .bind(delivered)
    .fetch_one(pool)
    .await
    .expect("automation delivery")
}

async fn outbox_at(pool: &PgPool, at: DateTime<Utc>, published: Option<DateTime<Utc>>) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO maidan_outbox (log_id, created_at, published_at)
         VALUES (1, $1, $2) RETURNING id",
    )
    .bind(at)
    .bind(published)
    .fetch_one(pool)
    .await
    .expect("outbox row")
}

async fn mail_at(pool: &PgPool, ws: WorkspaceId, at: DateTime<Utc>, status: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO maidan_mail_outbox (id, workspace_id, to_address, subject, body, status, created_at, updated_at)
         VALUES (gen_random_uuid(), $1, 'a@example.com', 's', 'b', $3, $2, $2)
         RETURNING id",
    )
    .bind(ws.0)
    .bind(at)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("mail row")
}

async fn dlq_at(pool: &PgPool, t: &Tenant, at: DateTime<Utc>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO maidan_agent_work_dlq (id, workspace_id, channel_id, thread_id, member_id, reason, failed_at)
         VALUES (gen_random_uuid(), $1, $2, $3, $4, 'budget', $5)
         RETURNING id",
    )
    .bind(t.member.workspace_id.0)
    .bind(t.channel_id.0)
    .bind(t.thread_id.0)
    .bind(t.member.id.0)
    .bind(at)
    .fetch_one(pool)
    .await
    .expect("dlq row")
}

/// The partition a row is in, by table and id text.
async fn home(pool: &PgPool, table: &str, id: &str) -> String {
    sqlx::query_scalar(&format!(
        "SELECT tableoid::regclass::text FROM {table} WHERE id::text = $1"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("home")
}

async fn names_of(pool: &PgPool, table: &PartitionedTable) -> Vec<String> {
    partitions::list(pool, table)
        .await
        .expect("partitions")
        .into_iter()
        .map(|p| p.name)
        .collect()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

/// Prune deliveries until a call removes fewer than `limit`: the caller's loop.
async fn prune_all(store: &dyn Store, cutoff: DateTime<Utc>, limit: i64) -> u64 {
    let mut total = 0;
    loop {
        let n = store.prune_deliveries(cutoff, limit).await.expect("prune");
        total += n;
        if n < u64::try_from(limit).expect("limit") {
            return total;
        }
    }
}

#[tokio::test]
async fn rows_land_in_the_month_they_were_enqueued_in() {
    let Some(pg) = postgres().await else { return };
    let t = tenant(&pg.store, "route").await;
    let ws = t.member.workspace_id;
    let sub = subscription(&pg.pool, ws).await;

    for table in [
        &WEBHOOK_DELIVERIES,
        &AUTOMATION_DELIVERIES,
        &OUTBOX,
        &MAIL_OUTBOX,
        &AGENT_WORK_DLQ,
    ] {
        assert_eq!(
            names_of(&pg.pool, table).await,
            vec![
                format!("{}_legacy", table.parent),
                partition_name(table, month(1)),
                partition_name(table, month(2)),
                partition_name(table, month(3)),
                format!("{}_default", table.parent),
            ],
            "{} partitions",
            table.parent
        );
    }

    let at = month(1) + days(2);
    let wh = webhook_at(&pg.pool, sub, at, true).await;
    assert_eq!(
        home(&pg.pool, "maidan_webhook_deliveries", &wh.to_string()).await,
        partition_name(&WEBHOOK_DELIVERIES, month(1))
    );
    let au = automation_at(&pg.pool, ws, at, true).await;
    assert_eq!(
        home(&pg.pool, "maidan_automation_deliveries", &au.to_string()).await,
        partition_name(&AUTOMATION_DELIVERIES, month(1))
    );
    let ob = outbox_at(&pg.pool, at, Some(at)).await;
    assert_eq!(
        home(&pg.pool, "maidan_outbox", &ob.to_string()).await,
        partition_name(&OUTBOX, month(1))
    );
    let ml = mail_at(&pg.pool, ws, at, "delivered").await;
    assert_eq!(
        home(&pg.pool, "maidan_mail_outbox", &ml.to_string()).await,
        partition_name(&MAIL_OUTBOX, month(1))
    );
    let dq = dlq_at(&pg.pool, &t, at).await;
    assert_eq!(
        home(&pg.pool, "maidan_agent_work_dlq", &dq.to_string()).await,
        partition_name(&AGENT_WORK_DLQ, month(1))
    );

    // And a row written now sits in the legacy partition, in every table.
    let now_wh = webhook_at(&pg.pool, sub, Utc::now(), false).await;
    assert_eq!(
        home(&pg.pool, "maidan_webhook_deliveries", &now_wh.to_string()).await,
        "maidan_webhook_deliveries_legacy"
    );
}

/// Mark 0166 applied before the first run, so the tables are built
/// unpartitioned and filled the way an older server filled them; then
/// unmark it and migrate.
const SKIP_0166: &str = "CREATE TABLE maidan_migrations (
        version BIGINT PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    INSERT INTO maidan_migrations (version) VALUES (166);";

#[tokio::test]
async fn the_migration_keeps_every_existing_row() {
    let Some(pg) = postgres_with(Some(SKIP_0166)).await else {
        return;
    };
    let t = tenant(&pg.store, "a").await;
    let ws = t.member.workspace_id;
    let sub = subscription(&pg.pool, ws).await;
    let old = Utc::now() - days(200);
    webhook_at(&pg.pool, sub, old, true).await;
    automation_at(&pg.pool, ws, old, true).await;
    outbox_at(&pg.pool, old, Some(old)).await;
    outbox_at(&pg.pool, old, None).await; // still queued at migration time
    mail_at(&pg.pool, ws, old, "delivered").await;
    dlq_at(&pg.pool, &t, old).await;
    let tables = [
        "maidan_webhook_deliveries",
        "maidan_automation_deliveries",
        "maidan_outbox",
        "maidan_mail_outbox",
        "maidan_agent_work_dlq",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(count(&pg.pool, table).await);
    }

    sqlx::query("DELETE FROM maidan_migrations WHERE version = 166")
        .execute(&pg.pool)
        .await
        .expect("unmark");
    run_postgres_migrations(&pg.pool)
        .await
        .expect("migrate 0166");

    for (i, table) in tables.iter().enumerate() {
        let relkind: String = sqlx::query_scalar(&format!(
            "SELECT relkind::text FROM pg_class WHERE oid = '{table}'::regclass"
        ))
        .fetch_one(&pg.pool)
        .await
        .expect("relkind");
        assert_eq!(relkind, "p", "{table} is partitioned");
        assert_eq!(
            count(&pg.pool, table).await,
            before[i],
            "{table} kept its rows"
        );
    }
}

#[tokio::test]
async fn retention_drops_a_webhook_month_past_the_cutoff() {
    let Some(pg) = postgres().await else { return };
    let t = tenant(&pg.store, "a").await;
    let sub = subscription(&pg.pool, t.member.workspace_id).await;
    webhook_at(&pg.pool, sub, month(1) + days(1), true).await;
    webhook_at(&pg.pool, sub, month(1) + days(2), true).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 2, "the dropped month's rows are counted");
    assert_eq!(count(&pg.pool, "maidan_webhook_deliveries").await, 0);
    assert!(
        !names_of(&pg.pool, &WEBHOOK_DELIVERIES)
            .await
            .contains(&partition_name(&WEBHOOK_DELIVERIES, month(1))),
        "the month is dropped, not emptied"
    );
}

#[tokio::test]
async fn retention_keeps_a_month_holding_a_pending_row_and_deletes_its_finished_rows() {
    let Some(pg) = postgres().await else { return };
    let t = tenant(&pg.store, "a").await;
    let ws = t.member.workspace_id;
    automation_at(&pg.pool, ws, month(1) + days(1), true).await;
    let pending = automation_at(&pg.pool, ws, month(1) + days(2), false).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 1);
    assert_eq!(
        home(
            &pg.pool,
            "maidan_automation_deliveries",
            &pending.to_string()
        )
        .await,
        partition_name(&AUTOMATION_DELIVERIES, month(1)),
        "a month holding a pending row is kept, not dropped"
    );
}

#[tokio::test]
async fn retention_keeps_a_held_workspaces_delivery_month() {
    let Some(pg) = postgres().await else { return };
    let held = tenant(&pg.store, "held").await;
    let free = tenant(&pg.store, "free").await;
    pg.store
        .place_legal_hold(held.member.workspace_id, "matter", None)
        .await
        .expect("hold");
    let kept = automation_at(&pg.pool, held.member.workspace_id, month(1) + days(1), true).await;
    automation_at(&pg.pool, free.member.workspace_id, month(1) + days(2), true).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 1, "only the unheld row goes");
    assert_eq!(
        home(&pg.pool, "maidan_automation_deliveries", &kept.to_string()).await,
        partition_name(&AUTOMATION_DELIVERIES, month(1))
    );
}

#[tokio::test]
async fn an_unpublished_outbox_row_keeps_its_month() {
    let Some(pg) = postgres().await else { return };
    let _t = tenant(&pg.store, "a").await;
    outbox_at(&pg.pool, month(1) + days(1), Some(month(1) + days(1))).await;
    let queued = outbox_at(&pg.pool, month(1) + days(2), None).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 1, "the published row goes, the queued one stays");
    assert_eq!(
        home(&pg.pool, "maidan_outbox", &queued.to_string()).await,
        partition_name(&OUTBOX, month(1)),
        "created_at keys the partition, published_at cuts it: the exact keep-check holds the month"
    );
}

#[tokio::test]
async fn retention_drops_mail_and_dlq_months() {
    let Some(pg) = postgres().await else { return };
    let t = tenant(&pg.store, "a").await;
    mail_at(
        &pg.pool,
        t.member.workspace_id,
        month(1) + days(1),
        "delivered",
    )
    .await;
    dlq_at(&pg.pool, &t, month(1) + days(1)).await;

    let removed = prune_all(&pg.store, month(2), 100).await;
    assert_eq!(removed, 2);
    assert!(!names_of(&pg.pool, &MAIL_OUTBOX)
        .await
        .contains(&partition_name(&MAIL_OUTBOX, month(1))));
    assert!(!names_of(&pg.pool, &AGENT_WORK_DLQ)
        .await
        .contains(&partition_name(&AGENT_WORK_DLQ, month(1))));
}

#[tokio::test]
async fn workspace_retention_still_prunes_its_own_rows() {
    let Some(pg) = postgres().await else { return };
    let a = tenant(&pg.store, "a").await;
    let b = tenant(&pg.store, "b").await;
    let old = Utc::now() - days(90);
    automation_at(&pg.pool, a.member.workspace_id, old, true).await;
    let b_row = automation_at(&pg.pool, b.member.workspace_id, old, true).await;

    let removed = pg
        .store
        .prune_workspace_deliveries(a.member.workspace_id, Utc::now(), 100)
        .await
        .expect("workspace prune");
    assert_eq!(removed, 1);
    assert_eq!(
        home(&pg.pool, "maidan_automation_deliveries", &b_row.to_string()).await,
        "maidan_automation_deliveries_legacy",
        "the other workspace's row is untouched"
    );
}
