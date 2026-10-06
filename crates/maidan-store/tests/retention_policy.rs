//! A workspace's own retention, on both backends: the policy is stored within
//! the instance's ceiling with its audit row, and the per-workspace prunes
//! take only that workspace's old messages, events and finished deliveries,
//! and nothing from a workspace under legal hold.

use std::future::Future;

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, ChannelId, Event, Member, MemberKind, MessageId, NewAuditEvent, NewChannel,
    NewDlqEntry, NewMember, NewMessage, NewThread, NewWorkspace, RetentionDays, ThreadId,
    WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

struct Room {
    workspace: WorkspaceId,
    channel: ChannelId,
    thread: ThreadId,
    member: Member,
}

async fn room(store: &dyn Store, name: &str) -> Room {
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
        })
        .await
        .expect("thread");
    Room {
        workspace: ws.id,
        channel: channel.id,
        thread: thread.id,
        member,
    }
}

async fn message(store: &dyn Store, room: &Room, body: &str) -> MessageId {
    store
        .post_message(NewMessage {
            thread_id: room.thread,
            author_id: room.member.id,
            body: body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("message")
        .id
}

async fn event(store: &dyn Store, room: &Room, days_ago: i64) -> i64 {
    store
        .append_event(&Event::MemberJoined {
            occurred_at: Utc::now() - Duration::days(days_ago),
            workspace_id: room.workspace,
            member: room.member.clone(),
        })
        .await
        .expect("event")
        .id
}

async fn dead_letter(store: &dyn Store, room: &Room) {
    store
        .record_dlq_entry(&NewDlqEntry {
            workspace_id: room.workspace,
            channel_id: room.channel,
            thread_id: room.thread,
            member_id: room.member.id,
            reason: "budget".into(),
            used_tokens: 1,
            used_usd_micros: 1,
            used_turns: 1,
        })
        .await
        .expect("dlq");
}

fn audit(workspace_id: WorkspaceId) -> maidan_store::AuditFor<RetentionDays> {
    Box::new(move |days| NewAuditEvent {
        scope: AuditScope::Workspace(workspace_id),
        actor_id: None,
        action: "retention_policy.set".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(workspace_id.0),
        metadata: serde_json::to_value(days).expect("days"),
    })
}

async fn policy_audits(store: &dyn Store, workspace_id: WorkspaceId) -> usize {
    store
        .list_audit_for_workspace(workspace_id, 100)
        .await
        .expect("audit")
        .iter()
        .filter(|row| row.action == "retention_policy.set")
        .count()
}

fn one_day() -> RetentionDays {
    RetentionDays {
        messages_days: Some(1),
        events_days: Some(1),
        deliveries_days: Some(1),
    }
}

async fn run_policy_suite(store: &dyn Store) {
    let a = room(store, "a").await;
    let b = room(store, "b").await;
    assert!(store
        .get_retention_policy(a.workspace)
        .await
        .expect("get")
        .is_unset());

    let instance = RetentionDays {
        messages_days: None,
        events_days: Some(30),
        deliveries_days: Some(14),
    };
    let too_long = RetentionDays {
        events_days: Some(31),
        ..one_day()
    };
    let err = store
        .set_retention_policy_audited(a.workspace, too_long, instance, audit(a.workspace))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidInput(ref m) if m.contains("events_days")),
        "a policy longer than the instance keeps is refused: {err:?}"
    );
    assert!(store
        .get_retention_policy(a.workspace)
        .await
        .expect("get")
        .is_unset());
    assert_eq!(
        policy_audits(store, a.workspace).await,
        0,
        "a refusal records nothing"
    );

    let set = store
        .set_retention_policy_audited(a.workspace, one_day(), instance, audit(a.workspace))
        .await
        .expect("set");
    assert_eq!(set, one_day());
    assert_eq!(
        store.get_retention_policy(a.workspace).await.expect("get"),
        one_day()
    );
    assert!(
        store
            .get_retention_policy(b.workspace)
            .await
            .expect("get b")
            .is_unset(),
        "one workspace's policy is not another's"
    );
    assert_eq!(policy_audits(store, a.workspace).await, 1);
    assert_eq!(policy_audits(store, b.workspace).await, 0);
    assert_eq!(
        store.list_retention_policies().await.expect("list"),
        vec![(a.workspace, one_day())]
    );

    store
        .set_retention_policy_audited(
            a.workspace,
            RetentionDays::default(),
            instance,
            audit(a.workspace),
        )
        .await
        .expect("clear");
    assert!(store
        .list_retention_policies()
        .await
        .expect("list")
        .is_empty());
    assert_eq!(policy_audits(store, a.workspace).await, 2);
}

