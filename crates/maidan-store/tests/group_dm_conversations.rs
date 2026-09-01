//! Group DM store parity (Cluster 97).

use std::sync::Arc;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewMember, NewWorkspace};
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
async fn open_group_dm_requires_three_members() {
    let store = store().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "gdm".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for h in ["a", "b", "c"] {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: h.into(),
                display_name: None,
                kind: maidan_types::MemberKind::Human,
            })
            .await
            .unwrap();
        ids.push(m.id);
    }
    let group = store
        .open_group_dm_conversation(ws.id, &ids, Some("standup".into()))
        .await
        .unwrap();
    assert_eq!(group.member_ids.len(), 3);
    assert!(store.group_dm_has_member(group.id, ids[0]).await.unwrap());
    let listed = store
        .list_group_dm_conversations_for_member(ws.id, ids[0])
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let err = store
        .open_group_dm_conversation(ws.id, &[ids[0], ids[1]], None)
        .await;
    assert!(err.is_err());
}
