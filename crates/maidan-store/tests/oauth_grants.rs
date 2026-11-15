//! OAuth grant store: the member-scoped list behind grant management,
//! both backends behaving identically. A grant's client always has a
//! registry row (a client identified by a metadata document gets its
//! row on first use at `/authorize`), and the list treats a URL-keyed
//! metadata client like any other. A grant is for one resource, the MCP
//! endpoint its tokens answer on.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const METADATA_CLIENT: &str = "https://client.example/oauth/metadata.json";
const STREAMABLE: &str = "https://maidan.example.com/mcp/streamable";
const COORDINATOR: &str = "https://maidan.example.com/mcp/coordinator";

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
            resource: STREAMABLE.into(),
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

/// A grant reads back the moment its lineage began, the one a refresh is
/// capped 90 days from, from every query the token endpoint and the grants
/// page use.
async fn assert_grant_records_when_its_lineage_began(store: &dyn Store) {
    let ws = seed_workspace(store, "lineage-start-ws").await;
    let member = seed_member(store, ws, "lineage-starter").await;
    seed_client(store, "lineage-client").await;
    // Postgres stamps the row with its own clock, which can drift from the
    // test's by a few seconds in a container.
    let before = Utc::now() - chrono::Duration::seconds(60);
    let created = grant(store, "lineage-client", member, ws).await;
    let after = Utc::now() + chrono::Duration::seconds(60);
    assert!(
        created.created_at > before && created.created_at < after,
        "the lineage starts when the grant is created: {}",
        created.created_at
    );
    let by_id = store.get_oauth_grant(created.id).await.unwrap().unwrap();
    let found = store
        .find_oauth_grant(
            "lineage-client",
            member,
            ws,
            &["workspace:read".to_string()],
            STREAMABLE,
        )
        .await
        .unwrap()
        .unwrap();
    let listed = store.list_oauth_grants(ws, member).await.unwrap();
    for read in [&by_id, &found, &listed[0]] {
        assert_eq!(read.id, created.id);
        // Postgres keeps microseconds, so compare at that precision.
        assert_eq!(
            read.created_at.timestamp_micros(),
            created.created_at.timestamp_micros()
        );
    }
}

/// The same client, member, workspace and scope hold one live grant per
/// resource, and a lookup finds the grant for the resource it names and no
/// other: a token minted under the coordinator grant must never be read as
/// the full catalog's.
async fn assert_a_grant_is_for_one_resource(store: &dyn Store) {
    let ws = seed_workspace(store, "resource-ws").await;
    let member = seed_member(store, ws, "resource-member").await;
    seed_client(store, "resource-client").await;
    let new = |resource: &str| NewOAuthGrant {
        client_id: "resource-client".into(),
        member_id: member,
        workspace_id: ws,
        scope: vec!["workspace:read".into()],
        lineage_id: uuid::Uuid::now_v7(),
        resource: resource.into(),
    };
    let full = store.create_oauth_grant(new(STREAMABLE)).await.unwrap();
    let coordinator = store
        .create_oauth_grant(new(COORDINATOR))
        .await
        .expect("a second resource is a second grant");
    assert_ne!(full.id, coordinator.id);
    for (resource, id) in [(STREAMABLE, full.id), (COORDINATOR, coordinator.id)] {
        let found = store
            .find_oauth_grant(
                "resource-client",
                member,
                ws,
                &["workspace:read".to_string()],
                resource,
            )
            .await
            .unwrap()
            .expect("the grant for this resource");
        assert_eq!(found.id, id, "{resource}");
        assert_eq!(found.resource, resource);
        let by_id = store.get_oauth_grant(id).await.unwrap().unwrap();
        assert_eq!(by_id.resource, resource);
    }
    assert!(
        store
            .find_oauth_grant(
                "resource-client",
                member,
                ws,
                &["workspace:read".to_string()],
                "https://maidan.example.com/mcp",
            )
            .await
            .unwrap()
            .is_none(),
        "no grant is found for a resource it does not name"
    );
    // One live grant per resource still: a second for the same one conflicts.
    let duplicate = store.create_oauth_grant(new(COORDINATOR)).await;
    assert!(
        matches!(duplicate, Err(StoreError::Conflict(_))),
        "{duplicate:?}"
    );
    let mut listed: Vec<String> = store
        .list_oauth_grants(ws, member)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.resource)
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        vec![COORDINATOR.to_string(), STREAMABLE.to_string()]
    );
}

/// Migration 0239 gives each existing grant the resource its refresh tokens
/// carry, and revokes a grant that has none: such a grant has no live token,
/// and every live grant must name a real endpoint.
#[tokio::test]
async fn migration_0239_takes_a_grants_resource_from_its_refresh_tokens_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool.clone());
    let ws = seed_workspace(&store, "migrated-ws").await;
    let member = seed_member(&store, ws, "migrated-member").await;
    seed_client(&store, "migrated-client").await;
    let new = |scope: &str| NewOAuthGrant {
        client_id: "migrated-client".into(),
        member_id: member,
        workspace_id: ws,
        scope: vec![scope.into()],
        lineage_id: uuid::Uuid::now_v7(),
        resource: STREAMABLE.into(),
    };
    let refreshed = store
        .create_oauth_grant(new("workspace:read"))
        .await
        .unwrap();
    let unused = store
        .create_oauth_grant(new("workspace:write"))
        .await
        .unwrap();
    // The refresh token names the coordinator, so a resource read back as
    // the coordinator came from it and not from a default.
    sqlx::query(
        "INSERT INTO oauth_refresh_tokens
            (token_hash, grant_id, lineage_id, client_id, resource, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("migrated-refresh-hash")
    .bind(refreshed.id.0)
    .bind(refreshed.lineage_id)
    .bind("migrated-client")
    .bind(COORDINATOR)
    .bind(Utc::now())
    .bind(Utc::now() + chrono::Duration::days(30))
    .execute(&pool)
    .await
    .unwrap();
    // Back to the schema before 0239, then run it.
    sqlx::raw_sql(
        "DROP INDEX idx_oauth_grants_live_unique;
         ALTER TABLE oauth_grants DROP COLUMN resource;
         CREATE UNIQUE INDEX idx_oauth_grants_live_unique
             ON oauth_grants (client_id, member_id, workspace_id, scope)
             WHERE revoked_at IS NULL;",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/0239_oauth_grant_resource.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let kept = store
        .get_oauth_grant(refreshed.id)
        .await
        .unwrap()
        .expect("a grant with a refresh token stays live");
    assert_eq!(kept.resource, COORDINATOR);
    assert!(
        store.get_oauth_grant(unused.id).await.unwrap().is_none(),
        "a grant with no refresh token is revoked"
    );
    let listed: Vec<_> = store
        .list_oauth_grants(ws, member)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(listed, vec![refreshed.id]);
}

#[tokio::test]
async fn a_grant_is_for_one_resource_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_a_grant_is_for_one_resource(&store).await;
}

#[tokio::test]
async fn a_grant_is_for_one_resource_postgres() {
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
    assert_a_grant_is_for_one_resource(&store).await;
}

#[tokio::test]
async fn a_grant_reads_back_when_its_lineage_began_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_grant_records_when_its_lineage_began(&store).await;
}

#[tokio::test]
async fn a_grant_reads_back_when_its_lineage_began_postgres() {
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
    assert_grant_records_when_its_lineage_began(&store).await;
}

#[tokio::test]
async fn grant_list_scopes_to_member_and_workspace_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
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
