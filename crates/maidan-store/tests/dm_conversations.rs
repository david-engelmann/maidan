//! Direct message conversation store (SQLite).

use std::sync::Arc;

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
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
async fn open_dm_conversation_is_idempotent_for_same_pair() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "dm-ws".into(),
        })
        .await
        .unwrap();
    let a = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let b = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let first = store.open_dm_conversation(ws.id, a.id, b.id).await.unwrap();
    let second = store.open_dm_conversation(ws.id, b.id, a.id).await.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.thread_id, second.thread_id);
}

#[tokio::test]
async fn open_dm_rejects_self_conversation() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "solo".into(),
        })
        .await
        .unwrap();
    let a = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "solo".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let err = store
        .open_dm_conversation(ws.id, a.id, a.id)
        .await
        .unwrap_err();
    assert!(matches!(err, maidan_store::StoreError::InvalidInput(_)));
}
