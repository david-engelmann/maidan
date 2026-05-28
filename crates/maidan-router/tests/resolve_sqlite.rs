//! Hierarchy resolution against an in-memory SQLite store.

use maidan_router::{resolve_channel_context, resolve_message_chain, resolve_thread_context};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite memory");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

#[tokio::test]
async fn resolve_thread_and_message_chain_match_channel_workspace() {
    let store = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "router-test".into(),
        })
        .await
        .expect("workspace");
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
            title: None,
            parent_thread_id: None,
        })
        .await
        .expect("thread");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "hello".into(),
            metadata: Default::default(),
        })
        .await
        .expect("message");

    let channel_ctx = resolve_channel_context(&store, ch.id)
        .await
        .expect("channel ctx");
    assert_eq!(channel_ctx.workspace_id, ws.id);
    assert_eq!(channel_ctx.channel_id, ch.id);

    let thread_ctx = resolve_thread_context(&store, thread.id)
        .await
        .expect("thread ctx");
    assert_eq!(thread_ctx.workspace_id, ws.id);
    assert_eq!(thread_ctx.channel_id, ch.id);
    assert_eq!(thread_ctx.thread_id, thread.id);

    let chain = resolve_message_chain(&store, msg.id)
        .await
        .expect("message chain");
    assert_eq!(chain.workspace_id, ws.id);
    assert_eq!(chain.channel_id, ch.id);
    assert_eq!(chain.thread_id, thread.id);
}
