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
