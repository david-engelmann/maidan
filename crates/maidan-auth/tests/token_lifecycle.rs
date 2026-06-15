//! Bearer resolution against a real store: minting, capability propagation,
//! revocation, expiry, and the constant-time rejection of forged bearers.
//! Exercises the SQLite backend (skips nothing — `sqlite::memory:` is always
//! available); the resolution path is backend-agnostic.

use chrono::{Duration, Utc};
use maidan_auth::{hash_secret, resolve_bearer, resolve_peer_bearer, AuthError, TokenSecret};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store, StoreError};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMember, NewPeer, NewWorkspace, WorkspaceId,
};

async fn store_with_member() -> (SqliteStore, WorkspaceId, MemberId) {
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

    let ws = store
        .create_workspace(NewWorkspace {
            name: "auth-suite".to_string(),
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
    (store, ws.id, member.id)
}

fn mint(
    workspace_id: WorkspaceId,
    member_id: MemberId,
    secret: &TokenSecret,
    capabilities: Vec<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> NewApiToken {
    NewApiToken {
        workspace_id,
        member_id,
        app_installation_id: None,
        token_hash: hash_secret(secret.as_str()),
        label: Some("suite".to_string()),
        capabilities,
        expires_at,
    }
}

#[tokio::test]
async fn resolve_bearer_returns_context_with_minted_capabilities() {
    let (store, workspace_id, member_id) = store_with_member().await;
    let secret = TokenSecret::generate();
    let caps = vec!["workspace:read".to_string(), "message:post".to_string()];
    store
        .create_api_token(mint(workspace_id, member_id, &secret, caps, None))
        .await
        .expect("mint");

    let ctx = resolve_bearer(&store, secret.as_str())
        .await
        .expect("resolve");
    assert_eq!(ctx.member_id, member_id);
    assert_eq!(ctx.workspace_id, workspace_id);
    assert!(ctx.token_id.is_some());
    assert!(!ctx.bypass);
    assert!(ctx.has_capability("workspace:read"));
    assert!(ctx.has_capability("message:post"));
    assert!(!ctx.has_capability("token:admin"));
    assert!(ctx.ensure_workspace(workspace_id).is_ok());
}

#[tokio::test]
async fn resolve_bearer_rejects_an_unknown_secret() {
    let (store, workspace_id, member_id) = store_with_member().await;
    let real = TokenSecret::generate();
    store
        .create_api_token(mint(
            workspace_id,
            member_id,
            &real,
            vec!["workspace:read".to_string()],
            None,
        ))
        .await
        .expect("mint");

    // A different, never-minted secret hashes to an absent row.
    let forged = TokenSecret::generate();
    let err = resolve_bearer(&store, forged.as_str())
        .await
        .expect_err("forged bearer must not resolve");
    assert!(matches!(err, AuthError::Store(StoreError::NotFound)));
}

#[tokio::test]
async fn resolve_bearer_fails_after_revocation() {
    let (store, workspace_id, member_id) = store_with_member().await;
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(mint(
            workspace_id,
            member_id,
            &secret,
            vec!["workspace:read".to_string()],
            None,
        ))
        .await
        .expect("mint");

    // Valid before revocation...
    resolve_bearer(&store, secret.as_str())
        .await
        .expect("resolves before revoke");

    store.revoke_api_token(token.id).await.expect("revoke");

    // ...and gone after.
    let err = resolve_bearer(&store, secret.as_str())
        .await
        .expect_err("revoked bearer must not resolve");
    assert!(matches!(err, AuthError::Store(StoreError::NotFound)));
}

#[tokio::test]
async fn resolve_bearer_fails_for_an_expired_token() {
    let (store, workspace_id, member_id) = store_with_member().await;
    let secret = TokenSecret::generate();
    let past = Utc::now() - Duration::hours(1);
    store
        .create_api_token(mint(
            workspace_id,
            member_id,
            &secret,
            vec!["workspace:read".to_string()],
            Some(past),
        ))
        .await
        .expect("mint expired");

    let err = resolve_bearer(&store, secret.as_str())
        .await
        .expect_err("expired bearer must not resolve");
    assert!(matches!(err, AuthError::Store(StoreError::NotFound)));
}

#[tokio::test]
async fn resolve_peer_bearer_round_trips_then_rejects_a_forged_secret() {
    let (store, workspace_id, _member_id) = store_with_member().await;
    let peer_secret = TokenSecret::generate();
    let created = store
        .create_peer(NewPeer {
            workspace_id,
            remote_workspace_id: workspace_id,
            name: "east".to_string(),
            base_url: "https://east.example".to_string(),
            token_hash: hash_secret(peer_secret.as_str()),
            outbound_secret_ciphertext: None,
        })
        .await
        .expect("create peer");

    let resolved = resolve_peer_bearer(&store, peer_secret.as_str())
        .await
        .expect("resolve peer");
    assert_eq!(resolved.id, created.id);
    assert_eq!(resolved.workspace_id, workspace_id);

    let forged = TokenSecret::generate();
    let err = resolve_peer_bearer(&store, forged.as_str())
        .await
        .expect_err("forged peer bearer must not resolve");
    assert!(matches!(err, AuthError::Store(StoreError::NotFound)));
}
