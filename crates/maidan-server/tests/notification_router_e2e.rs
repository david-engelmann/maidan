//! Cluster 238: the notification router resolves a `MentionRecorded` event to a
//! per-recipient notification row, and dedups replays / multi-replica delivery.

use std::sync::Arc;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{notification_router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventKind, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn router_writes_a_notification_per_mention_and_dedups() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    let ws = store
        .create_workspace(NewWorkspace { name: "n".into() })
        .await
        .unwrap();
    let mentioned = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "mentioned".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();

    let message_id = maidan_types::MessageId::new();
    let mention = Event::MentionRecorded {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        thread_id: thread.id,
        message_id,
        member_id: mentioned.id,
    };

    // No notifications before routing.
    assert!(store
        .list_notifications(mentioned.id, false, 10)
        .await
        .unwrap()
        .is_empty());

    // Routing the mention writes one notification with the resolved context.
    notification_router::route_event(&state, 1, &mention)
        .await
        .unwrap();
    let notes = store
        .list_notifications(mentioned.id, false, 10)
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    let n = &notes[0];
    assert_eq!(n.member_id, mentioned.id);
    assert_eq!(n.kind, EventKind::MentionRecorded);
    assert_eq!(n.source_log_id, 1);
    assert_eq!(
        n.channel_id,
        Some(channel.id),
        "channel resolved from the thread"
    );
    assert_eq!(n.thread_id, Some(thread.id));
    assert_eq!(n.message_id, Some(message_id));
    assert!(n.read_at.is_none());
    assert_eq!(
        store.unread_notification_count(mentioned.id).await.unwrap(),
        1
    );

    // Re-routing the SAME event (a replay or a second replica) does not
    // double-notify — dedup on (member_id, source_log_id).
    notification_router::route_event(&state, 1, &mention)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        1,
        "a replay of the same event is deduped"
    );

    // A distinct event (new log_id) is a distinct notification.
    notification_router::route_event(&state, 2, &mention)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2
    );

    // A non-mention event routes to nothing.
    let ready = Event::ThreadReady {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        thread: thread.clone(),
    };
    notification_router::route_event(&state, 3, &ready)
        .await
        .unwrap();
    assert_eq!(
        store
            .list_notifications(mentioned.id, false, 10)
            .await
            .unwrap()
            .len(),
        2,
        "a non-mention event produces no notification (yet)"
    );
}
