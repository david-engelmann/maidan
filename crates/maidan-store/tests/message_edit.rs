//! Message body/metadata edit with `edited_at` (Cluster 29).

use maidan_store::{configure_sqlite_pool, SqliteStore, Store};
use maidan_types::{
    EditMessage, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn edit_message_sets_body_and_edited_at() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("pragmas");
    maidan_store::run_sqlite_migrations(&pool)
        .await
        .expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "original".into(),
            metadata: serde_json::json!({"k": 1}),
        })
        .await
        .expect("post");

    let updated = store
        .edit_message(
            msg.id,
            member.id,
            EditMessage {
                body: "revised".into(),
                metadata: serde_json::json!({"k": 2}),
            },
        )
        .await
        .expect("edit");
    assert_eq!(updated.body, "revised");
    assert_eq!(updated.metadata["k"], 2);
    assert!(updated.edited_at.is_some());

    let history = store.list_message_edits(msg.id, 10).await.expect("edits");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].body_before, "original");
    assert_eq!(history[0].body_after, "revised");
    assert_eq!(history[0].editor_id, member.id);

    let fetched = store.get_message(msg.id).await.expect("get");
    assert_eq!(fetched.body, "revised");
}

#[tokio::test]
async fn edit_message_rejects_tombstoned() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("pragmas");
    maidan_store::run_sqlite_migrations(&pool)
        .await
        .expect("migrate");
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "x".into(),
            metadata: serde_json::json!({}),
        })
        .await
        .expect("post");
    store.tombstone_message(msg.id).await.expect("tombstone");

    let err = store
        .edit_message(
            msg.id,
            member.id,
            EditMessage {
                body: "nope".into(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect_err("tombstoned");
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}
