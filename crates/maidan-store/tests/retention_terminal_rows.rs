//! Delivery retention prunes finished rows from every delivery table, and only
//! those: delivered egress and mail, published outbox rows, and dead-lettered
//! agent runs go; pending rows and egress or mail dead letters (an operator's
//! to-do, with an alert on it) stay, and so does everything in a workspace
//! under a legal hold, including webhook, automation and transactional-outbox
//! rows. Both backends.

use std::future::Future;

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{
    AutomationSourceKind, ChannelId, EgressKind, EgressTarget, Event, MemberKind,
    NewAutomationDelivery, NewChannel, NewDlqEntry, NewEgressOutbox, NewMailOutbox, NewMember,
    NewThread, NewWebhookSubscription, NewWorkspace, ThreadId, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

struct Room {
    workspace: WorkspaceId,
    channel: ChannelId,
    thread: ThreadId,
}

fn egress(ws: WorkspaceId, thread: ThreadId, source_log_id: i64) -> NewEgressOutbox {
    NewEgressOutbox {
        workspace_id: ws,
        thread_id: thread,
        source_log_id,
        target: EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
            thread_ts: None,
        },
        body: "hello".into(),
        kind: EgressKind::Projector,
    }
}

fn mail(ws: WorkspaceId) -> NewMailOutbox {
    NewMailOutbox {
        workspace_id: Some(ws),
        source_log_id: None,
        to_address: "a@example.com".into(),
        subject: "s".into(),
        body: "b".into(),
    }
}

async fn room(store: &dyn Store, name: &str, log_base: i64) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    store
        .append_event(&Event::MemberJoined {
            occurred_at: Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("an event, which queues an outbox row");
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
        })
        .await
        .expect("thread");

    // One delivered and one dead-lettered row per outbox. Each is claimed as
    // soon as it is queued, so the claim takes that row.
    for (offset, delivered) in [(0, true), (1, false)] {
        let egress = store
            .enqueue_egress(egress(ws.id, thread.id, log_base + offset))
            .await
            .expect("enqueue egress")
            .expect("queued");
        let mail = store
            .enqueue_mail(mail(ws.id))
            .await
            .expect("enqueue mail")
            .expect("queued");
        let claimed = store
            .claim_next_due_egress(Utc::now(), 300)
            .await
            .expect("claim egress")
            .expect("due");
        assert_eq!(claimed.id, egress);
        let claimed = store
            .claim_next_due_mail(Utc::now(), 300)
            .await
            .expect("claim mail")
            .expect("due");
        assert_eq!(claimed.id, mail);
        if delivered {
            store.mark_egress_delivered(egress).await.expect("egress");
            store.mark_mail_delivered(mail).await.expect("mail");
        } else {
            store
                .mark_egress_failed(egress, "gave up", None)
                .await
                .expect("egress");
            store
                .mark_mail_failed(mail, "gave up", None)
                .await
                .expect("mail");
        }
    }
    store
        .record_dlq_entry(&NewDlqEntry {
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: member.id,
            reason: "budget".into(),
            used_tokens: 1,
            used_usd_micros: 1,
            used_turns: 1,
        })
        .await
        .expect("dlq");
    Room {
        workspace: ws.id,
        channel: channel.id,
        thread: thread.id,
    }
}

