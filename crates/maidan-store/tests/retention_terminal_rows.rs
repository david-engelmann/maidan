//! Delivery retention prunes finished rows from every delivery table, and only
//! those: delivered egress and mail, published outbox rows, and dead-lettered
//! agent runs go; pending rows and egress or mail dead letters (an operator's
//! to-do, with an alert on it) stay, and so does everything in a workspace
//! under a legal hold. Both backends.

use std::future::Future;

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EgressKind, EgressTarget, Event, MemberKind, NewChannel, NewDlqEntry,
    NewEgressOutbox, NewMailOutbox, NewMember, NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

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
/// relay would, and returns how many it marked.
async fn run_suite<F, Fut>(store: &dyn Store, publish_outbox: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = u64>,
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
    assert!(published >= 2, "each room appended an event");

    let past = Utc::now() - Duration::days(1);
    assert_eq!(
        store.prune_deliveries(past, 5_000).await.expect("prune"),
        0,
        "nothing is older than a cutoff in the past"
    );

    let future = Utc::now() + Duration::days(1);
    assert_eq!(
        store.prune_deliveries(future, 5_000).await.expect("prune"),
        published + 3,
        "the free room's delivered egress, delivered mail and dlq entry, and \
         every published outbox row"
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
        3,
        "lifting the hold releases the held room's delivered egress, mail and dlq entry"
    );
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
    run_suite(&store, || async {
        sqlx::query(
            "UPDATE maidan_outbox SET published_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE published_at IS NULL",
        )
        .execute(&pool)
        .await
        .expect("publish")
        .rows_affected()
    })
    .await;
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
    run_suite(&store, || async {
        sqlx::query("UPDATE maidan_outbox SET published_at = now() WHERE published_at IS NULL")
            .execute(&pool)
            .await
            .expect("publish")
            .rows_affected()
    })
    .await;
}
