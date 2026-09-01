//! Workspace-wide message tombstone + hard-delete (Cluster 25).

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn purge_workspace_tombstones_then_deletes_all_messages() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "purge-ws".into(),
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
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: "hello".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: "world".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let result = store.purge_workspace_messages(ws.id).await.unwrap();
    assert_eq!(result.messages_tombstoned, 2);
    assert_eq!(result.messages_purged, 2);
    assert_eq!(result.embeddings_removed, 0);
    assert_eq!(result.references_removed, 0);
    assert_eq!(result.api_tokens_revoked, 0);
    assert_eq!(result.events_removed, 0);
    assert!(store.list_messages(th.id, 10).await.unwrap().is_empty());
}