/// `publish_outbox` marks every transactional-outbox row published, as the
/// relay would, and returns how many it marked. `published_outbox` counts one
/// workspace's published rows.
async fn run_suite<P, PFut, C, CFut>(store: &dyn Store, publish_outbox: P, published_outbox: C)
where
    P: Fn() -> PFut,
    PFut: Future<Output = u64>,
    C: Fn(WorkspaceId) -> CFut,
    CFut: Future<Output = i64>,
{
    let free = room(store, "free", 1_000).await;
    let held = room(store, "held", 2_000).await;
    // Pending rows last, once nothing else will be claimed before the prune.
    for (room, log_id) in [(&free, 1_999), (&held, 2_999)] {
        store
            .enqueue_egress(egress(room.workspace, room.thread, log_id))
            .await
            .expect("pending egress")
            .expect("queued");
        store
            .enqueue_mail(mail(room.workspace))
            .await
            .expect("pending mail")
            .expect("queued");
    }
    store
        .place_legal_hold(held.workspace, "matter 1", None)
        .await
        .expect("hold");
    let published = publish_outbox().await;
    let free_published = published_outbox(free.workspace).await;
    let held_published = published_outbox(held.workspace).await;
    assert_eq!(
        free_published + held_published,
        i64::try_from(published).expect("count"),
        "each published outbox row belongs to one of the two rooms"
    );
    assert!(free_published > 0 && held_published > 0);

    let past = Utc::now() - Duration::days(1);
    assert_eq!(
        store.prune_deliveries(past, 5_000).await.expect("prune"),
        0,
        "nothing is older than a cutoff in the past"
    );

    let future = Utc::now() + Duration::days(1);
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("prune"),
        u64::try_from(free_published).expect("count") + 3,
        "the free room's delivered egress, delivered mail, dlq entry and \
         published outbox; the held room's outbox stays"
    );
    assert_eq!(published_outbox(free.workspace).await, 0);
    assert_eq!(
        published_outbox(held.workspace).await,
        held_published,
        "a held workspace keeps its published outbox rows"
    );
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("again"),
        0,
        "a second sweep finds nothing"
    );

    assert_eq!(
        store.count_dead_egress().await.expect("dead egress"),
        2,
        "dead letters are the operator's, whatever their age"
    );
    assert_eq!(store.count_dead_mail().await.expect("dead mail"), 2);
    let mut pending_egress = 0;
    while store
        .claim_next_due_egress(Utc::now(), 300)
        .await
        .expect("claim")
        .is_some()
    {
        pending_egress += 1;
    }
    assert_eq!(pending_egress, 2, "pending egress is never pruned");
    assert!(
        store
            .list_channel_dlq(free.channel, 10)
            .await
            .expect("dlq")
            .is_empty(),
        "the free room's dead-lettered run is gone"
    );
    assert_eq!(
        store
            .list_channel_dlq(held.channel, 10)
            .await
            .expect("dlq")
            .len(),
        1,
        "the held room keeps its record"
    );

    let hold = store
        .list_legal_holds()
        .await
        .expect("holds")
        .into_iter()
        .find(|hold| hold.workspace_id == held.workspace)
        .expect("the held room's hold");
    store
        .lift_legal_hold(held.workspace, hold.id)
        .await
        .expect("lift");
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("lifted"),
        u64::try_from(held_published).expect("count") + 3,
        "lifting the hold releases the held room's delivered egress, mail, dlq \
         entry and published outbox"
    );
    assert_eq!(published_outbox(held.workspace).await, 0);
}

#[tokio::test]
async fn delivery_retention_prunes_finished_rows_and_keeps_dead_letters_pending_and_held_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    let published = |ws: WorkspaceId| {
        let pool = pool.clone();
        async move { count_published_sqlite(&pool, ws).await }
    };
    run_suite(
        &store,
        || async {
            sqlx::query(
                "UPDATE maidan_outbox SET published_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE published_at IS NULL",
            )
            .execute(&pool)
            .await
            .expect("publish")
            .rows_affected()
        },
        &published,
    )
    .await;
}

async fn count_published_sqlite(pool: &sqlx::SqlitePool, ws: WorkspaceId) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_outbox o
         INNER JOIN maidan_events e ON e.id = o.log_id
         WHERE e.workspace_id = ? AND o.published_at IS NOT NULL",
    )
    .bind(ws.0)
    .fetch_one(pool)
    .await
    .expect("count");
    row.get("n")
}

