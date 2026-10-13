//! OAuth refresh tokens, lineage revocation and access-token revocation
//! (`docs/OAuth.md`, phase three): one suite, run on SQLite and Postgres.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const RESOURCE: &str = "https://maidan.example.com/mcp/streamable";

struct Seed {
    ws: WorkspaceId,
    grant: OAuthGrant,
}

async fn seed(store: &dyn Store, name: &str, client_id: &str) -> Seed {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-owner"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    if store
        .get_oauth_client_by_client_id(client_id)
        .await
        .unwrap()
        .is_none()
    {
        store
            .create_oauth_client(NewOAuthClient {
                client_id: client_id.into(),
                name: client_id.into(),
                redirect_uris: vec!["https://client.example/callback".into()],
                client_secret_hash: None,
                allowed_scopes: vec!["workspace:read".into(), "workspace:write".into()],
            })
            .await
            .unwrap();
    }
    let grant = store
        .create_oauth_grant(NewOAuthGrant {
            client_id: client_id.into(),
            member_id: member.id,
            workspace_id: ws.id,
            scope: vec!["workspace:read".into(), "workspace:write".into()],
            lineage_id: uuid::Uuid::now_v7(),
        })
        .await
        .unwrap();
    Seed { ws: ws.id, grant }
}

fn mint_audit(action: &'static str) -> maidan_store::AuditFor<ApiToken> {
    Box::new(move |token| NewAuditEvent {
        scope: AuditScope::Workspace(token.workspace_id),
        actor_id: None,
        action: action.into(),
        target_kind: Some("api_token".into()),
        target_id: Some(token.id.0),
        metadata: serde_json::json!({}),
    })
}

fn lineage_audit() -> maidan_store::AuditFor<OAuthLineageRevocation> {
    Box::new(|revoked| NewAuditEvent {
        scope: revoked
            .workspace_id
            .map_or(AuditScope::Instance, AuditScope::Workspace),
        actor_id: None,
        action: "oauth_lineage.revoke".into(),
        target_kind: Some("oauth_lineage".into()),
        target_id: Some(revoked.lineage_id),
        metadata: serde_json::json!({}),
    })
}