/// `backdate` moves every stored message, dead letter and (published) outbox
/// row ten days into the past, in the database's own clock.
async fn run_prune_suite<F, Fut>(store: &dyn Store, backdate: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let a = room(store, "pruned").await;
    let b = room(store, "untouched").await;
    let h = room(store, "held").await;
    let mut old = Vec::new();
    for r in [&a, &b, &h] {
        let msg = message(store, r, "old words").await;
        let ev = event(store, r, 10).await;
        dead_letter(store, r).await;
        old.push((msg, ev));
    }
    backdate().await;
    let a_recent_msg = message(store, &a, "new words").await;
    let a_recent_ev = event(store, &a, 0).await;
    store
        .place_legal_hold(h.workspace, "matter", None)
        .await
        .expect("hold");

    let cutoff = Utc::now() - Duration::days(1);
    // Deliveries before events: an event's outbox row goes with the event.
    assert_eq!(
        store
            .prune_workspace_deliveries(a.workspace, cutoff, 100)
            .await
            .expect("deliveries"),
        2,
        "A's old dead-lettered run and its old published outbox row, not B's or H's"
    );
    assert_eq!(
        store
            .prune_workspace_messages(a.workspace, cutoff, 100)
            .await
            .expect("messages"),
        1,
        "A's old message goes"
    );
    assert_eq!(
        store
            .prune_workspace_events(a.workspace, cutoff, 100)
            .await
            .expect("events"),
        1,
        "A's old event goes"
    );

    let (a_old_msg, a_old_ev) = old[0];
    assert!(
        store.get_message(a_old_msg).await.is_err(),
        "A's old message is gone"
    );
    assert!(
        store.get_stored_event(a_old_ev).await.is_err(),
        "A's old event is gone"
    );
    assert!(store
        .list_channel_dlq(a.channel, 10)
        .await
        .expect("dlq")
        .is_empty());
    assert_eq!(
        store.get_message(a_recent_msg).await.expect("recent").body,
        "new words",
        "a message inside the window stays"
    );
    assert!(
        store.get_stored_event(a_recent_ev).await.is_ok(),
        "a recent event stays"
    );

    for (r, (msg, ev)) in [(&b, old[1]), (&h, old[2])] {
        assert_eq!(
            store.get_message(msg).await.expect("kept").body,
            "old words",
            "A's prune leaves another workspace's messages"
        );
        assert!(store.get_stored_event(ev).await.is_ok());
        assert_eq!(
            store
                .list_channel_dlq(r.channel, 10)
                .await
                .expect("dlq")
                .len(),
            1
        );
    }

    // The held workspace's own prunes take nothing, however old its rows.
    assert_eq!(
        store
            .prune_workspace_messages(h.workspace, cutoff, 100)
            .await
            .expect("held messages"),
        0
    );
    assert_eq!(
        store
            .prune_workspace_events(h.workspace, cutoff, 100)
            .await
            .expect("held events"),
        0
    );
    assert_eq!(
        store
            .prune_workspace_deliveries(h.workspace, cutoff, 100)
            .await
            .expect("held deliveries"),
        0
    );
    let (h_msg, h_ev) = old[2];
    assert_eq!(
        store.get_message(h_msg).await.expect("held").body,
        "old words"
    );
    assert!(store.get_stored_event(h_ev).await.is_ok());
    assert_eq!(
        store
            .list_channel_dlq(h.channel, 10)
            .await
            .expect("dlq")
            .len(),
        1
    );
}

async fn sqlite() -> (SqliteStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (SqliteStore::for_tests(pool.clone()), pool)
}

async fn backdate_sqlite(pool: &sqlx::SqlitePool) {
    for sql in [
        "UPDATE maidan_messages SET posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', posted_at, '-10 days')",
        "UPDATE maidan_agent_work_dlq SET failed_at = strftime('%Y-%m-%dT%H:%M:%fZ', failed_at, '-10 days')",
        "UPDATE maidan_outbox SET published_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-10 days')",
    ] {
        sqlx::query(sql).execute(pool).await.expect(sql);
    }
}

async fn backdate_postgres(pool: &sqlx::PgPool) {
    for sql in [
        "UPDATE maidan_messages SET posted_at = posted_at - INTERVAL '10 days'",
        "UPDATE maidan_agent_work_dlq SET failed_at = failed_at - INTERVAL '10 days'",
        "UPDATE maidan_outbox SET published_at = NOW() - INTERVAL '10 days'",
    ] {
        sqlx::query(sql).execute(pool).await.expect(sql);
    }
}

#[tokio::test]
async fn a_workspace_retention_policy_is_bounded_by_the_instance_and_audited_sqlite() {
    let (store, _pool) = sqlite().await;
    run_policy_suite(&store).await;
}

#[tokio::test]
async fn a_workspace_prune_takes_only_its_own_old_rows_and_nothing_held_sqlite() {
    let (store, pool) = sqlite().await;
    run_prune_suite(&store, || backdate_sqlite(&pool)).await;
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    maidan_store::PostgresStore,
    sqlx::PgPool,
)> {
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
            return None;
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
    Some((container, PostgresStore::for_tests(pool.clone()), pool))
}

#[tokio::test]
async fn a_workspace_retention_policy_is_bounded_by_the_instance_and_audited_postgres() {
    let Some((_container, store, _pool)) = postgres().await else {
        return;
    };
    run_policy_suite(&store).await;
}

#[tokio::test]
async fn a_workspace_prune_takes_only_its_own_old_rows_and_nothing_held_postgres() {
    let Some((_container, store, pool)) = postgres().await else {
        return;
    };
    run_prune_suite(&store, || backdate_postgres(&pool)).await;
}