#[tokio::test]
async fn delivery_retention_prunes_finished_rows_and_keeps_dead_letters_pending_and_held_postgres()
{
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    let published = |ws: WorkspaceId| {
        let pool = pool.clone();
        async move { count_published_postgres(&pool, ws).await }
    };
    run_suite(
        &store,
        || async {
            sqlx::query("UPDATE maidan_outbox SET published_at = now() WHERE published_at IS NULL")
                .execute(&pool)
                .await
                .expect("publish")
                .rows_affected()
        },
        &published,
    )
    .await;
}

async fn count_published_postgres(pool: &sqlx::PgPool, ws: WorkspaceId) -> i64 {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_outbox o
         INNER JOIN maidan_events e ON e.id = o.log_id
         WHERE e.workspace_id = $1 AND o.published_at IS NOT NULL",
    )
    .bind(ws.0)
    .fetch_one(pool)
    .await
    .expect("count");
    row.get("n")
}

struct Tenant {
    workspace: WorkspaceId,
    member: maidan_types::Member,
}

/// One workspace with a delivered webhook, a quarantined webhook, a pending
/// webhook, a delivered automation, a pending automation, and one event (so
/// one transactional-outbox row).
async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "n".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    store
        .append_event(&Event::MemberJoined {
            occurred_at: Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("event");
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws.id,
            url: "https://hooks.example/in".into(),
            label: None,
            event_kinds: vec!["member_joined".into()],
            secret_ciphertext: "x".into(),
        })
        .await
        .expect("subscription");
    let delivered = store
        .enqueue_webhook_delivery(sub.id, 1, "{}")
        .await
        .expect("webhook");
    store
        .mark_webhook_delivery_delivered(delivered)
        .await
        .expect("delivered");
    let quarantined = store
        .enqueue_webhook_delivery(sub.id, 2, "{}")
        .await
        .expect("webhook");
    store
        .quarantine_webhook_delivery(quarantined)
        .await
        .expect("quarantine");
    store
        .enqueue_webhook_delivery(sub.id, 3, "{}")
        .await
        .expect("pending webhook");
    let automation = |payload: &str| NewAutomationDelivery {
        workspace_id: ws.id,
        source_kind: AutomationSourceKind::SlashCommand,
        source_id: uuid::Uuid::new_v4(),
        target_url: "https://hooks.example/auto".into(),
        header_name: "X-Maidan-Event".into(),
        header_value: "slash".into(),
        payload: payload.into(),
    };
    let delivered = store
        .enqueue_automation_delivery(automation("delivered"))
        .await
        .expect("automation");
    store
        .mark_automation_delivery_delivered(delivered)
        .await
        .expect("delivered");
    store
        .enqueue_automation_delivery(automation("pending"))
        .await
        .expect("pending automation");
    Tenant {
        workspace: ws.id,
        member,
    }
}

async fn listed(store: &dyn Store, ws: WorkspaceId, filter: AutomationDeliveryFilter) -> usize {
    store
        .list_webhook_deliveries(ws, filter, 20)
        .await
        .expect("webhooks")
        .len()
}