/// Mint the first access and refresh pair under `seed`'s grant.
async fn mint_pair(store: &dyn Store, seed: &Seed, access_hash: &str, refresh_hash: &str) {
    store
        .mint_oauth_token_pair_audited(
            NewApiToken {
                workspace_id: seed.ws,
                member_id: seed.grant.member_id,
                app_installation_id: None,
                token_hash: access_hash.into(),
                label: Some("oauth:test".into()),
                capabilities: seed.grant.scope.clone(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            },
            seed.grant.id,
            NewOAuthRefreshToken {
                token_hash: refresh_hash.into(),
                grant_id: seed.grant.id,
                lineage_id: seed.grant.lineage_id,
                client_id: seed.grant.client_id.clone(),
                resource: RESOURCE.into(),
                expires_at: Utc::now() + chrono::Duration::days(30),
            },
            mint_audit("oauth_token.mint"),
        )
        .await
        .unwrap();
}

fn rotation(presented: &str, access: &str, refresh: &str, caps: &[&str]) -> OAuthRefreshRotation {
    OAuthRefreshRotation {
        presented_hash: presented.into(),
        access_token_hash: access.into(),
        access_label: Some("oauth:test".into()),
        access_capabilities: caps.iter().map(|c| c.to_string()).collect(),
        access_expires_at: Utc::now() + chrono::Duration::hours(1),
        refresh_hash: refresh.into(),
        refresh_expires_at: Utc::now() + chrono::Duration::days(30),
    }
}

async fn rotate(store: &dyn Store, r: OAuthRefreshRotation) -> OAuthRefreshOutcome {
    store
        .rotate_oauth_refresh_token(r, mint_audit("oauth_token.refresh"), lineage_audit())
        .await
        .unwrap()
}

async fn assert_rotation_and_reuse(store: &dyn Store) {
    let a = seed(store, "refresh-a", "client-x").await;
    let b = seed(store, "refresh-b", "client-x").await;
    mint_pair(store, &a, "a-access-1", "a-refresh-1").await;
    mint_pair(store, &b, "b-access-1", "b-refresh-1").await;

    let stored = store
        .get_oauth_refresh_token("a-refresh-1")
        .await
        .unwrap()
        .expect("the refresh token is stored");
    assert_eq!(stored.grant_id, a.grant.id);
    assert_eq!(stored.lineage_id, a.grant.lineage_id);
    assert_eq!(stored.resource, RESOURCE);
    assert!(stored.used_at.is_none() && stored.revoked_at.is_none());

    // A rotation marks the presented token used and mints under the same
    // grant, member and workspace, whatever the caller would have liked.
    let OAuthRefreshOutcome::Rotated { access, refresh } = rotate(
        store,
        rotation(
            "a-refresh-1",
            "a-access-2",
            "a-refresh-2",
            &["workspace:read"],
        ),
    )
    .await
    else {
        panic!("a live refresh token rotates");
    };
    assert_eq!(access.workspace_id, a.ws);
    assert_eq!(access.member_id, a.grant.member_id);
    assert_eq!(access.oauth_grant_id, Some(a.grant.id));
    assert_eq!(access.capabilities, vec!["workspace:read".to_string()]);
    assert_eq!(refresh.lineage_id, a.grant.lineage_id);
    assert_eq!(refresh.grant_id, a.grant.id);
    assert!(store
        .get_oauth_refresh_token("a-refresh-1")
        .await
        .unwrap()
        .unwrap()
        .used_at
        .is_some());
    assert!(store
        .get_active_api_token_by_hash("a-access-2")
        .await
        .is_ok());

    // A scope wider than the grant is refused, and the token stays unused.
    let widened = store
        .rotate_oauth_refresh_token(
            rotation("a-refresh-2", "a-access-x", "a-refresh-x", &["token:mint"]),
            mint_audit("oauth_token.refresh"),
            lineage_audit(),
        )
        .await;
    assert!(matches!(widened, Err(StoreError::InvalidInput(_))));
    assert!(store
        .get_oauth_refresh_token("a-refresh-2")
        .await
        .unwrap()
        .unwrap()
        .used_at
        .is_none());

    // Presenting the used token again is a reuse: the whole lineage goes.
    let OAuthRefreshOutcome::Reused(revoked) = rotate(
        store,
        rotation(
            "a-refresh-1",
            "a-access-3",
            "a-refresh-3",
            &["workspace:read"],
        ),
    )
    .await
    else {
        panic!("a used refresh token is a reuse");
    };
    assert_eq!(revoked.lineage_id, a.grant.lineage_id);
    assert_eq!(revoked.workspace_id, Some(a.ws));
    assert_eq!(revoked.grants_revoked, 1);
    assert_eq!(revoked.access_tokens_revoked, 2);
    assert_eq!(revoked.refresh_tokens_revoked, 2);
    assert!(store.get_oauth_grant(a.grant.id).await.unwrap().is_none());
    for hash in ["a-access-1", "a-access-2"] {
        assert!(
            store.get_active_api_token_by_hash(hash).await.is_err(),
            "{hash} stops working"
        );
    }
    assert!(store
        .get_oauth_refresh_token("a-refresh-2")
        .await
        .unwrap()
        .unwrap()
        .revoked_at
        .is_some());
    assert!(matches!(
        rotate(
            store,
            rotation(
                "a-refresh-2",
                "a-access-4",
                "a-refresh-4",
                &["workspace:read"]
            )
        )
        .await,
        OAuthRefreshOutcome::Invalid
    ));
    let audit = store.list_audit_for_workspace(a.ws, 50).await.unwrap();
    assert!(audit.iter().any(|e| e.action == "oauth_lineage.revoke"));
    assert!(audit.iter().any(|e| e.action == "oauth_token.refresh"));

    // The other tenant's lineage, under the same client, is untouched.
    assert!(store
        .get_active_api_token_by_hash("b-access-1")
        .await
        .is_ok());
    assert!(matches!(
        rotate(
            store,
            rotation(
                "b-refresh-1",
                "b-access-2",
                "b-refresh-2",
                &["workspace:read"]
            )
        )
        .await,
        OAuthRefreshOutcome::Rotated { .. }
    ));

    // Unknown and expired tokens change nothing.
    assert!(matches!(
        rotate(store, rotation("nope", "n-a", "n-r", &[])).await,
        OAuthRefreshOutcome::Invalid
    ));
    let c = seed(store, "refresh-c", "client-x").await;
    store
        .mint_oauth_token_pair_audited(
            NewApiToken {
                workspace_id: c.ws,
                member_id: c.grant.member_id,
                app_installation_id: None,
                token_hash: "c-access-1".into(),
                label: None,
                capabilities: c.grant.scope.clone(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            },
            c.grant.id,
            NewOAuthRefreshToken {
                token_hash: "c-refresh-1".into(),
                grant_id: c.grant.id,
                lineage_id: c.grant.lineage_id,
                client_id: "client-x".into(),
                resource: RESOURCE.into(),
                expires_at: Utc::now() - chrono::Duration::seconds(5),
            },
            mint_audit("oauth_token.mint"),
        )
        .await
        .unwrap();
    assert!(matches!(
        rotate(store, rotation("c-refresh-1", "c-a", "c-r", &[])).await,
        OAuthRefreshOutcome::Invalid
    ));
    assert!(store.get_oauth_grant(c.grant.id).await.unwrap().is_some());
}

async fn assert_code_reuse_and_access_revocation(store: &dyn Store) {
    let d = seed(store, "code-d", "client-y").await;
    mint_pair(store, &d, "d-access-1", "d-refresh-1").await;
    store
        .create_oauth_authorization_code(NewOAuthAuthorizationCode {
            code_hash: "d-code".into(),
            client_id: "client-y".into(),
            member_id: d.grant.member_id,
            workspace_id: d.ws,
            redirect_uri: "https://client.example/callback".into(),
            code_challenge: "challenge".into(),
            scope: d.grant.scope.clone(),
            resource: Some(RESOURCE.into()),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
            lineage_id: Some(d.grant.lineage_id),
        })
        .await
        .unwrap();

    // An unused code revokes nothing.
    assert!(store
        .revoke_oauth_lineage_for_used_code("d-code", lineage_audit())
        .await
        .unwrap()
        .is_none());
    assert!(store
        .consume_oauth_authorization_code("d-code")
        .await
        .unwrap()
        .is_some());

    // Access-token revocation is scoped to the client that owns it.
    assert!(store
        .revoke_oauth_access_token_audited(
            "d-access-1",
            "client-x",
            mint_audit("oauth_token.revoke")
        )
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_active_api_token_by_hash("d-access-1")
        .await
        .is_ok());
    let revoked = store
        .revoke_oauth_access_token_audited(
            "d-access-1",
            "client-y",
            mint_audit("oauth_token.revoke"),
        )
        .await
        .unwrap()
        .expect("the owning client revokes its token");
    assert_eq!(revoked.token_hash, "d-access-1");
    assert!(store
        .get_active_api_token_by_hash("d-access-1")
        .await
        .is_err());
    assert!(
        store.get_oauth_grant(d.grant.id).await.unwrap().is_some(),
        "revoking an access token leaves its grant"
    );

    // The used code, presented again, revokes the grant it minted under.
    let revoked = store
        .revoke_oauth_lineage_for_used_code("d-code", lineage_audit())
        .await
        .unwrap()
        .expect("a used code revokes its grant's lineage");
    assert_eq!(revoked.lineage_id, d.grant.lineage_id);
    assert_eq!(revoked.grants_revoked, 1);
    assert_eq!(revoked.refresh_tokens_revoked, 1);
    assert!(store.get_oauth_grant(d.grant.id).await.unwrap().is_none());

    // The member consents again: a new grant, a new lineage, for the same
    // client, member, workspace and scope. Replaying the old code revokes the
    // old lineage only, never the new grant, which never saw that code.
    let renewed = store
        .create_oauth_grant(NewOAuthGrant {
            client_id: "client-y".into(),
            member_id: d.grant.member_id,
            workspace_id: d.ws,
            scope: d.grant.scope.clone(),
            lineage_id: Uuid::now_v7(),
        })
        .await
        .unwrap();
    store
        .revoke_oauth_lineage_for_used_code("d-code", lineage_audit())
        .await
        .unwrap();
    assert!(
        store.get_oauth_grant(renewed.id).await.unwrap().is_some(),
        "a replayed code leaves a later grant for the same tuple alone"
    );

    // Revoking a lineage directly is idempotent.
    let again = store
        .revoke_oauth_lineage_audited(d.grant.lineage_id, lineage_audit())
        .await
        .unwrap();
    assert_eq!(again.grants_revoked, 0);
    assert_eq!(again.workspace_id, Some(d.ws));
}

async fn run_suite(store: &dyn Store) {
    assert_rotation_and_reuse(store).await;
    assert_code_reuse_and_access_revocation(store).await;
}

#[tokio::test]
async fn oauth_refresh_rotation_and_lineage_revocation_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    run_suite(&store).await;
}

#[tokio::test]
async fn oauth_refresh_rotation_and_lineage_revocation_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
