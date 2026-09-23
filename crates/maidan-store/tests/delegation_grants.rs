//! Durable delegation-grant lifecycle and tenant isolation on both stores.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewDelegationGrant, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn member(
    store: &dyn Store,
    workspace_id: maidan_types::WorkspaceId,
    handle: &str,
) -> maidan_types::Member {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "delegation".into(),
        })
        .await
        .unwrap();
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let subject = member(store, ws.id, "subject").await;
    let delegate = member(store, ws.id, "delegate").await;
    let authorizer = member(store, ws.id, "authorizer").await;
    let outsider = member(store, other_ws.id, "outsider").await;
    let expiry = Utc::now() + ChronoDuration::hours(8);

    let grant = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec![
                "member:write".into(),
                " member:read ".into(),
                "member:write".into(),
            ],
            purpose: "  cover the incident shift  ".into(),
            authorized_by: authorizer.id,
            expires_at: expiry,
        })
        .await
        .expect("grant");
    assert_eq!(grant.capabilities, vec!["member:read", "member:write"]);
    assert_eq!(grant.purpose, "cover the incident shift");
    assert_eq!(store.get_delegation_grant(grant.id).await.unwrap(), grant);
    assert_eq!(
        store.list_delegation_grants(ws.id).await.unwrap(),
        vec![grant.clone()]
    );
    assert!(store
        .list_delegation_grants(other_ws.id)
        .await
        .unwrap()
        .is_empty());

    for invalid in [
        NewApiToken {
            workspace_id: ws.id,
            member_id: subject.id,
            app_installation_id: None,
            token_hash: "c".repeat(64),
            label: None,
            capabilities: vec!["token:admin".into()],
            expires_at: Some(Utc::now() + ChronoDuration::minutes(15)),
        },
        NewApiToken {
            workspace_id: ws.id,
            member_id: subject.id,
            app_installation_id: None,
            token_hash: "d".repeat(64),
            label: None,
            capabilities: vec!["member:read".into()],
            expires_at: Some(Utc::now() + ChronoDuration::hours(2)),
        },
    ] {
        assert!(store
            .create_delegated_api_token(invalid, grant.id, delegate.id, None)
            .await
            .is_err());
    }

    let direct_hash = "a".repeat(64);
    let direct = store
        .create_delegated_api_token(
            NewApiToken {
                workspace_id: ws.id,
                member_id: subject.id,
                app_installation_id: None,
                token_hash: direct_hash.clone(),
                label: Some("delegated".into()),
                capabilities: vec!["member:read".into()],
                expires_at: Some(Utc::now() + ChronoDuration::minutes(15)),
            },
            grant.id,
            delegate.id,
            None,
        )
        .await
        .expect("delegated token");
    let child_hash = "b".repeat(64);
    let child = store
        .create_attenuated_api_token(
            NewApiToken {
                workspace_id: ws.id,
                member_id: subject.id,
                app_installation_id: None,
                token_hash: child_hash.clone(),
                label: Some("delegated child".into()),
                capabilities: vec!["member:read".into()],
                expires_at: direct.expires_at,
            },
            direct.id,
        )
        .await
        .expect("attenuated child");
    // A narrowed child of a borrowed token inherits its parent's grant, so it
    // still resolves with the delegate as actor. Without this the child would
    // read, forever after, as the subject acting alone.
    assert_eq!(
        child.delegation_grant_id,
        Some(grant.id),
        "a child of a borrowed token must inherit the grant"
    );
    assert_eq!(
        store
            .get_active_api_token_by_hash(&direct_hash)
            .await
            .unwrap()
            .id,
        direct.id
    );
    assert_eq!(
        store
            .get_active_api_token_by_hash(&child_hash)
            .await
            .unwrap()
            .id,
        child.id
    );

    for invalid in [
        NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: subject.id,
            capabilities: vec!["member:write".into()],
            purpose: "self".into(),
            authorized_by: authorizer.id,
            expires_at: expiry,
        },
        NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec![],
            purpose: "empty caps".into(),
            authorized_by: authorizer.id,
            expires_at: expiry,
        },
        NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec!["member:write".into()],
            purpose: "   ".into(),
            authorized_by: authorizer.id,
            expires_at: expiry,
        },
        NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: outsider.id,
            capabilities: vec!["member:write".into()],
            purpose: "cross tenant".into(),
            authorized_by: authorizer.id,
            expires_at: expiry,
        },
        NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec!["member:write".into()],
            purpose: "expired".into(),
            authorized_by: authorizer.id,
            expires_at: Utc::now() - ChronoDuration::seconds(1),
        },
    ] {
        assert!(store.create_delegation_grant(invalid).await.is_err());
    }

    assert!(!store
        .revoke_delegation_grant(other_ws.id, grant.id)
        .await
        .unwrap());
    assert!(store
        .revoke_delegation_grant(ws.id, grant.id)
        .await
        .unwrap());
    assert!(!store
        .revoke_delegation_grant(ws.id, grant.id)
        .await
        .unwrap());
    assert!(store
        .get_delegation_grant(grant.id)
        .await
        .unwrap()
        .revoked_at
        .is_some());
    assert!(store
        .get_active_api_token_by_hash(&direct_hash)
        .await
        .is_err());
    assert!(store
        .get_active_api_token_by_hash(&child_hash)
        .await
        .is_err());
    assert!(store
        .get_api_token(direct.id)
        .await
        .unwrap()
        .revoked_at
        .is_some());
    assert!(store
        .get_api_token(child.id)
        .await
        .unwrap()
        .revoked_at
        .is_some());
}

#[tokio::test]
async fn delegation_grant_lifecycle_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn delegation_grant_lifecycle_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::new(pool)).await;
}
