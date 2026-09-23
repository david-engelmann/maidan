//! Durable delegation-grant lifecycle and tenant isolation on both stores.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewDelegationGrant, NewMember, NewWorkspace};
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
