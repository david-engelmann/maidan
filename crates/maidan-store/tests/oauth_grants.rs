//! OAuth grant store: the member-scoped list behind grant management,
//! both backends behaving identically. A grant's client always has a
//! registry row (a client identified by a metadata document gets its
//! row on first use at `/authorize`), and the list treats a URL-keyed
//! metadata client like any other.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const METADATA_CLIENT: &str = "https://client.example/oauth/metadata.json";

async fn seed_member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id
}

async fn seed_workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn seed_client(store: &dyn Store, client_id: &str) {
    store
        .create_oauth_client(NewOAuthClient {
            client_id: client_id.into(),
            name: client_id.into(),
            redirect_uris: vec!["https://client.example/callback".into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();
}

async fn grant(
    store: &dyn Store,
    client_id: &str,
    member_id: MemberId,
    workspace_id: WorkspaceId,
) -> OAuthGrant {
    store
        .create_oauth_grant(NewOAuthGrant {
            client_id: client_id.into(),
            member_id,
            workspace_id,
            scope: vec!["workspace:read".into()],
            lineage_id: uuid::Uuid::new_v4(),
        })
        .await
        .unwrap()
}

/// Shared assertions exercised against both backends.
async fn assert_grant_list_scopes(store: &dyn Store) {
    let ws = seed_workspace(store, "grants-ws").await;
    let other_ws = seed_workspace(store, "grants-other-ws").await;
    let member = seed_member(store, ws, "grantee").await;
    let bystander = seed_member(store, ws, "bystander").await;
    for client_id in [
        "registered-client",
        "another-client",
        "revoked-client",
        METADATA_CLIENT,
    ] {
        seed_client(store, client_id).await;
    }

    let first = grant(store, "registered-client", member, ws).await;
    let second = grant(store, "another-client", member, ws).await;
    // The same member's grant in a second workspace, and a grant of the
    // member's that has already been revoked: neither belongs in the list.
    grant(store, "registered-client", member, other_ws).await;
    let revoked = grant(store, "revoked-client", member, ws).await;
    store.revoke_oauth_grant(revoked.id).await.unwrap();

    let listed = store.list_oauth_grants(ws, member).await.unwrap();
    let ids: Vec<_> = listed.iter().map(|g| g.id).collect();
    assert_eq!(
        ids,
        vec![first.id, second.id],
        "the list is the member's live grants in this workspace, oldest first"
    );
    assert!(
        store
            .list_oauth_grants(ws, bystander)
            .await
            .unwrap()
            .is_empty(),
        "another member's list in the same workspace must be empty"
    );
    assert_eq!(
        store
            .list_oauth_grants(other_ws, member)
            .await
            .unwrap()
            .len(),
        1,
        "the member's grant in the other workspace lists there, not here"
    );

    // A client identified by a metadata document holds its grant under
    // its URL, with its registry row created on first use; a code and a
    // grant naming it record and list like any registered client's.
    store
        .create_oauth_authorization_code(NewOAuthAuthorizationCode {
            code_hash: "metadata-code-hash".into(),
            client_id: METADATA_CLIENT.into(),
            member_id: member,
            workspace_id: ws,
            redirect_uri: "https://client.example/callback".into(),
            code_challenge: "challenge".into(),
            scope: vec!["workspace:read".into()],
            resource: None,
            expires_at: Utc::now() + chrono::Duration::minutes(10),
            lineage_id: None,
        })
        .await
        .unwrap();
    let metadata_grant = grant(store, METADATA_CLIENT, member, ws).await;
    assert!(
        store
            .list_oauth_grants(ws, member)
            .await
            .unwrap()
            .iter()
            .any(|g| g.id == metadata_grant.id),
        "a metadata client's grant must list like any other"
    );
}

#[tokio::test]
async fn grant_list_scopes_to_member_and_workspace_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_grant_list_scopes(&store).await;
}

#[tokio::test]
async fn grant_list_scopes_to_member_and_workspace_postgres() {
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
    assert_grant_list_scopes(&store).await;
}
