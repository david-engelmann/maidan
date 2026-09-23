//! How a borrowed token resolves. Two properties that do not depend on any one
//! route: a borrowed context cannot hold authority, and a borrowed token is
//! refused once its grant is dead even if revocation never reached the token.

use chrono::{Duration, Utc};
use maidan_auth::{capability, hash_secret, resolve_bearer, AuthContext, AuthError, TokenSecret};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, DelegationGrantId, MemberId, MemberKind, NewApiToken, NewDelegationGrant,
    NewMember, NewWorkspace, WorkspaceId,
};

#[test]
fn a_borrowed_context_drops_authority_capabilities() {
    let auth = AuthContext::from_delegated_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        MemberId(uuid::Uuid::new_v4()),
        MemberId(uuid::Uuid::new_v4()),
        WorkspaceId(uuid::Uuid::new_v4()),
        DelegationGrantId(uuid::Uuid::new_v4()),
        vec![
            capability::WORKSPACE_READ.into(),
            capability::TOKEN_ADMIN.into(),
            capability::SECRET_ADMIN.into(),
        ],
    );
    assert_eq!(
        auth.capabilities(),
        [capability::WORKSPACE_READ.to_string()]
    );
    assert!(!auth.has_capability(capability::TOKEN_ADMIN));
}

/// Revocation cascades to every token exchanged from a grant, but resolution
/// does not rely on that: the active-token lookup itself requires the grant to be
/// unrevoked and unexpired. The grant is killed here directly, leaving the token
/// row untouched, to pin that resolution checks the grant rather than trusting
/// the cascade ran.
#[tokio::test]
async fn a_borrowed_token_is_refused_once_its_grant_is_dead() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool.clone());
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["subject", "delegate", "authorizer"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let (subject, delegate, authorizer) = (ids[0], ids[1], ids[2]);

    for kill in [
        "UPDATE maidan_delegation_grants SET revoked_at = ?1 WHERE id = ?2",
        "UPDATE maidan_delegation_grants SET expires_at = ?1 WHERE id = ?2",
    ] {
        let grant = store
            .create_delegation_grant(NewDelegationGrant {
                workspace_id: ws.id,
                subject_id: subject,
                delegate_id: delegate,
                capabilities: vec![capability::WORKSPACE_READ.into()],
                purpose: "test".into(),
                authorized_by: authorizer,
                expires_at: Utc::now() + Duration::hours(1),
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        let token = store
            .create_delegated_api_token(
                NewApiToken {
                    workspace_id: ws.id,
                    member_id: subject,
                    app_installation_id: None,
                    token_hash: hash_secret(secret.as_str()),
                    label: None,
                    capabilities: vec![capability::WORKSPACE_READ.into()],
                    expires_at: Some(Utc::now() + Duration::minutes(15)),
                },
                grant.id,
                delegate,
                None,
            )
            .await
            .unwrap();
        assert!(resolve_bearer(&store, secret.as_str()).await.is_ok());

        sqlx::query(kill)
            .bind((Utc::now() - Duration::seconds(1)).to_rfc3339())
            .bind(grant.id.0)
            .execute(&pool)
            .await
            .unwrap();
        let row = store
            .get_active_api_token_by_hash(&hash_secret(secret.as_str()))
            .await;
        assert!(
            row.is_err(),
            "the lookup must refuse a token whose grant is dead"
        );
        assert!(
            store
                .get_api_token(token.id)
                .await
                .unwrap()
                .revoked_at
                .is_none(),
            "setup: the token row itself must be untouched, or this proves nothing"
        );
        assert!(
            matches!(
                resolve_bearer(&store, secret.as_str()).await,
                Err(AuthError::Store(StoreError::NotFound))
            ),
            "a borrowed token must be refused once its grant is dead: {kill}"
        );
    }
}
