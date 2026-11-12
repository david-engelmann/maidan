//! Durable delegation-grant lifecycle and tenant isolation on both stores.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewDelegationGrant, NewMember, NewWorkspace};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
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

/// A workspace with a live grant from `subject` to `delegate`, ready to mint on.
async fn live_grant(
    store: &dyn Store,
    name: &str,
) -> (
    maidan_types::DelegationGrant,
    maidan_types::Member,
    maidan_types::Member,
) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let subject = member(store, ws.id, "subject").await;
    let delegate = member(store, ws.id, "delegate").await;
    let authorizer = member(store, ws.id, "authorizer").await;
    let grant = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec!["member:read".into()],
            purpose: "mint after revoke".into(),
            authorized_by: authorizer.id,
            expires_at: Utc::now() + ChronoDuration::hours(8),
        })
        .await
        .expect("grant");
    (grant, subject, delegate)
}

fn borrowed(
    grant: &maidan_types::DelegationGrant,
    subject: &maidan_types::Member,
    hash: char,
) -> NewApiToken {
    NewApiToken {
        workspace_id: grant.workspace_id,
        member_id: subject.id,
        app_installation_id: None,
        token_hash: hash.to_string().repeat(64),
        label: None,
        capabilities: vec!["member:read".into()],
        expires_at: Some(Utc::now() + ChronoDuration::minutes(15)),
    }
}

/// A revoked grant mints nothing, and revoking one tenant's grant leaves the
/// other tenant's grant minting.
async fn run_mint_after_revoke(store: &dyn Store) {
    let (revoked, subject, delegate) = live_grant(store, "revoked").await;
    let (kept, kept_subject, kept_delegate) = live_grant(store, "kept").await;
    assert!(store
        .revoke_delegation_grant(revoked.workspace_id, revoked.id)
        .await
        .unwrap());
    assert!(matches!(
        store
            .create_delegated_api_token(
                borrowed(&revoked, &subject, 'e'),
                revoked.id,
                delegate.id,
                None
            )
            .await,
        Err(StoreError::NotFound)
    ));
    assert!(store
        .get_active_api_token_by_hash(&"e".repeat(64))
        .await
        .is_err());
    store
        .create_delegated_api_token(
            borrowed(&kept, &kept_subject, 'f'),
            kept.id,
            kept_delegate.id,
            None,
        )
        .await
        .expect("the other workspace's grant still mints");
}

#[tokio::test]
async fn delegation_grant_lifecycle_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_revoked_grant_mints_nothing_sqlite() {
    run_mint_after_revoke(&sqlite().await).await;
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    sqlx::PgPool,
)> {
    use maidan_store::run_postgres_migrations;
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
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
    Some((container, pool))
}

#[tokio::test]
async fn delegation_grant_lifecycle_postgres() {
    let Some((_container, pool)) = postgres().await else {
        return;
    };
    run_suite(&PostgresStore::for_tests(pool)).await;
}

#[tokio::test]
async fn a_revoked_grant_mints_nothing_postgres() {
    let Some((_container, pool)) = postgres().await else {
        return;
    };
    run_mint_after_revoke(&PostgresStore::for_tests(pool)).await;
}

/// SEC-006: a mint that runs while a revoke of its grant is uncommitted waits
/// for the revoke, then refuses. Without the grant row lock the mint read the
/// old committed row, inserted a live token, and the revoke's cascade (already
/// past) never saw it.
#[tokio::test]
async fn a_mint_racing_an_uncommitted_revoke_waits_and_refuses_postgres() {
    use std::time::Duration;
    let Some((_container, pool)) = postgres().await else {
        return;
    };
    let store: std::sync::Arc<dyn Store> =
        std::sync::Arc::new(PostgresStore::for_tests(pool.clone()));
    let (grant, subject, delegate) = live_grant(store.as_ref(), "race").await;
    let (other, other_subject, other_delegate) = live_grant(store.as_ref(), "bystander").await;

    let mut revoke = pool.begin().await.unwrap();
    sqlx::query("UPDATE maidan_delegation_grants SET revoked_at = now() WHERE id = $1")
        .bind(grant.id.0)
        .execute(&mut *revoke)
        .await
        .unwrap();

    let mint = tokio::spawn({
        let store = store.clone();
        let new = borrowed(&grant, &subject, 'g');
        async move {
            store
                .create_delegated_api_token(new, grant.id, delegate.id, None)
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !mint.is_finished(),
        "the mint must wait on the grant row an uncommitted revoke holds"
    );
    // Another tenant's grant is not held by this revoke.
    store
        .create_delegated_api_token(
            borrowed(&other, &other_subject, 'h'),
            other.id,
            other_delegate.id,
            None,
        )
        .await
        .expect("an unrelated grant mints while the revoke is open");
    revoke.commit().await.unwrap();

    let minted = tokio::time::timeout(Duration::from_secs(15), mint)
        .await
        .expect("the mint finishes once the revoke commits")
        .unwrap();
    assert!(matches!(minted, Err(StoreError::NotFound)), "{minted:?}");
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM maidan_api_tokens WHERE delegation_grant_id = $1 AND revoked_at IS NULL",
    )
    .bind(grant.id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 0, "no live token row may sit on a revoked grant");
}
