//! Store-level thread transitions via `maidan-fsm`.

mod common;

use maidan_fsm::ThreadAction;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadState};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn transition_thread_advances_state_and_logs_row() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool.clone());

    let ws = store
        .create_workspace(NewWorkspace {
            name: "t-ws".to_string(),
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
            name: "t-ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            title: None,
        })
        .await
        .expect("thread");
    assert_eq!(thread.state, ThreadState::Open);

    let r1 = store
        .transition_thread(thread.id, actor.id, ThreadAction::StartReview)
        .await
        .expect("start_review");
    assert_eq!(r1.from_state, ThreadState::Open);
    assert_eq!(r1.to_state, ThreadState::InReview);

    let r2 = store
        .transition_thread(thread.id, actor.id, ThreadAction::Close)
        .await
        .expect("close");
    assert_eq!(r2.to_state, ThreadState::Closed);

    let err = store
        .transition_thread(thread.id, actor.id, ThreadAction::StartReview)
        .await
        .expect_err("illegal from closed");
    assert!(matches!(err, maidan_store::StoreError::Conflict(_)));
}