/// The instance sweep drops one tenant's finished webhook, automation and
/// outbox rows and keeps the other's, because that workspace is held.
async fn run_two_tenant<P, PFut, C, CFut>(store: &dyn Store, publish_outbox: P, published_outbox: C)
where
    P: Fn() -> PFut,
    PFut: Future<Output = ()>,
    C: Fn(WorkspaceId) -> CFut,
    CFut: Future<Output = i64>,
{
    let free = tenant(store, "tenant-free").await;
    let held = tenant(store, "tenant-held").await;
    store
        .place_legal_hold(held.workspace, "matter", None)
        .await
        .expect("hold");
    publish_outbox().await;
    // One unpublished outbox row each, which retention must not touch.
    for room in [&free, &held] {
        store
            .append_event(&Event::MemberJoined {
                occurred_at: Utc::now(),
                workspace_id: room.workspace,
                member: room.member.clone(),
            })
            .await
            .expect("unpublished event");
    }
    let free_published = published_outbox(free.workspace).await;
    let held_published = published_outbox(held.workspace).await;
    assert!(free_published >= 1 && held_published >= 1);

    let future = Utc::now() + Duration::days(1);
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("prune"),
        u64::try_from(free_published).expect("count") + 3,
        "the free tenant's delivered webhook, quarantined webhook, delivered \
         automation and published outbox"
    );
    assert_eq!(published_outbox(free.workspace).await, 0);
    assert_eq!(
        published_outbox(held.workspace).await,
        held_published,
        "the held tenant keeps its published outbox"
    );
    assert_eq!(
        listed(store, free.workspace, AutomationDeliveryFilter::Delivered).await,
        0
    );
    assert_eq!(
        listed(store, free.workspace, AutomationDeliveryFilter::DeadLetter).await,
        0
    );
    assert_eq!(
        listed(store, free.workspace, AutomationDeliveryFilter::Pending).await,
        1
    );
    assert_eq!(
        listed(store, held.workspace, AutomationDeliveryFilter::Delivered).await,
        1
    );
    assert_eq!(
        listed(store, held.workspace, AutomationDeliveryFilter::DeadLetter).await,
        1
    );
    assert_eq!(
        listed(store, held.workspace, AutomationDeliveryFilter::Pending).await,
        1
    );
    assert!(store
        .list_automation_deliveries(free.workspace, AutomationDeliveryFilter::Delivered, 20)
        .await
        .expect("automation")
        .is_empty());
    assert_eq!(
        store
            .list_automation_deliveries(free.workspace, AutomationDeliveryFilter::Pending, 20)
            .await
            .expect("automation")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_automation_deliveries(held.workspace, AutomationDeliveryFilter::Delivered, 20)
            .await
            .expect("automation")
            .len(),
        1,
        "the held tenant keeps its delivered automation"
    );
    assert_eq!(
        store
            .list_automation_deliveries(held.workspace, AutomationDeliveryFilter::Pending, 20)
            .await
            .expect("automation")
            .len(),
        1
    );

    let hold = store
        .list_legal_holds()
        .await
        .expect("holds")
        .into_iter()
        .find(|h| h.workspace_id == held.workspace)
        .expect("hold");
    store
        .lift_legal_hold(held.workspace, hold.id)
        .await
        .expect("lift");
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("lifted"),
        u64::try_from(held_published).expect("count") + 3,
        "lifting the hold releases the held tenant's finished deliveries"
    );
    assert_eq!(published_outbox(held.workspace).await, 0);
    assert_eq!(
        listed(store, held.workspace, AutomationDeliveryFilter::Pending).await,
        1
    );
    assert_eq!(
        store
            .list_automation_deliveries(held.workspace, AutomationDeliveryFilter::Pending, 20)
            .await
            .expect("automation")
            .len(),
        1,
        "a pending delivery is never pruned"
    );
}

#[tokio::test]
async fn instance_sweep_skips_a_held_workspaces_webhook_automation_and_outbox_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    let published = |ws: WorkspaceId| {
        let pool = pool.clone();
        async move { count_published_sqlite(&pool, ws).await }
    };
    run_two_tenant(
        &store,
        || async {
            sqlx::query(
                "UPDATE maidan_outbox SET published_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE published_at IS NULL",
            )
            .execute(&pool)
            .await
            .expect("publish");
        },
        &published,
    )
    .await;
}

#[tokio::test]
async fn instance_sweep_skips_a_held_workspaces_webhook_automation_and_outbox_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    let published = |ws: WorkspaceId| {
        let pool = pool.clone();
        async move { count_published_postgres(&pool, ws).await }
    };
    run_two_tenant(
        &store,
        || async {
            sqlx::query("UPDATE maidan_outbox SET published_at = now() WHERE published_at IS NULL")
                .execute(&pool)
                .await
                .expect("publish");
        },
        &published,
    )
    .await;
}
