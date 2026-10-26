//! A sign-in resolves its member, then creates its session. A SCIM delete or
//! deactivation can commit in between, so the session write itself refuses a
//! tombstoned or SCIM-inactive member, and a session already issued to one no
//! longer resolves. Two tenants on both backends: removing the member in
//! workspace A leaves workspace B's member, with the same email and subject,
//! signing in as before. Postgres skips when Docker is unavailable.

use std::sync::Arc;
use std::time::Duration;

use maidan_server::error::ApiError;
use maidan_server::oidc::{resolve_member_for_login, LoginMember};
use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    AuditScope, MaidanSession, MemberId, MemberKind, NewAuditEvent, NewMaidanSession, NewMember,
    NewWorkspace, WorkspaceId,
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
    resolve_member_for_login(store, ws, ISSUER, SUBJECT, Some(EMAIL), true, false, true).await
}

fn event(ws: WorkspaceId, member: MemberId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: Some("member".into()),
        target_id: Some(member.0),
        metadata: json!({}),
    }
}

/// The callback's last step: a session for the member it resolved.
async fn create_session(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
) -> Result<MaidanSession, StoreError> {
    store
        .create_session_audited(
            NewMaidanSession {
                workspace_id: ws,
                member_id: member,
                api_token_id: None,
                oidc_identity_id: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            },
            Box::new(move |s| NewAuditEvent {
                scope: AuditScope::Workspace(s.workspace_id),
                actor_id: Some(s.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(s.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
}

#[derive(Clone, Copy, Debug)]
enum Removal {
    Delete,
    Deactivate,
}

async fn remove(store: &dyn Store, ws: WorkspaceId, member: MemberId, how: Removal) {
    match how {
        Removal::Delete => assert!(store
            .scim_deprovision_audited(ws, member, event(ws, member, "scim.user.delete"))
            .await
            .unwrap()),
        Removal::Deactivate => assert!(store
            .scim_update_user_audited(
                ws,
                member,
                None,
                Some("ext-ada"),
                false,
                event(ws, member, "scim.user.update"),
            )
            .await
            .unwrap()
            .is_some()),
    }
}

async fn a_sign_in_racing_a_scim_removal_mints_no_session_while_another_tenants_does(
    store: Arc<dyn Store>,
    how: Removal,
) {
    let store = store.as_ref();
    let (a, a_member) = tenant(store, "Alpha").await;
    let (b, b_member) = tenant(store, "Bravo").await;
    let earlier = create_session(store, a, a_member).await.unwrap();

    // The callback resolved the member, then the SCIM removal committed.
    assert_eq!(
        sign_in(store, a).await.unwrap(),
        (a_member, LoginMember::Existing)
    );
    remove(store, a, a_member, how).await;

    match create_session(store, a, a_member).await {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("{how:?}: a session was minted for a removed member: {other:?}"),
    }
    assert!(
        matches!(
            store.get_session(earlier.id).await,
            Err(StoreError::NotFound)
        ),
        "{how:?}: a session issued before the removal no longer resolves"
    );

    assert_eq!(
        sign_in(store, b).await.unwrap(),
        (b_member, LoginMember::Existing)
    );
    let session = create_session(store, b, b_member).await.unwrap();
    assert_eq!(
        store.get_session(session.id).await.unwrap().member_id,
        b_member
    );
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    Arc<dyn Store>,
)> {
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
            return None;
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
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

#[tokio::test]
async fn a_sign_in_racing_a_scim_delete_mints_no_session_while_another_tenants_does_on_sqlite() {
    a_sign_in_racing_a_scim_removal_mints_no_session_while_another_tenants_does(
        sqlite().await,
        Removal::Delete,
    )
    .await;
}

#[tokio::test]
async fn a_sign_in_racing_a_scim_deactivation_mints_no_session_while_another_tenants_does_on_sqlite(
) {
    a_sign_in_racing_a_scim_removal_mints_no_session_while_another_tenants_does(
        sqlite().await,
        Removal::Deactivate,
    )
    .await;
}

#[tokio::test]
async fn a_sign_in_racing_a_scim_delete_mints_no_session_while_another_tenants_does_on_postgres() {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    a_sign_in_racing_a_scim_removal_mints_no_session_while_another_tenants_does(
        store,
        Removal::Delete,
    )
    .await;
}

#[tokio::test]
async fn a_sign_in_racing_a_scim_deactivation_mints_no_session_while_another_tenants_does_on_postgres(
) {
    let Some((_container, store)) = postgres().await else {
        return;
    };
    a_sign_in_racing_a_scim_removal_mints_no_session_while_another_tenants_does(
        store,
        Removal::Deactivate,
    )
    .await;
}
