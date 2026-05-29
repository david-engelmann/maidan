//! Inbox cursor and enriched member inbox (Cluster 40).

use std::sync::Arc;

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{InboxItemKind, MemberKind, NewMember, NewMessage, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn store() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::new(pool))
}

#[tokio::test]
async fn inbox_unread_count_and_mark_read_on_sqlite() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "inbox".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(maidan_types::NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: "ping".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    store.record_mention(msg.id, bot.id).await.unwrap();

    let inbox = store.list_member_inbox(bot.id, 10).await.unwrap();
    assert_eq!(inbox.unread_count, 1);
    assert_eq!(inbox.items.len(), 1);
    assert!(inbox.items[0].unread);
    assert_eq!(inbox.items[0].kind, InboxItemKind::Mention);

    store
        .advance_inbox_last_read_at(bot.id, inbox.items[0].created_at)
        .await
        .unwrap();
    let inbox2 = store.list_member_inbox(bot.id, 10).await.unwrap();
    assert_eq!(inbox2.unread_count, 0);
    assert!(!inbox2.items[0].unread);
}

#[tokio::test]
async fn advance_inbox_cursor_is_monotonic() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cursor".into(),
        })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let t0 = chrono::Utc::now();
    let t1 = t0 + chrono::Duration::seconds(1);
    store.advance_inbox_last_read_at(bot.id, t1).await.unwrap();
    let after = store.advance_inbox_last_read_at(bot.id, t0).await.unwrap();
    assert!(after >= t1);
}
