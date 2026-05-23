//! Nested thread HSM: child state cannot outrun parent.

use maidan_fsm::ThreadAction;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadState};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn child_cannot_advance_beyond_open_parent() {
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
            name: "hsm-ws".to_string(),
        })
        .await
        .expect("ws");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "hsm-ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let parent = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("parent".to_string()),
        })
        .await
        .expect("parent");
    let child = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: Some(parent.id),
            title: Some("child".to_string()),
        })
        .await
        .expect("child");

    let err = store
        .transition_thread(child.id, actor.id, ThreadAction::StartReview)
        .await
        .expect_err("child ahead of open parent");
    assert!(matches!(err, maidan_store::StoreError::Conflict(_)));

    store
        .transition_thread(parent.id, actor.id, ThreadAction::StartReview)
        .await
        .expect("parent to in_review");
    store
        .transition_thread(child.id, actor.id, ThreadAction::StartReview)
        .await
        .expect("child may match parent in_review");

    let child = store.get_thread(child.id).await.expect("get child");
    assert_eq!(child.state, ThreadState::InReview);
}
