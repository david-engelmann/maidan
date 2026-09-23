//! A workspace bounds how long a delegation grant may live (D-B), on both
//! backends.
//!
//! A delegated token lasts at most an hour, but the grant behind it is the
//! standing authority to keep minting them. Without a ceiling a grant could be
//! issued to expire in a century, and only remembering to revoke it would end
//! it.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewDelegationGrant, NewMember, NewWorkspace, DEFAULT_MAX_GRANT_DAYS,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["subject", "delegate", "admin"] {
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
    let grant = |days: i64| NewDelegationGrant {
        workspace_id: ws.id,
        subject_id: ids[0],
        delegate_id: ids[1],
        capabilities: vec!["workspace:read".into()],
        purpose: "p".into(),
        authorized_by: ids[2],
        expires_at: Utc::now() + Duration::days(days),
    };

    // No policy set: the default applies, and is reported as the default.
    let policy = store.get_delegation_policy(ws.id).await.unwrap();
    assert_eq!(policy.max_grant_days, DEFAULT_MAX_GRANT_DAYS);
    assert!(policy.is_default);
    store
        .create_delegation_grant(grant(DEFAULT_MAX_GRANT_DAYS - 1))
        .await
        .unwrap();
    let err = store
        .create_delegation_grant(grant(DEFAULT_MAX_GRANT_DAYS + 1))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidInput(ref m) if m.contains("ceiling")),
        "{err:?}"
    );

    // A workspace tightens it.
    let policy = store.set_delegation_policy(ws.id, Some(7)).await.unwrap();
    assert_eq!(policy.max_grant_days, 7);
    assert!(!policy.is_default);
    assert!(store.create_delegation_grant(grant(8)).await.is_err());
    store.create_delegation_grant(grant(6)).await.unwrap();

    // Out-of-range ceilings are refused.
    for days in [0, -1, 3651] {
        assert!(
            store
                .set_delegation_policy(ws.id, Some(days))
                .await
                .is_err(),
            "{days} days must be refused"
        );
    }

    // Clearing restores the default.
    let policy = store.set_delegation_policy(ws.id, None).await.unwrap();
    assert_eq!(policy.max_grant_days, DEFAULT_MAX_GRANT_DAYS);
    assert!(policy.is_default);
}

#[tokio::test]
async fn a_grant_cannot_outlive_its_workspace_ceiling_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::new(pool)).await;
}

#[tokio::test]
async fn a_grant_cannot_outlive_its_workspace_ceiling_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::new(pool)).await;
}
