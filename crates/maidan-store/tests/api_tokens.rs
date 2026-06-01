//! API token store: create, lookup, revoke, expiry.

use chrono::{Duration, Utc};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};

async fn seed_member(store: &dyn Store) -> (WorkspaceId, maidan_types::MemberId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "auth-ws".to_string(),
        })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    (ws.id, member.id)
}

#[tokio::test]
async fn api_token_create_lookup_and_revoke() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let (workspace_id, member_id) = seed_member(&store).await;
    let token = store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: "a".repeat(64),
            label: Some("ci-bot".to_string()),
            capabilities: vec!["workspace:read".to_string(), "message:post".to_string()],
            expires_at: None,
        })
        .await
        .expect("create token");

    let fetched = store
        .get_active_api_token_by_hash(&token.token_hash)
        .await
        .expect("lookup");
    assert_eq!(fetched.id, token.id);
    assert_eq!(fetched.capabilities.len(), 2);

    let revoked = store.revoke_api_token(token.id).await.expect("revoke");
    assert!(revoked.revoked_at.is_some());

    let err = store
        .get_active_api_token_by_hash(&token.token_hash)
        .await
        .expect_err("revoked token must not resolve");
    assert!(matches!(err, maidan_store::StoreError::NotFound));
}

#[tokio::test]
async fn expired_api_token_is_not_active() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let (workspace_id, member_id) = seed_member(&store).await;
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: "b".repeat(64),
            label: None,
            capabilities: vec!["workspace:read".to_string()],
            expires_at: Some(Utc::now() - Duration::hours(1)),
        })
        .await
        .expect("create expired token");

    store
        .get_active_api_token_by_hash(&"b".repeat(64))
        .await
        .expect_err("expired token must not resolve");
}

#[tokio::test]
async fn api_token_hash_is_unique() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    let (workspace_id, member_id) = seed_member(&store).await;
    let hash = "c".repeat(64);
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash.clone(),
            label: None,
            capabilities: vec!["workspace:read".to_string()],
            expires_at: None,
        })
        .await
        .expect("first token");

    let channel = store
        .create_channel(NewChannel {
            workspace_id,
            name: "ch".to_string(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let _thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread");

    let err = store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash,
            label: None,
            capabilities: vec![],
            expires_at: None,
        })
        .await
        .expect_err("duplicate hash");
    assert!(matches!(err, maidan_store::StoreError::Conflict(_)));
}
