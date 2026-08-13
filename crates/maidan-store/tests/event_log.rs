//! Persistent event log append + replay.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventKind, MemberKind, NewChannel, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn append_and_replay_events_in_order() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "log-ws".to_string(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "log-ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");

    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("append1");
    let e2 = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .expect("append2");

    assert_eq!(e1.kind, EventKind::MemberJoined);
    assert_eq!(e2.kind, EventKind::ChannelCreated);
    assert!(e2.id > e1.id);

    let page = store.list_events_after(ws.id, 0, 10).await.expect("list");
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, e1.id);
    assert_eq!(page[1].id, e2.id);

    let tail = store
        .list_events_after(ws.id, e1.id, 10)
        .await
        .expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, e2.id);
}

#[tokio::test]
async fn list_events_after_stable_gates_on_insert_time() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "stable-ws".to_string(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");

    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("append1");
    let e2 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member,
        })
        .await
        .expect("append2");

    // A horizon in the past treats the just-inserted rows as not-yet-stable.
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let none = store
        .list_events_after_stable(ws.id, 0, past, 10)
        .await
        .expect("stable past");
    assert!(
        none.is_empty(),
        "rows inserted now must be excluded by a past horizon, got {}",
        none.len()
    );

    // A horizon in the future treats them all as stable, in id order, honoring after_id.
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let all = store
        .list_events_after_stable(ws.id, 0, future, 10)
        .await
        .expect("stable future");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, e1.id);
    assert_eq!(all[1].id, e2.id);

    let tail = store
        .list_events_after_stable(ws.id, e1.id, future, 10)
        .await
        .expect("stable tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, e2.id);
}

#[tokio::test]
async fn get_stored_event_returns_row_and_missing_is_not_found() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "get-log".to_string(),
        })
        .await
        .expect("ws");
    let stored = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: "u".to_string(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .expect("member"),
        })
        .await
        .expect("append");

    let fetched = store.get_stored_event(stored.id).await.expect("get");
    assert_eq!(fetched.id, stored.id);
    assert_eq!(fetched.kind, EventKind::MemberJoined);

    let err = store.get_stored_event(999_999).await.unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

/// Cluster 205: `*_with_event` commits the domain row and its event atomically —
/// after the single call, both the row and the durable event exist.
#[tokio::test]
async fn create_with_event_commits_row_and_event() {
    use maidan_types::NewThread;
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace {
            name: "tx-outbox".to_string(),
        })
        .await
        .expect("ws");

    let (ch, ch_event) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "atomic".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel + event");
    assert_eq!(ch_event.kind, EventKind::ChannelCreated);
    // The channel row committed.
    assert_eq!(
        store.get_channel(ch.id).await.expect("get channel").id,
        ch.id
    );

    let (th, th_event) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".to_string()),
        })
        .await
        .expect("thread + event");
    assert_eq!(th_event.kind, EventKind::ThreadCreated);
    assert_eq!(store.get_thread(th.id).await.expect("get thread").id, th.id);

    // Both events are durably in the log — the returned events are the logged ones.
    let events = store
        .list_events_after(ws.id, 0, 100)
        .await
        .expect("events");
    assert!(events
        .iter()
        .any(|e| e.id == ch_event.id && e.kind == EventKind::ChannelCreated));
    assert!(events
        .iter()
        .any(|e| e.id == th_event.id && e.kind == EventKind::ThreadCreated));
}
