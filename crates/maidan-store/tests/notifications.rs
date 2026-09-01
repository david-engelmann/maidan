//! Per-recipient notifications (Cluster 237, Program C): create / list / mark-read
//! / unread-count. Both backends. Zero-blast-radius foundation — no router yet.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EventKind, MemberKind, NewChannel, NewMember, NewNotification, NewThread, NewWorkspace,
    NotificationId,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "notif".into(),
        })
        .await
        .expect("ws");
    let recipient = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "recipient".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("recipient");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("actor");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    let mk = |log_id: i64| NewNotification {
        workspace_id: ws.id,
        member_id: recipient.id,
        kind: EventKind::MentionRecorded,
        source_log_id: log_id,
        channel_id: Some(channel.id),
        thread_id: Some(thread.id),
        message_id: None,
        actor_id: Some(actor.id),
    };

    // Fields round-trip on create.
    let n1 = store.create_notification(mk(1)).await.expect("create n1");
    assert_eq!(n1.member_id, recipient.id);
    assert_eq!(n1.kind, EventKind::MentionRecorded);
    assert_eq!(n1.source_log_id, 1);
    assert_eq!(n1.channel_id, Some(channel.id));
    assert_eq!(n1.thread_id, Some(thread.id));
    assert_eq!(n1.actor_id, Some(actor.id));
    assert!(n1.read_at.is_none(), "new notification is unread");

    let n2 = store.create_notification(mk(2)).await.expect("create n2");

    // Both count as unread; both list.
    assert_eq!(
        store
            .unread_notification_count(recipient.id)
            .await
            .expect("count"),
        2
    );
    let all = store
        .list_notifications(recipient.id, false, 10)
        .await
        .expect("list all");
    assert_eq!(all.len(), 2);
    let ids: std::collections::HashSet<_> = all.iter().map(|n| n.id).collect();
    assert!(ids.contains(&n1.id) && ids.contains(&n2.id));

    // limit bounds the page.
    assert_eq!(
        store
            .list_notifications(recipient.id, false, 1)
            .await
            .expect("list limit")
            .len(),
        1
    );

    // Mark one read: idempotent, recipient-scoped, and unread drops.
    assert!(store
        .mark_notification_read(recipient.id, n1.id)
        .await
        .expect("mark n1"));
    assert!(
        store
            .mark_notification_read(recipient.id, n1.id)
            .await
            .expect("re-mark n1"),
        "re-marking an existing notification is idempotent-true"
    );
    // A different member can't mark this recipient's notification.
    assert!(
        !store
            .mark_notification_read(actor.id, n2.id)
            .await
            .expect("cross-member mark"),
        "another member cannot mark this recipient's notification"
    );
    assert_eq!(
        store
            .unread_notification_count(recipient.id)
            .await
            .expect("count after mark"),
        1
    );
    let unread = store
        .list_notifications(recipient.id, true, 10)
        .await
        .expect("list unread");
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].id, n2.id, "only the still-unread one remains");

    // Marking an unknown id is false, not an error.
    assert!(!store
        .mark_notification_read(recipient.id, NotificationId::new())
        .await
        .expect("mark unknown"));

    // mark_all clears the badge.
    assert_eq!(
        store
            .mark_all_notifications_read(recipient.id)
            .await
            .expect("mark all"),
        1,
        "one unread remained"
    );
    assert_eq!(
        store
            .unread_notification_count(recipient.id)
            .await
            .expect("count after all"),
        0
    );

    // Another member is isolated.
    assert_eq!(
        store
            .unread_notification_count(actor.id)
            .await
            .expect("actor count"),
        0
    );
    assert!(store
        .list_notifications(actor.id, false, 10)
        .await
        .expect("actor list")
        .is_empty());
}

#[tokio::test]
async fn notifications_create_list_mark_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn notifications_create_list_mark_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
