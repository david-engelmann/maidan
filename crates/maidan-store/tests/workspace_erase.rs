//! Workspace full erasure deletes the workspace row (Cluster 53).

use maidan_auth::hash_secret;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store, StoreError};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite_store() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::new(pool)
}

#[tokio::test]
async fn erase_workspace_removes_row_and_peers() {
    let store = sqlite_store().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "erase-me".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            label: None,
            token_hash: hash_secret("test-token"),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let peer = store
        .create_peer(NewPeer {
            workspace_id: ws.id,
            remote_workspace_id: ws.id,
            name: "peer".into(),
            base_url: "https://peer.example".into(),
            token_hash: "a".repeat(64),
            outbound_secret_ciphertext: None,
        })
        .await
        .unwrap();

    let result = store.erase_workspace(ws.id).await.unwrap();
    assert!(result.workspace_erased);
    assert!(result.purge.api_tokens_revoked >= 1);

    assert!(matches!(
        store.get_workspace(ws.id).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.get_peer(peer.id).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn erase_unknown_workspace_is_not_found() {
    let store = sqlite_store().await;
    assert!(matches!(
        store
            .erase_workspace(WorkspaceId(uuid::Uuid::new_v4()))
            .await,
        Err(StoreError::NotFound)
    ));
}
