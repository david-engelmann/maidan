//! A SCIM delete removes the person: the member is tombstoned and its OIDC
//! identities are deleted in the same transaction as the token revocation,
//! so the same login cannot sign straight back in. Two tenants on both
//! backends: deleting the member in workspace A leaves workspace B's member,
//! with the same email and the same subject, signing in as before. Postgres
//! skips when Docker is unavailable.

use std::sync::Arc;
use std::time::Duration;

use maidan_server::error::ApiError;
use maidan_server::oidc::{resolve_member_for_login, LoginMember, NOT_PROVISIONED};
use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    AuditScope, MemberId, MemberKind, NewAuditEvent, NewMember, NewWorkspace, WorkspaceId,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

const ISSUER: &str = "https://idp.example";
const EMAIL: &str = "ada@example.com";
const SUBJECT: &str = "sub-ada";

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

/// A workspace whose member was provisioned over SCIM, under the verified
/// email as its handle, and has signed in once (linked by email).
async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let (member, _) = store
        .scim_provision_audited(
            NewMember {
                workspace_id: ws,
                handle: EMAIL.into(),
                display_name: None,
                kind: MemberKind::Human,
            },
            Some("ext-ada"),
            true,
            Box::new(move |(m, _)| NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "scim.user.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(m.id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    let (signed_in, how) = sign_in(store, ws).await.unwrap();
    assert_eq!((signed_in, how), (member.id, LoginMember::Linked));
    (ws, member.id)
}

/// A sign-in by the same IdP user, with email linking on and auto-provision
/// off, as in a SCIM-managed workspace.
async fn sign_in(store: &dyn Store, ws: WorkspaceId) -> Result<(MemberId, LoginMember), ApiError> {
    resolve_member_for_login(store, ws, ISSUER, SUBJECT, Some(EMAIL), true, false, Some(ISSUER)).await
}

async fn a_scim_deleted_member_cannot_sign_back_in_while_another_tenants_can(
    store: Arc<dyn Store>,
) {
    let store = store.as_ref();
    let (a, a_member) = tenant(store, "Alpha").await;
    let (b, b_member) = tenant(store, "Bravo").await;

    let deleted = store
        .scim_deprovision_audited(
            a,
            a_member,
            NewAuditEvent {
                scope: AuditScope::Workspace(a),
                actor_id: None,
                action: "scim.user.delete".into(),
                target_kind: Some("member".into()),
                target_id: Some(a_member.0),
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    assert!(deleted);

    let member = store.get_member(a_member).await.unwrap();
    assert!(member.tombstoned_at.is_some(), "the member is tombstoned");
    assert!(
        matches!(
            store.get_oidc_identity(a, ISSUER, SUBJECT).await,
            Err(StoreError::NotFound)
        ),
        "its identity is gone"
    );
    match sign_in(store, a).await {
        Err(ApiError::Forbidden(msg)) => assert_eq!(msg, NOT_PROVISIONED),
        other => panic!("a deleted member signed back in: {other:?}"),
    }

    let untouched = store.get_member(b_member).await.unwrap();
    assert!(untouched.tombstoned_at.is_none());
    assert_eq!(
        sign_in(store, b).await.unwrap(),
        (b_member, LoginMember::Existing),
        "the other tenant's member, same email and subject, still signs in"
    );
}

#[tokio::test]
async fn a_scim_deleted_member_cannot_sign_back_in_while_another_tenants_can_on_sqlite() {
    a_scim_deleted_member_cannot_sign_back_in_while_another_tenants_can(sqlite().await).await;
}

#[tokio::test]
async fn a_scim_deleted_member_cannot_sign_back_in_while_another_tenants_can_on_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

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
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    a_scim_deleted_member_cannot_sign_back_in_while_another_tenants_can(Arc::new(
        PostgresStore::for_tests(pool),
    ))
    .await;
}
