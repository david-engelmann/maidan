//! Hard-delete tombstoned messages (Track V.2).

use maidan_store::{configure_sqlite_pool, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn purge_removes_only_tombstoned_messages() {
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
            kind: MemberKind::Agent,
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
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            title: None,
            parent_thread_id: None,
        })
        .await
        .expect("thread");
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "secret".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("msg");

    assert!(store.purge_message(msg.id).await.is_err());

    store.tombstone_message(msg.id).await.expect("tombstone");
    store.purge_message(msg.id).await.expect("purge");

    assert!(store.get_message(msg.id).await.is_err());
}
