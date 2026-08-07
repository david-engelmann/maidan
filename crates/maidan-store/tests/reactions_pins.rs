//! Reactions and thread pins (Cluster 41).

use std::sync::Arc;

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewMember, NewMessage, NewPin, NewReaction, NewWorkspace};
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
async fn reactions_add_list_remove() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "rx".into() })
        .await
        .unwrap();
    let m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(maidan_types::NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: m.id,
            body: "hi".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    store
        .add_reaction(NewReaction {
            message_id: msg.id,
            member_id: m.id,
            emoji: "👍".into(),
        })
        .await
        .unwrap();
    let list = store.list_reactions_for_message(msg.id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].emoji, "👍");

    assert!(store.remove_reaction(msg.id, m.id, "👍").await.unwrap());
    assert!(store
        .list_reactions_for_message(msg.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn pin_message_requires_same_thread() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "pin".into() })
        .await
        .unwrap();
    let m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th1 = store
        .create_thread(maidan_types::NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t1".into()),
        })
        .await
        .unwrap();
    let th2 = store
        .create_thread(maidan_types::NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t2".into()),
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: th1.id,
            author_id: m.id,
            body: "only in t1".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let err = store
        .pin_message(NewPin {
            thread_id: th2.id,
            message_id: msg.id,
            member_id: m.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::InvalidInput(_)));

    store
        .pin_message(NewPin {
            thread_id: th1.id,
            message_id: msg.id,
            member_id: m.id,
        })
        .await
        .unwrap();
    let pins = store.list_pins_for_thread(th1.id).await.unwrap();
    assert_eq!(pins.len(), 1);
    assert!(store.unpin_message(th1.id, msg.id).await.unwrap());
}
